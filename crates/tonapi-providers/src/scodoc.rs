//! Fournisseur `scodoc-gateway` : notes et absences par la passerelle ScoDoc des IUT
//! (`Scodoc_Notes`, `services/data.php`).
//!
//! La passerelle renvoie tout en un seul appel, `dataPremièreConnexion` : relevé du semestre
//! courant (bulletin ScoDoc au format BUT), détail des absences par jour et totaux. Les notes et les
//! absences se lisent donc dans la même réponse, avec deux analyses pures.
//!
//! Connexion, selon `sign-in` :
//! - `university-cas` : le TGC de la session ouvre `services/doAuth.php` ([`Net::open_protected`]),
//!   sans mot de passe. Les cookies (dont `PHPSESSID`) repartent dans la session de l'étudiant.
//! - `own-cas` : CAS propre à la composante, avec d'autres identifiants. [`sign_in`] ouvre la session
//!   et ne garde que les cookies dans `tokens["scodoc.<hôte>"]` ; jamais le mot de passe. Les
//!   requêtes suivantes n'utilisent que ce jeton, jamais les cookies de l'université.
//!
//! Source des formats : `Docs/Montpellier/scodoc.md` et `Docs/Rennes/scodoc.md`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt::Display;
use std::time::Duration;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, TimeZone};
use chrono_tz::Tz;
use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde_json::Value;
use tonapi_config::{ScoDocGateway, SignIn};
use tonapi_core::{
    Absence, AbsenceReport, AbsenceTotals, ErrorCode, Evaluation, Grade, GradeReport, ModuleGrades,
    Problem, SemesterGrades, UnitGrade,
};
use tonapi_http::cas::{self, CasError, CasServer, Credentials, Outcome};
use tonapi_http::{CookieJar, Page};
use tonapi_session::StoredCookie;
use url::Url;

use crate::Context;
use crate::net::{Net, http_problem, upstream};

const PROVIDER: &str = "scodoc-gateway";
/// Cookie de session PHP de la passerelle.
const SESSION_COOKIE: &str = "PHPSESSID";
/// Délai d'une requête quand `options.timeout-secs` est absent.
const DEFAULT_TIMEOUT_SECS: u32 = 20;
/// Les notes et moyennes du bulletin ScoDoc sont ramenées sur 20 [déduit]. `note.max` du bulletin
/// est la meilleure note de la promotion, pas le barème.
const OUT_OF: f64 = 20.0;
/// Mention donnée à `"~"` (pas encore de note) [déduit].
const PENDING: &str = "ATT";

fn bad(what: impl Display) -> Problem {
    upstream(PROVIDER, what)
}

// ─────────────────────────────── Analyse pure ───────────────────────────────

/// Réponse de `data.php`, une fois lue.
#[derive(Debug)]
pub(crate) enum Reply {
    Data(Value),
    /// `{"redirect":"/services/doAuth.php"}` en HTTP 200 : la session PHP est perdue.
    SessionLost,
}

/// Distingue les données, la session perdue et l'erreur applicative de la passerelle.
pub(crate) fn classify_reply(value: Value) -> Result<Reply, Problem> {
    let Value::Object(map) = &value else {
        return Err(bad("réponse inattendue (objet JSON attendu)"));
    };
    if map.contains_key("redirect") {
        return Ok(Reply::SessionLost);
    }
    if map.contains_key("erreur") {
        // Le message est écrit par l'IUT et peut citer l'étudiant : on ne le recopie pas.
        return Err(Problem::new(ErrorCode::ServiceUnavailable).detail(format!(
            "{PROVIDER} : la passerelle a répondu par une erreur"
        )));
    }
    Ok(Reply::Data(value))
}

/// Contenu d'une cellule de note.
#[derive(Debug, PartialEq)]
enum Cell {
    Value(f64),
    /// Mention à la place d'une note (`ABS`, `EXC`, `ATT`…).
    Status(String),
    Empty,
}

/// `"12.50"` ou `"12,50"`. `NaN` et `inf` ne sont pas des notes.
fn decimal(text: &str) -> Option<f64> {
    let value: f64 = text.trim().replace(',', ".").parse().ok()?;
    value.is_finite().then_some(value)
}

/// Nombre envoyé comme nombre ou comme chaîne.
fn lossy_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64().filter(|v| v.is_finite()),
        Value::String(s) => decimal(s),
        _ => None,
    }
}

/// `"12.50"` → note ; `"~"` → en attente ; `"-"` et `""` → vide ; `"ABS"`, `"EXC"`… → mention.
/// Une valeur qui n'est pas un nombre ne devient jamais zéro.
fn cell(value: Option<&Value>) -> Cell {
    match value {
        Some(Value::String(s)) => {
            let text = s.trim();
            if text.is_empty() || text == "-" {
                Cell::Empty
            } else if text == "~" {
                Cell::Status(PENDING.to_owned())
            } else if let Some(v) = decimal(text) {
                Cell::Value(v)
            } else {
                Cell::Status(text.to_uppercase())
            }
        }
        Some(number @ Value::Number(_)) => lossy_f64(number).map_or(Cell::Empty, Cell::Value),
        _ => Cell::Empty,
    }
}

/// `{"value": …, "min": …}` (bulletin) ou la valeur elle-même (moyenne d'UE d'une ressource).
fn scalar(value: Option<&Value>) -> Option<&Value> {
    match value {
        Some(object @ Value::Object(_)) => object.get("value"),
        other => other,
    }
}

fn average(value: Option<&Value>) -> Option<Grade> {
    match cell(scalar(value)) {
        Cell::Value(value) => Some(Grade {
            value,
            out_of: OUT_OF,
        }),
        Cell::Status(_) | Cell::Empty => None,
    }
}

/// Identifiant envoyé comme nombre ou comme chaîne.
fn id_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    let s = value?.as_str()?.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

/// Entrées d'un objet. PHP envoie `[]` pour un objet vide.
fn entries<'a>(
    value: Option<&'a Value>,
    what: &str,
) -> Result<Vec<(&'a String, &'a Value)>, Problem> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) if a.is_empty() => Ok(Vec::new()),
        Some(Value::Object(map)) => Ok(map.iter().collect()),
        Some(_) => Err(bad(format!("`{what}` : objet attendu"))),
    }
}

/// Éléments d'un tableau. PHP envoie parfois `{}` pour un tableau vide.
fn items<'a>(value: Option<&'a Value>, what: &str) -> Result<&'a [Value], Problem> {
    match value {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(a)) => Ok(a),
        Some(Value::Object(m)) if m.is_empty() => Ok(&[]),
        Some(_) => Err(bad(format!("`{what}` : tableau attendu"))),
    }
}

/// Tri « naturel » : `R1.2` avant `R1.10`.
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let run = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(d) = chars.next_if(char::is_ascii_digit) {
                        digits.push(d);
                    }
                    digits.trim_start_matches('0').to_owned()
                };
                let (left, right) = (run(&mut a), run(&mut b));
                let order = left.len().cmp(&right.len()).then_with(|| left.cmp(&right));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

fn sorted<'a>(mut list: Vec<(&'a String, &'a Value)>) -> Vec<(&'a String, &'a Value)> {
    list.sort_by(|(a, _), (b, _)| natural_cmp(a, b));
    list
}

/// Date d'une évaluation dans le fuseau de l'université. Un instant avec décalage est converti ;
/// une date sans fuseau est prise telle quelle.
fn evaluation_date(value: Option<&Value>, tz: Tz) -> Option<NaiveDate> {
    let text = value?.as_str()?.trim();
    if let Ok(instant) = DateTime::parse_from_rfc3339(text) {
        return Some(instant.with_timezone(&tz).date_naive());
    }
    NaiveDate::parse_from_str(text.get(..10)?, "%Y-%m-%d").ok()
}

fn parse_evaluation(value: &Value, tz: Tz) -> Result<Evaluation, Problem> {
    if !value.is_object() {
        return Err(bad("évaluation : objet attendu"));
    }
    let id = id_string(value.get("id")).ok_or_else(|| bad("évaluation sans identifiant"))?;
    let (grade, status) = match cell(scalar(value.get("note"))) {
        Cell::Value(value) => (
            Some(Grade {
                value,
                out_of: OUT_OF,
            }),
            None,
        ),
        Cell::Status(status) => (None, Some(status)),
        Cell::Empty => (None, None),
    };
    Ok(Evaluation {
        name: text(value.get("description")).unwrap_or_else(|| format!("Évaluation {id}")),
        date: evaluation_date(value.get("date"), tz),
        coefficient: value.get("coef").and_then(lossy_f64),
        id,
        grade,
        status,
    })
}

fn parse_module(code: &str, value: &Value, tz: Tz) -> Result<ModuleGrades, Problem> {
    if !value.is_object() {
        return Err(bad("ressource ou SAÉ : objet attendu"));
    }
    let evaluations = items(value.get("evaluations"), "evaluations")?
        .iter()
        .map(|e| parse_evaluation(e, tz))
        .collect::<Result<_, _>>()?;
    Ok(ModuleGrades {
        code: code.to_owned(),
        name: text(value.get("titre")).unwrap_or_else(|| code.to_owned()),
        average: average(value.get("moyenne")),
        evaluations,
    })
}

fn parse_unit(code: &str, value: &Value) -> Result<UnitGrade, Problem> {
    if !value.is_object() {
        return Err(bad("UE : objet attendu"));
    }
    Ok(UnitGrade {
        code: code.to_owned(),
        name: text(value.get("titre")).unwrap_or_else(|| code.to_owned()),
        average: average(value.get("moyenne")),
        ects: value
            .get("ECTS")
            .and_then(|e| e.get("total"))
            .and_then(lossy_f64),
    })
}

/// Rang du semestre (`12/56`). `-` ou vide quand l'IUT masque les rangs.
fn rank(value: Option<&Value>) -> Option<String> {
    let value = value?;
    let position = match value.get("value")? {
        Value::String(s) if !s.trim().is_empty() && s.trim() != "-" => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    match value.get("total").and_then(lossy_f64) {
        Some(total) if total >= 1.0 => Some(format!("{position}/{total:.0}")),
        _ => Some(position),
    }
}

/// Relevé du semestre courant. Le bulletin n'est lu que dans la clé `relevé` (avec accent) ;
/// les données personnelles de `relevé.etudiant` ne sont jamais copiées.
pub(crate) fn parse_grades(data: &Value, tz: Tz) -> Result<GradeReport, Problem> {
    let Some(report @ Value::Object(_)) = data.get("relevé") else {
        return Err(bad("clé `relevé` absente ou illisible"));
    };
    let listed = items(data.get("semestres"), "semestres")?;
    // Le semestre courant est le dernier de la liste triée par la passerelle [lu].
    let current = id_string(report.get("formsemestre_id")).or_else(|| {
        listed
            .last()
            .and_then(|s| id_string(s.get("formsemestre_id")))
    });
    let Some(id) = current else {
        // Aucun semestre : la passerelle renvoie un relevé vide.
        return Ok(GradeReport {
            semesters: Vec::new(),
        });
    };

    let semester = report.get("semestre");
    let name = listed
        .iter()
        .find(|s| id_string(s.get("formsemestre_id")).as_deref() == Some(id.as_str()))
        .and_then(|s| text(s.get("titre")))
        .or_else(|| {
            let number = semester.and_then(|s| s.get("numero")).and_then(lossy_f64)?;
            Some(format!("Semestre {number:.0}"))
        })
        .unwrap_or_else(|| format!("Semestre {id}"));

    let units = sorted(entries(report.get("ues"), "ues")?)
        .into_iter()
        .map(|(code, value)| parse_unit(code, value))
        .collect::<Result<_, _>>()?;
    let mut modules = Vec::new();
    for key in ["ressources", "saes"] {
        for (code, value) in sorted(entries(report.get(key), key)?) {
            modules.push(parse_module(code, value, tz)?);
        }
    }
    Ok(GradeReport {
        semesters: vec![SemesterGrades {
            id,
            name,
            average: average(semester.and_then(|s| s.get("notes"))),
            rank: rank(semester.and_then(|s| s.get("rang"))),
            units,
            modules,
        }],
    })
}

/// Instant local à partir d'un jour et d'une heure décimale (`15.5` = 15 h 30). Une heure qui
/// n'existe pas (saut du changement d'heure) avance d'une heure ; une heure qui existe deux fois
/// prend la première.
fn at(day: NaiveDate, hours: f64, tz: Tz) -> Result<DateTime<FixedOffset>, Problem> {
    if !(0.0..=24.0).contains(&hours) {
        return Err(bad("horaire d'absence hors d'une journée"));
    }
    #[allow(clippy::cast_possible_truncation)] // 0 ≤ hours ≤ 24 : au plus 1 440 minutes
    let minutes = (hours * 60.0).round() as i64;
    let local = day.and_time(NaiveTime::MIN) + chrono::Duration::minutes(minutes);
    tz.from_local_datetime(&local)
        .earliest()
        .or_else(|| {
            tz.from_local_datetime(&(local + chrono::Duration::hours(1)))
                .earliest()
        })
        .map(|instant| instant.fixed_offset())
        .ok_or_else(|| bad("horaire d'absence illisible"))
}

fn flag(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|v| v != 0.0),
        Some(Value::String(s)) => matches!(s.trim().to_lowercase().as_str(), "true" | "1" | "oui"),
        _ => false,
    }
}

/// Titre des ressources et SAÉ par identifiant ScoDoc (`matiereComplet` est un `moduleimpl_id`).
fn module_titles(data: &Value) -> HashMap<String, String> {
    let mut titles = HashMap::new();
    let Some(report) = data.get("relevé") else {
        return titles;
    };
    for key in ["ressources", "saes"] {
        for (_, value) in entries(report.get(key), key).unwrap_or_default() {
            if let (Some(id), Some(title)) = (id_string(value.get("id")), text(value.get("titre")))
            {
                titles.insert(id, title);
            }
        }
    }
    titles
}

/// Absences du semestre courant. Seules les entrées `absent` comptent : les retards et les présences
/// du même détail sont ignorés. Les totaux sont recomptés sur ces entrées, car `totauxAbsences` ne
/// contient que la métrique choisie par l'IUT.
pub(crate) fn parse_absences(data: &Value, tz: Tz) -> Result<AbsenceReport, Problem> {
    if !matches!(
        data.get("absences"),
        Some(Value::Object(_) | Value::Array(_))
    ) {
        return Err(bad("clé `absences` absente ou illisible"));
    }
    let titles = module_titles(data);
    let mut absences = Vec::new();
    for (day_key, list) in entries(data.get("absences"), "absences")? {
        let day = NaiveDate::parse_from_str(day_key, "%Y-%m-%d")
            .map_err(|_| bad("absences : jour illisible"))?;
        for entry in items(Some(list), "absences[jour]")? {
            let status = entry
                .get("statut")
                .and_then(Value::as_str)
                .ok_or_else(|| bad("absence sans statut"))?;
            if !status.trim().eq_ignore_ascii_case("absent") {
                continue;
            }
            let from = entry
                .get("debut")
                .and_then(lossy_f64)
                .ok_or_else(|| bad("absence sans heure de début"))?;
            let to = entry
                .get("fin")
                .and_then(lossy_f64)
                .ok_or_else(|| bad("absence sans heure de fin"))?;
            let last_day = text(entry.get("dateFin"))
                .and_then(|d| NaiveDate::parse_from_str(d.get(..10)?, "%Y-%m-%d").ok())
                .unwrap_or(day);
            absences.push(Absence {
                // Sans `data_absences_scodoc`, la passerelle n'envoie pas d'identifiant.
                id: id_string(entry.get("idAbs"))
                    .unwrap_or_else(|| format!("{day_key}-{from}-{to}")),
                start: at(day, from, tz)?,
                end: at(last_day, to, tz)?,
                justified: flag(entry.get("justifie")),
                module: id_string(entry.get("matiereComplet"))
                    .and_then(|id| titles.get(&id).cloned()),
            });
        }
    }
    absences.sort_by_key(|a| a.start);
    let count = |justified: bool| {
        let n = absences.iter().filter(|a| a.justified == justified).count();
        u32::try_from(n).unwrap_or(u32::MAX)
    };
    let totals = AbsenceTotals {
        justified: count(true),
        unjustified: count(false),
    };
    Ok(AbsenceReport { absences, totals })
}

// ──────────────────────────────── Réseau ────────────────────────────────────

/// Racine de la passerelle, avec une barre finale.
fn base_of(config: &ScoDocGateway) -> Url {
    let mut base = config.base_url.clone();
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base.set_query(None);
    base.set_fragment(None);
    base
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.host_str() == b.host_str() && a.port_or_known_default() == b.port_or_known_default()
}

/// `services/doAuth.php?href=<racine>` : ouvre la session PHP puis passe par le CAS.
fn auth_url(base: &Url) -> Url {
    let mut url = base
        .join("services/doAuth.php")
        .unwrap_or_else(|_| base.clone());
    url.query_pairs_mut().append_pair("href", base.as_str());
    url
}

/// `services/data.php?q=dataPremièreConnexion` (l'accent est encodé en UTF-8).
fn data_url(base: &Url) -> Url {
    let mut url = base
        .join("services/data.php")
        .unwrap_or_else(|_| base.clone());
    url.query_pairs_mut()
        .append_pair("q", "dataPremièreConnexion");
    url
}

/// Clé de `SessionData::tokens` du service : une connexion sert aux notes et aux absences.
fn token_key(base: &Url) -> String {
    let host = base.host_str().unwrap_or_default();
    match base.port() {
        Some(port) => format!("scodoc.{host}:{port}"),
        None => format!("scodoc.{host}"),
    }
}

fn login_required(detail: &str) -> Problem {
    Problem::new(ErrorCode::ServiceLoginRequired).detail(format!(
        "{PROVIDER} : {detail} ; une connexion au service est requise"
    ))
}

fn cas_problem(error: CasError) -> Problem {
    match error {
        CasError::InvalidCredentials(message) => {
            let problem = Problem::new(ErrorCode::InvalidCredentials);
            match message {
                Some(m) => problem.detail(m),
                None => problem,
            }
        }
        CasError::NotAuthenticated => Problem::new(ErrorCode::SessionExpired),
        CasError::Unsupported("password_change") => Problem::new(ErrorCode::AccountActionRequired)
            .detail("mot de passe à changer sur le site de l'établissement"),
        CasError::Unsupported(step) => Problem::new(ErrorCode::NotImplemented).detail(format!(
            "{PROVIDER} : étape de connexion `{step}` pas encore prise en charge pour ce CAS"
        )),
        CasError::Http(e) => http_problem(PROVIDER, &e),
        CasError::InsecureForm | CasError::Unexpected(_) => bad(error),
    }
}

/// Fuseau de l'université.
fn timezone(ctx: &Context<'_>) -> Result<Tz, Problem> {
    ctx.university.university.timezone.parse().map_err(|_| {
        Problem::new(ErrorCode::Internal).detail("fuseau horaire de l'université illisible")
    })
}

fn stored_cookies(ctx: &Context<'_>, key: &str) -> Option<Vec<StoredCookie>> {
    serde_json::from_str(ctx.session.tokens.get(key)?).ok()
}

/// Les cookies du service, et rien d'autre, dans le jeton de la session.
fn store_cookies(ctx: &mut Context<'_>, key: &str, jar: &CookieJar) {
    if let Ok(json) = serde_json::to_string(&jar.to_stored(ctx.now.timestamp())) {
        ctx.session.tokens.insert(key.to_owned(), json);
    }
}

/// Ouvre la session de la passerelle. Pour un CAS de composante sans TGC valable, c'est à
/// l'étudiant de se reconnecter avec ses identifiants du service.
async fn open_session(net: &mut Net, config: &ScoDocGateway, base: &Url) -> Result<(), Problem> {
    let entry = match (&config.sign_in, net.cas.as_ref()) {
        (
            SignIn::UniversityCas {
                service: Some(service),
            },
            Some(cas),
        ) => cas.login_url(Some(service)),
        _ => auth_url(base),
    };
    let page = net.open_protected(entry).await.map_err(|problem| {
        if matches!(config.sign_in, SignIn::OwnCas { .. })
            && problem.code == ErrorCode::SessionExpired
        {
            login_required("la session du CAS de la composante a expiré")
        } else {
            problem
        }
    })?;
    landed(&page, base)
}

/// La chaîne de redirections doit s'arrêter sur la passerelle, pas sur un formulaire du CAS.
fn landed(page: &Page, base: &Url) -> Result<(), Problem> {
    if same_origin(&page.url, base) {
        Ok(())
    } else {
        Err(bad("la connexion n'a pas abouti sur la passerelle"))
    }
}

/// Un appel de `dataPremièreConnexion`.
async fn call(net: &mut Net, base: &Url, timeout: Duration) -> Result<Reply, Problem> {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    if let Ok(referer) = HeaderValue::from_str(base.as_str()) {
        headers.insert(header::REFERER, referer);
    }
    let (status, _, body) = net
        .browser
        .send(
            Method::POST,
            data_url(base),
            headers,
            Some(String::new()),
            Some(timeout),
        )
        .await
        .map_err(|e| http_problem(PROVIDER, &e))?;
    if status.is_server_error() {
        return Err(Problem::new(ErrorCode::ServiceUnavailable)
            .detail(format!("{PROVIDER} : HTTP {status}")));
    }
    let value = serde_json::from_str(&body)
        .map_err(|_| bad(format!("réponse non JSON (HTTP {status})")))?;
    classify_reply(value)
}

/// Lit `dataPremièreConnexion`. Avec un `PHPSESSID` déjà posé, on essaie d'abord sans se
/// reconnecter ; si la passerelle répond `{"redirect"}`, la session est rouverte et l'appel est
/// refait **une seule fois**.
async fn fetch_value(
    net: &mut Net,
    config: &ScoDocGateway,
    base: &Url,
    timeout: Duration,
) -> Result<Value, Problem> {
    let host = base.host_str().unwrap_or_default();
    if net.browser.jar.contains(host, SESSION_COOKIE)
        && let Reply::Data(value) = call(net, base, timeout).await?
    {
        return Ok(value);
    }
    open_session(net, config, base).await?;
    match call(net, base, timeout).await? {
        Reply::Data(value) => Ok(value),
        Reply::SessionLost if matches!(config.sign_in, SignIn::OwnCas { .. }) => {
            Err(login_required("la passerelle refuse la session"))
        }
        Reply::SessionLost => Err(Problem::new(ErrorCode::SessionExpired).detail(format!(
            "{PROVIDER} : la passerelle refuse la session après une nouvelle connexion"
        ))),
    }
}

async fn fetch_data(ctx: &mut Context<'_>, config: &ScoDocGateway) -> Result<Value, Problem> {
    let base = base_of(config);
    let timeout = Duration::from_secs(u64::from(
        config.options.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS),
    ));
    let mut net = Net::open(ctx, PROVIDER)?;
    match &config.sign_in {
        SignIn::UniversityCas { .. } => {
            let result = fetch_value(&mut net, config, &base, timeout).await;
            net.save(ctx);
            result
        }
        SignIn::OwnCas {
            base_url, flavor, ..
        } => {
            let key = token_key(&base);
            let Some(cookies) = stored_cookies(ctx, &key) else {
                return Err(login_required("aucune session ouverte sur ce service"));
            };
            // Pot de cookies du service seul : ceux de l'université n'ont rien à faire ici, et
            // `net.save` ne doit pas les écraser.
            net.browser.jar = CookieJar::from_stored(cookies);
            net.cas = Some(CasServer::new(base_url.clone(), *flavor));
            let result = fetch_value(&mut net, config, &base, timeout).await;
            store_cookies(ctx, &key, &net.browser.jar);
            result
        }
        SignIn::LocalPassword { .. } => Err(Problem::new(ErrorCode::NotImplemented).detail(
            format!("{PROVIDER} : la connexion `local-password` n'existe pas pour ScoDoc"),
        )),
    }
}

/// Notes du semestre courant.
pub(crate) async fn fetch_grades(
    ctx: &mut Context<'_>,
    config: &ScoDocGateway,
) -> Result<GradeReport, Problem> {
    let tz = timezone(ctx)?;
    parse_grades(&fetch_data(ctx, config).await?, tz)
}

/// Absences du semestre courant.
pub(crate) async fn fetch_absences(
    ctx: &mut Context<'_>,
    config: &ScoDocGateway,
) -> Result<AbsenceReport, Problem> {
    let tz = timezone(ctx)?;
    parse_absences(&fetch_data(ctx, config).await?, tz)
}

/// Notes et absences d'une seule lecture de la passerelle : `data.php` les donne ensemble, et une
/// session PHP traite ses requêtes l'une après l'autre.
pub(crate) async fn fetch_both(
    ctx: &mut Context<'_>,
    config: &ScoDocGateway,
) -> (Result<GradeReport, Problem>, Result<AbsenceReport, Problem>) {
    let data = match timezone(ctx) {
        Ok(tz) => fetch_data(ctx, config).await.map(|data| (data, tz)),
        Err(problem) => Err(problem),
    };
    match data {
        Ok((data, tz)) => (parse_grades(&data, tz), parse_absences(&data, tz)),
        Err(problem) => (Err(problem.clone()), Err(problem)),
    }
}

/// Connexion à une passerelle derrière un CAS propre à la composante (`sign-in = own-cas`).
///
/// Le mot de passe sert une fois, vers le CAS de la composante seulement. Seuls les cookies de la
/// chaîne (CAS de la composante et passerelle) restent, dans `ctx.session.tokens["scodoc.<hôte>"]` ;
/// les cookies de l'université ne sont ni lus ni modifiés.
pub(crate) async fn sign_in(
    ctx: &mut Context<'_>,
    config: &ScoDocGateway,
    username: &str,
    password: &str,
) -> Result<(), Problem> {
    let SignIn::OwnCas {
        base_url, flavor, ..
    } = &config.sign_in
    else {
        return Err(Problem::new(ErrorCode::InvalidRequest).detail(format!(
            "le fournisseur `{PROVIDER}` n'a pas de connexion séparée pour ce service"
        )));
    };
    let base = base_of(config);
    let cas = CasServer::new(base_url.clone(), *flavor);
    let mut net = Net::open(ctx, PROVIDER)?;
    // Pot vide : on repart de zéro, comme un navigateur qui n'a jamais visité ce CAS.
    net.browser.jar = CookieJar::default();
    let outcome = cas::open(
        &mut net.browser,
        &cas,
        auth_url(&base),
        Some(Credentials { username, password }),
    )
    .await
    .map_err(cas_problem)?;
    match outcome {
        Outcome::SignedIn {
            page,
            password_posted: true,
        } => landed(&page, &base)?,
        Outcome::SignedIn { .. } => {
            return Err(bad(
                "le CAS n'a pas demandé le mot de passe : connexion refusée",
            ));
        }
        Outcome::SecondFactor(_) => {
            return Err(Problem::new(ErrorCode::NotImplemented).detail(format!(
                "{PROVIDER} : second facteur du CAS de la composante pas encore pris en charge"
            )));
        }
    }
    store_cookies(ctx, &token_key(&base), &net.browser.jar);
    Ok(())
}

// Faux serveur ScoDoc + CAS, partagé avec `tests/scodoc_mock.rs`.
#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod support;

#[cfg(test)]
mod tests;
