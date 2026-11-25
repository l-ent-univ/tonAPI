//! Fournisseur `scodoc-gateway` : notes et absences par la passerelle ScoDoc des IUT
//! (`Scodoc_Notes`, `services/data.php`).
//!
//! La passerelle renvoie tout en un seul appel, `dataPremièreConnexion` : relevé du semestre
//! courant (bulletin ScoDoc au format BUT), détail des absences par jour et totaux. Les notes et les
//! absences se lisent donc dans la même réponse, avec deux analyses pures.
//!
//! Autour du relevé : celui d'un autre semestre (`relevéEtudiant`), le message d'un département
//! (`getReportPageMessage`), la répartition des notes d'une évaluation (`listeNotes`) et le relevé
//! PDF (`bulletin_PDF.php`).
//!
//! Emplacements dans `SessionData::tokens` : `scodoc.<hôte>` (cookies d'un service à compte propre)
//! et `scodoc-student.<hôte>` (identifiant de l'étudiant pour la passerelle et type de son
//! bulletin, pour le relevé PDF).
//!
//! Connexion, selon `sign-in` :
//! - `university-cas` : le TGC de la session ouvre `services/doAuth.php` ([`Net::open_protected`]),
//!   sans mot de passe. Les cookies (dont `PHPSESSID`) repartent dans la session de l'étudiant.
//! - `own-cas` : CAS propre à la composante, avec d'autres identifiants. [`sign_in`] ouvre la session
//!   et ne garde que les cookies dans `tokens["scodoc.<hôte>"]` ; jamais le mot de passe. Les
//!   requêtes suivantes n'utilisent que ce jeton, jamais les cookies de l'université.
//!
//! Source des formats : `docs/recherche/montpellier/scodoc.md` et `docs/recherche/rennes/scodoc.md`.

use std::fmt::Display;
use std::time::Duration;

use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tonapi_config::{ScoDocGateway, SignIn};
use tonapi_core::{AbsenceReport, ErrorCode, GradeDistribution, GradeNotice, GradeReport, Problem};
use tonapi_http::cas::{self, CasError, CasServer, Credentials, Outcome};
use tonapi_http::{CookieJar, Page};
use tonapi_session::StoredCookie;
use url::Url;

use crate::common::{self, university_tz};
use crate::net::{Net, http_problem, upstream};
use crate::{Context, Download};

mod parse;

use parse::{
    Reply, classify_reply, hidden_distribution, is_published, owns_evaluation, parse_absences,
    parse_distribution, parse_grades, parse_notice, parse_report, student_id, transcript_type,
};

const PROVIDER: &str = "scodoc-gateway";
/// Cookie de session PHP de la passerelle.
const SESSION_COOKIE: &str = "PHPSESSID";
/// Délai d'une requête quand `options.timeout-secs` est absent.
const DEFAULT_TIMEOUT_SECS: u32 = 20;

fn bad(what: impl Display) -> Problem {
    upstream(PROVIDER, what)
}

/// Racine de la passerelle, avec une barre finale.
fn base_of(config: &ScoDocGateway) -> Url {
    common::root(&config.base_url)
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

/// Une action de `data.php` (`q=`) et ses paramètres.
struct Query<'a> {
    action: &'a str,
    params: &'a [(&'a str, &'a str)],
}

/// Tout en un : réglages, semestres, relevé du semestre courant, absences.
const FIRST_CONNECTION: Query<'static> = Query {
    action: "dataPremièreConnexion",
    params: &[],
};

/// `services/data.php?q=<action>&…` (les accents sont encodés en UTF-8).
fn data_url(base: &Url, query: &Query<'_>) -> Url {
    let mut url = base
        .join("services/data.php")
        .unwrap_or_else(|_| base.clone());
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("q", query.action);
        for (name, value) in query.params {
            pairs.append_pair(name, value);
        }
    }
    url
}

/// `services/bulletin_PDF.php?type=…&sem_id=…&etudiant=…`.
fn transcript_url(base: &Url, kind: &str, semester: &str, student: &str) -> Url {
    let mut url = base
        .join("services/bulletin_PDF.php")
        .unwrap_or_else(|_| base.clone());
    url.query_pairs_mut()
        .append_pair("type", kind)
        .append_pair("sem_id", semester)
        .append_pair("etudiant", student);
    url
}

/// Un identifiant de semestre ou d'évaluation ScoDoc : un entier.
fn scodoc_id<'a>(id: &'a str, what: &str) -> Result<&'a str, Problem> {
    let id = id.trim();
    if (1..=10).contains(&id.len()) && id.chars().all(|c| c.is_ascii_digit()) {
        Ok(id)
    } else {
        Err(Problem::new(ErrorCode::InvalidRequest)
            .detail(format!("{PROVIDER} : identifiant {what} invalide")))
    }
}

/// Clé de `SessionData::tokens` du service : une connexion sert aux notes et aux absences.
fn token_key(base: &Url) -> String {
    common::token_key("scodoc", base)
}

/// Clé du jeton où reste ce que `bulletin_PDF.php` demande : l'identifiant de l'étudiant pour la
/// passerelle et le type de son bulletin. Les lire coûte une lecture complète du relevé.
fn student_key(base: &Url) -> String {
    common::token_key("scodoc-student", base)
}

/// Ce que la première lecture apprend pour le relevé PDF, gardé dans la session de l'étudiant.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Student {
    id: String,
    /// `BUT` ou `DUT`.
    kind: String,
}

fn remember_student(ctx: &mut Context<'_>, base: &Url, first: &Value) {
    let Some(id) = student_id(first) else {
        return;
    };
    let student = Student {
        id,
        kind: transcript_type(first).to_owned(),
    };
    if let Ok(json) = serde_json::to_string(&student) {
        ctx.session.tokens.insert(student_key(base), json);
    }
}

fn remembered_student(ctx: &Context<'_>, base: &Url) -> Option<Student> {
    serde_json::from_str(ctx.session.tokens.get(&student_key(base))?).ok()
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

fn request_headers(base: &Url, accept: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static(accept));
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    if let Ok(referer) = HeaderValue::from_str(base.as_str()) {
        headers.insert(header::REFERER, referer);
    }
    headers
}

/// `{"redirect":"/services/doAuth.php"}` en HTTP 200 : la session PHP est perdue.
fn session_lost(value: &Value) -> bool {
    value.get("redirect").is_some()
}

/// Session ouverte sur la passerelle, le temps d'une lecture : cookies de l'université
/// (`university-cas`) ou pot de cookies du service seul (`own-cas`).
struct Gateway<'a> {
    net: Net,
    config: &'a ScoDocGateway,
    base: Url,
    timeout: Duration,
    /// Clé du jeton où repartent les cookies d'un service à compte propre.
    own: Option<String>,
}

impl<'a> Gateway<'a> {
    fn open(ctx: &Context<'_>, config: &'a ScoDocGateway) -> Result<Self, Problem> {
        let base = base_of(config);
        let timeout = Duration::from_secs(u64::from(
            config.options.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS),
        ));
        let mut net = Net::open(ctx, PROVIDER)?;
        let own = match &config.sign_in {
            SignIn::UniversityCas { .. } => None,
            SignIn::OwnCas {
                base_url, flavor, ..
            } => {
                let key = token_key(&base);
                let Some(cookies) = stored_cookies(ctx, &key) else {
                    return Err(login_required("aucune session ouverte sur ce service"));
                };
                // Pot de cookies du service seul : ceux de l'université n'ont rien à faire ici, et
                // ils ne doivent pas être écrasés à la fermeture.
                net.browser.jar = CookieJar::from_stored(cookies);
                net.cas = Some(CasServer::new(base_url.clone(), *flavor));
                Some(key)
            }
            SignIn::LocalPassword { .. } => {
                return Err(Problem::new(ErrorCode::NotImplemented).detail(format!(
                    "{PROVIDER} : la connexion `local-password` n'existe pas pour ScoDoc"
                )));
            }
        };
        Ok(Gateway {
            net,
            config,
            base,
            timeout,
            own,
        })
    }

    /// Remet les cookies dans la session de l'étudiant.
    fn close(self, ctx: &mut Context<'_>) {
        match self.own {
            Some(key) => store_cookies(ctx, &key, &self.net.browser.jar),
            None => self.net.save(ctx),
        }
    }

    fn has_session(&self) -> bool {
        let host = self.base.host_str().unwrap_or_default();
        self.net.browser.jar.contains(host, SESSION_COOKIE)
    }

    async fn sign_in(&mut self) -> Result<(), Problem> {
        open_session(&mut self.net, self.config, &self.base).await
    }

    fn refused(&self) -> Problem {
        if matches!(self.config.sign_in, SignIn::OwnCas { .. }) {
            login_required("la passerelle refuse la session")
        } else {
            Problem::new(ErrorCode::SessionExpired).detail(format!(
                "{PROVIDER} : la passerelle refuse la session après une nouvelle connexion"
            ))
        }
    }

    /// Un appel de `data.php`. `dataPremièreConnexion` part en `POST` à corps vide, comme le site
    /// de la passerelle ; les autres actions en `GET`.
    async fn call(&mut self, query: &Query<'_>) -> Result<Value, Problem> {
        let first = query.action == FIRST_CONNECTION.action;
        let (method, body) = if first {
            (Method::POST, Some(String::new()))
        } else {
            (Method::GET, None)
        };
        let (status, _, body) = self
            .net
            .browser
            .send(
                method,
                data_url(&self.base, query),
                request_headers(&self.base, "application/json"),
                body,
                Some(self.timeout),
            )
            .await
            .map_err(|e| http_problem(PROVIDER, &e))?;
        if status.is_server_error() {
            return Err(Problem::new(ErrorCode::ServiceUnavailable)
                .detail(format!("{PROVIDER} : HTTP {status}")));
        }
        serde_json::from_str(&body).map_err(|_| bad(format!("réponse non JSON (HTTP {status})")))
    }

    /// Réponse d'une action, telle quelle. Avec un `PHPSESSID` déjà posé, on essaie d'abord sans
    /// se reconnecter ; si la passerelle répond `{"redirect"}`, la session est rouverte et l'appel
    /// est refait **une seule fois**.
    async fn reply(&mut self, query: &Query<'_>) -> Result<Value, Problem> {
        if self.has_session() {
            let value = self.call(query).await?;
            if !session_lost(&value) {
                return Ok(value);
            }
        }
        self.sign_in().await?;
        let value = self.call(query).await?;
        if session_lost(&value) {
            return Err(self.refused());
        }
        Ok(value)
    }

    /// Données d'une action : un objet JSON, sans erreur applicative de la passerelle.
    async fn data(&mut self, query: &Query<'_>) -> Result<Value, Problem> {
        match classify_reply(self.reply(query).await?)? {
            Reply::Data(value) => Ok(value),
            Reply::SessionLost => Err(self.refused()),
        }
    }

    /// Un essai de `bulletin_PDF.php` : le fichier s'il commence comme un PDF.
    async fn transcript_once(&mut self, url: &Url) -> Result<Option<Vec<u8>>, Problem> {
        let (status, _, bytes) = self
            .net
            .browser
            .send_bytes(
                Method::POST,
                url.clone(),
                request_headers(&self.base, "application/pdf, */*"),
                Some(String::new()),
                Some(self.timeout),
            )
            .await
            .map_err(|e| http_problem(PROVIDER, &e))?;
        if status.is_server_error() {
            return Err(Problem::new(ErrorCode::ServiceUnavailable)
                .detail(format!("{PROVIDER} : HTTP {status}")));
        }
        Ok((status.is_success() && bytes.starts_with(b"%PDF")).then_some(bytes))
    }
}

/// Relevé PDF au plus : un bulletin fait quelques centaines de kilo-octets.
const MAX_TRANSCRIPT_BYTES: usize = 20 * 1024 * 1024;

async fn fetch_data(ctx: &mut Context<'_>, config: &ScoDocGateway) -> Result<Value, Problem> {
    let mut gateway = Gateway::open(ctx, config)?;
    let result = gateway.data(&FIRST_CONNECTION).await;
    let base = gateway.base.clone();
    gateway.close(ctx);
    if let Ok(first) = &result {
        remember_student(ctx, &base, first);
    }
    result
}

/// Notes du semestre courant, ou du semestre demandé (un identifiant de `GradeReport::available`).
pub(crate) async fn fetch_grades(
    ctx: &mut Context<'_>,
    config: &ScoDocGateway,
    semester: Option<&str>,
) -> Result<GradeReport, Problem> {
    let tz = university_tz(ctx)?;
    let Some(semester) = semester else {
        return parse_grades(&fetch_data(ctx, config).await?, tz);
    };
    let semester = scodoc_id(semester, "de semestre")?;
    let query = Query {
        action: "relevéEtudiant",
        params: &[("semestre", semester)],
    };
    let mut gateway = Gateway::open(ctx, config)?;
    let result = gateway.data(&query).await;
    gateway.close(ctx);
    parse_report(&result?, tz, Some(semester))
}

/// Message du département affiché au-dessus de ses relevés.
pub(crate) async fn fetch_notice(
    ctx: &mut Context<'_>,
    config: &ScoDocGateway,
    department: &str,
) -> Result<GradeNotice, Problem> {
    let query = Query {
        action: "getReportPageMessage",
        params: &[("dep", department)],
    };
    let mut gateway = Gateway::open(ctx, config)?;
    let result = gateway.data(&query).await;
    gateway.close(ctx);
    Ok(parse_notice(&result?))
}

/// Répartition des notes de la promotion à une évaluation du relevé de l'étudiant.
///
/// Le relevé du semestre est relu d'abord : la passerelle donnerait la répartition de n'importe
/// quelle évaluation de l'établissement, et n'en affiche pas pour un relevé non publié.
pub(crate) async fn fetch_distribution(
    ctx: &mut Context<'_>,
    config: &ScoDocGateway,
    semester: &str,
    evaluation: &str,
) -> Result<GradeDistribution, Problem> {
    let semester = scodoc_id(semester, "de semestre")?;
    let evaluation = scodoc_id(evaluation, "d'évaluation")?;
    let mut gateway = Gateway::open(ctx, config)?;
    let result = distribution_with(&mut gateway, semester, evaluation).await;
    gateway.close(ctx);
    result
}

async fn distribution_with(
    gateway: &mut Gateway<'_>,
    semester: &str,
    evaluation: &str,
) -> Result<GradeDistribution, Problem> {
    let report = gateway
        .data(&Query {
            action: "relevéEtudiant",
            params: &[("semestre", semester)],
        })
        .await?;
    if !owns_evaluation(&report, evaluation) {
        return Err(Problem::new(ErrorCode::NotFound)
            .detail("cette évaluation n'est pas dans le relevé de ce semestre"));
    }
    if !is_published(&report) {
        return Ok(hidden_distribution());
    }
    let grades = gateway
        .reply(&Query {
            action: "listeNotes",
            params: &[("eval", evaluation)],
        })
        .await?;
    parse_distribution(&grades)
}

/// Relevé PDF d'un semestre, produit par la passerelle.
///
/// `bulletin_PDF.php` veut l'identifiant de l'étudiant : il vient de la passerelle
/// (`dataPremièreConnexion`), jamais de la demande du client. Une lecture du relevé le laisse dans
/// la session de l'étudiant : le PDF se demande alors sans relire le relevé.
pub(crate) async fn fetch_transcript(
    ctx: &mut Context<'_>,
    config: &ScoDocGateway,
    semester: &str,
) -> Result<Download, Problem> {
    let semester = scodoc_id(semester, "de semestre")?;
    let mut gateway = Gateway::open(ctx, config)?;
    let known = remembered_student(ctx, &gateway.base);
    let result = transcript_with(&mut gateway, semester, known).await;
    let base = gateway.base.clone();
    gateway.close(ctx);
    let (bytes, first) = result?;
    if let Some(first) = &first {
        remember_student(ctx, &base, first);
    }
    Ok(Download {
        content_type: "application/pdf".to_owned(),
        file_name: format!("releve-{semester}.pdf"),
        bytes,
    })
}

/// Le PDF, et la première lecture s'il a fallu la faire pour connaître l'étudiant.
async fn transcript_with(
    gateway: &mut Gateway<'_>,
    semester: &str,
    known: Option<Student>,
) -> Result<(Vec<u8>, Option<Value>), Problem> {
    let (student, first) = if let Some(student) = known {
        (student, None)
    } else {
        let first = gateway.data(&FIRST_CONNECTION).await?;
        let id = student_id(&first)
            .ok_or_else(|| bad("identifiant de l'étudiant absent de la session"))?;
        let kind = transcript_type(&first).to_owned();
        (Student { id, kind }, Some(first))
    };
    let url = transcript_url(&gateway.base, &student.kind, semester, &student.id);
    // Avec une session déjà ouverte, elle n'est rouverte que si le PDF ne vient pas.
    let mut pdf = if gateway.has_session() {
        gateway.transcript_once(&url).await?
    } else {
        None
    };
    if pdf.is_none() {
        gateway.sign_in().await?;
        pdf = gateway.transcript_once(&url).await?;
    }
    match pdf {
        Some(bytes) if bytes.len() <= MAX_TRANSCRIPT_BYTES => Ok((bytes, first)),
        Some(_) => Err(bad("relevé PDF démesuré")),
        None => Err(Problem::new(ErrorCode::NotFound)
            .detail(format!("{PROVIDER} : pas de relevé PDF pour ce semestre"))),
    }
}

/// Absences du semestre courant.
pub(crate) async fn fetch_absences(
    ctx: &mut Context<'_>,
    config: &ScoDocGateway,
) -> Result<AbsenceReport, Problem> {
    let tz = university_tz(ctx)?;
    parse_absences(&fetch_data(ctx, config).await?, tz)
}

/// Notes et absences d'une seule lecture de la passerelle : `data.php` les donne ensemble, et une
/// session PHP traite ses requêtes l'une après l'autre.
pub(crate) async fn fetch_both(
    ctx: &mut Context<'_>,
    config: &ScoDocGateway,
) -> (Result<GradeReport, Problem>, Result<AbsenceReport, Problem>) {
    let data = match university_tz(ctx) {
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
