//! Moodle : devoirs et échéances.
//!
//! Deux fournisseurs, une même lecture (`core_calendar_get_action_events_by_timesort`, les
//! événements « à faire » de la chronologie Moodle) :
//! - `moodle-ajax` : session web, entrée par le CAS ou la fédération Shibboleth de l'université,
//!   `sesskey` lu dans la page, appels à `lib/ajax/service.php` ;
//! - `moodle-webservice` : `webservice/rest/server.php` avec un jeton obtenu par `login/token.php`
//!   ([`sign_in`]). Le mot de passe ne sert qu'à cet appel : seul le jeton reste dans la session.
//!
//! Moodle renvoie ses erreurs en HTTP 200. Aucun message d'erreur de Moodle n'est recopié dans un
//! problème : seul son `errorcode` (nettoyé) peut y figurer.
//!
//! Emplacements dans `SessionData::tokens` (un par hôte Moodle) : `moodle.sesskey.<hôte>` et
//! `moodle.token.<hôte>`.

use std::collections::HashSet;

use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::Tz;
use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use tonapi_config::{HomeworkService, MoodleEntry, SamlConfig, SignIn, Sso};
use tonapi_core::{ErrorCode, Homework, Problem};
use tonapi_http::{Page, forms};
use url::Url;

use crate::Context;
use crate::net::{Net, http_problem, upstream};

const AJAX: &str = "moodle-ajax";
const WEBSERVICE: &str = "moodle-webservice";
/// Service mobile officiel de Moodle, le seul qui donne un jeton sans configuration particulière.
const MOBILE_SERVICE: &str = "moodle_mobile_app";
const EVENTS_FUNCTION: &str = "core_calendar_get_action_events_by_timesort";

/// Maximum accepté par Moodle pour `limitnum`.
const PAGE_SIZE: i64 = 50;
const MAX_PAGES: usize = 4;
/// Les devoirs en retard restent utiles : fenêtre de −14 j à +120 j.
const WINDOW_BEFORE_DAYS: i64 = 14;
const WINDOW_AFTER_DAYS: i64 = 120;
const MAX_SAML_STEPS: usize = 25;

/// `errorcode` qui disent « la session web est perdue » (Moodle, tafac et lent-ios).
const SESSION_CODES: [&str; 5] = [
    "servicerequireslogin",
    "requireloginerror",
    "invalidsesskey",
    "sessionerroruser",
    "sessiontimedout",
];
/// `errorcode` qui disent « le jeton est refusé » : `accessexception` couvre aussi un jeton expiré.
const TOKEN_CODES: [&str; 2] = ["invalidtoken", "accessexception"];

/// Fin de libellé ajoutée par Moodle à l'événement (« … doit être rendu »).
const NAME_SUFFIXES: [&str; 5] = [
    "doit être rendu",
    "doit être effectué",
    "est dû",
    "ferme",
    "se termine",
];

// ---------------------------------------------------------------------------------------------
// Emplacements dans la session
// ---------------------------------------------------------------------------------------------

/// `hôte[:port]` du Moodle.
fn authority(base: &Url) -> String {
    let host = base.host_str().unwrap_or_default();
    match base.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    }
}

fn sesskey_slot(base: &Url) -> String {
    format!("moodle.sesskey.{}", authority(base))
}

fn token_slot(base: &Url) -> String {
    format!("moodle.token.{}", authority(base))
}

fn timezone(ctx: &Context<'_>) -> Result<Tz, Problem> {
    ctx.university
        .university
        .timezone
        .parse::<Tz>()
        .map_err(|_| {
            Problem::new(ErrorCode::Internal).detail("fuseau horaire de l'université invalide")
        })
}

fn saml_of(ctx: &Context<'_>) -> Option<SamlConfig> {
    match &ctx.university.university.sso {
        Sso::Cas(cas) => cas.saml.clone(),
        Sso::Demo => None,
    }
}

fn bad_url(provider: &str) -> Problem {
    Problem::new(ErrorCode::Internal).detail(format!("{provider} : adresse du service invalide"))
}

// ---------------------------------------------------------------------------------------------
// Appels : erreurs et transport
// ---------------------------------------------------------------------------------------------

/// Échec d'un appel Moodle : session ou jeton refusé (on peut en refaire un), ou autre problème.
#[derive(Debug)]
enum CallError {
    SessionLost,
    Failed(Problem),
}

impl From<Problem> for CallError {
    fn from(problem: Problem) -> Self {
        CallError::Failed(problem)
    }
}

/// Où et avec quoi l'appel est fait.
enum Endpoint<'a> {
    Ajax { base: &'a Url, sesskey: &'a str },
    Webservice { base: &'a Url, token: &'a str },
}

impl Endpoint<'_> {
    fn provider(&self) -> &'static str {
        match self {
            Endpoint::Ajax { .. } => AJAX,
            Endpoint::Webservice { .. } => WEBSERVICE,
        }
    }
}

/// `errorcode` de Moodle : à la racine (REST, `token.php`) ou dans `exception` (AJAX).
fn error_code(value: &Value) -> Option<&str> {
    value.get("errorcode").and_then(Value::as_str).or_else(|| {
        value
            .get("exception")
            .and_then(|e| e.get("errorcode"))
            .and_then(Value::as_str)
    })
}

/// Ne garde que ce qui peut former un identifiant d'erreur Moodle.
fn safe_code(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(48)
        .collect()
}

/// Erreur Moodle → problème. Ni `message` ni `debuginfo` : ils peuvent citer le compte.
fn moodle_error(provider: &str, value: &Value, session_codes: &[&str]) -> CallError {
    let code = error_code(value)
        .map(safe_code)
        .filter(|code| !code.is_empty());
    match code.as_deref() {
        Some(code) if session_codes.contains(&code) => CallError::SessionLost,
        Some(code @ ("servicenotavailable" | "sitemaintenance")) => {
            CallError::Failed(Problem::new(ErrorCode::ServiceUnavailable).detail(format!(
                "{provider} : fonction ou site indisponible (`{code}`)"
            )))
        }
        Some(code) => CallError::Failed(upstream(provider, format!("erreur Moodle `{code}`"))),
        None => CallError::Failed(upstream(provider, "erreur Moodle sans code")),
    }
}

/// Contenu utile d'une réponse de `lib/ajax/service.php`.
///
/// Formes vues : `[{"error":false,"data":…}]`, `[{"error":true,"exception":{…}}]` et, quand toute
/// la requête est refusée, un objet `{"error":"…","errorcode":"…"}`.
fn ajax_data(response: Value) -> Result<Value, CallError> {
    let mut item = match response {
        Value::Array(mut items) if !items.is_empty() => items.swap_remove(0),
        other => other,
    };
    let refused = item.get("error") == Some(&Value::Bool(true)) || error_code(&item).is_some();
    if refused {
        return Err(moodle_error(AJAX, &item, &SESSION_CODES));
    }
    match item.get_mut("data") {
        Some(data) => Ok(data.take()),
        None => Err(upstream(AJAX, "réponse sans `data`").into()),
    }
}

/// Contenu utile d'une réponse de `webservice/rest/server.php` (erreur : `exception`).
fn webservice_data(response: Value) -> Result<Value, CallError> {
    if response.get("exception").is_some() || response.get("errorcode").is_some() {
        return Err(moodle_error(WEBSERVICE, &response, &TOKEN_CODES));
    }
    Ok(response)
}

/// Paires `nom=valeur` d'arguments scalaires, au format des paramètres de `server.php`.
fn form_pairs(args: &Value) -> Vec<(String, String)> {
    let Some(object) = args.as_object() else {
        return Vec::new();
    };
    object
        .iter()
        .filter_map(|(name, value)| {
            let text = match value {
                Value::Bool(flag) => String::from(if *flag { "1" } else { "0" }),
                Value::Number(number) => number.to_string(),
                Value::String(text) => text.clone(),
                _ => return None,
            };
            Some((name.clone(), text))
        })
        .collect()
}

fn status_problem(provider: &str, status: reqwest::StatusCode) -> Problem {
    Problem::new(ErrorCode::ServiceUnavailable).detail(format!("{provider} : HTTP {status}"))
}

async fn call(
    net: &mut Net,
    endpoint: &Endpoint<'_>,
    function: &str,
    args: &Value,
) -> Result<Value, CallError> {
    match endpoint {
        Endpoint::Ajax { base, sesskey } => call_ajax(net, base, sesskey, function, args).await,
        Endpoint::Webservice { base, token } => {
            call_webservice(net, base, token, function, args).await
        }
    }
}

async fn call_ajax(
    net: &mut Net,
    base: &Url,
    sesskey: &str,
    function: &str,
    args: &Value,
) -> Result<Value, CallError> {
    let mut url = base
        .join("lib/ajax/service.php")
        .map_err(|_| bad_url(AJAX))?;
    url.query_pairs_mut()
        .append_pair("sesskey", sesskey)
        .append_pair("info", function);
    let body = json!([{ "index": 0, "methodname": function, "args": args }]).to_string();

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("application/json, text/javascript, */*; q=0.01"),
    );
    headers.insert(
        "x-requested-with",
        HeaderValue::from_static("XMLHttpRequest"),
    );
    if let Ok(origin) = HeaderValue::from_str(&base.origin().ascii_serialization()) {
        headers.insert(header::ORIGIN, origin);
    }
    if let Some(referer) = base
        .join("my/")
        .ok()
        .and_then(|u| HeaderValue::from_str(u.as_str()).ok())
    {
        headers.insert(header::REFERER, referer);
    }

    let (status, _, text) = net.request(Method::POST, url, headers, Some(body)).await?;
    // Une session perdue peut se manifester par une redirection vers la page de connexion.
    if status.is_redirection() || matches!(status.as_u16(), 401 | 403) {
        return Err(CallError::SessionLost);
    }
    if !status.is_success() {
        return Err(status_problem(AJAX, status).into());
    }
    let response = serde_json::from_str(&text)
        .map_err(|_| upstream(AJAX, format!("réponse non JSON (HTTP {status})")))?;
    ajax_data(response)
}

async fn call_webservice(
    net: &mut Net,
    base: &Url,
    token: &str,
    function: &str,
    args: &Value,
) -> Result<Value, CallError> {
    let url = base
        .join("webservice/rest/server.php")
        .map_err(|_| bad_url(WEBSERVICE))?;
    // En POST : le jeton ne figure jamais dans une URL.
    let body = {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("wstoken", token)
            .append_pair("wsfunction", function)
            .append_pair("moodlewsrestformat", "json");
        for (name, value) in form_pairs(args) {
            form.append_pair(&name, &value);
        }
        form.finish()
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));

    let (status, _, text) = net.request(Method::POST, url, headers, Some(body)).await?;
    if !status.is_success() {
        return Err(status_problem(WEBSERVICE, status).into());
    }
    let response = serde_json::from_str(&text)
        .map_err(|_| upstream(WEBSERVICE, format!("réponse non JSON (HTTP {status})")))?;
    webservice_data(response)
}

/// Événements « à faire », page après page (4 pages de 50 au plus).
async fn fetch_events(
    net: &mut Net,
    endpoint: &Endpoint<'_>,
    now: DateTime<Utc>,
) -> Result<Vec<Value>, CallError> {
    let from = (now - Duration::days(WINDOW_BEFORE_DAYS)).timestamp();
    let to = (now + Duration::days(WINDOW_AFTER_DAYS)).timestamp();
    let mut events = Vec::new();
    let mut after = None;
    for _ in 0..MAX_PAGES {
        let mut args = json!({
            "timesortfrom": from,
            "timesortto": to,
            "limitnum": PAGE_SIZE,
            "limittononsuspendedevents": true,
        });
        if let Some(id) = after {
            args["aftereventid"] = json!(id);
        }
        let mut data = call(net, endpoint, EVENTS_FUNCTION, &args).await?;
        let last = data.get("lastid").and_then(Value::as_i64);
        let Some(Value::Array(page)) = data.get_mut("events").map(Value::take) else {
            return Err(upstream(endpoint.provider(), "réponse sans liste `events`").into());
        };
        let full = i64::try_from(page.len()).is_ok_and(|n| n >= PAGE_SIZE);
        events.extend(page);
        match last {
            Some(id) if full => after = Some(id),
            _ => break,
        }
    }
    Ok(events)
}

// ---------------------------------------------------------------------------------------------
// moodle-ajax
// ---------------------------------------------------------------------------------------------

/// Devoirs d'un Moodle ouvert par la session web de l'étudiant.
///
/// Le `sesskey` est gardé dans la session avec les cookies de Moodle. S'il est refusé, la session
/// du service est rouverte par le SSO de l'université et l'appel refait **une seule fois**.
/// Racine de Moodle avec sa barre finale : `…/moodle` + `login/token.php` donnerait sinon
/// `…/login/token.php`.
fn base_of(base_url: &Url) -> Url {
    let mut base = base_url.clone();
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base.set_query(None);
    base.set_fragment(None);
    base
}

pub(crate) async fn fetch_ajax(
    ctx: &mut Context<'_>,
    base: &Url,
    entry: MoodleEntry,
) -> Result<Vec<Homework>, Problem> {
    let base = &base_of(base);
    let tz = timezone(ctx)?;
    let saml = saml_of(ctx);
    let mut net = Net::open(ctx, AJAX)?;
    let events = ajax_events(ctx, &mut net, base, entry, saml.as_ref()).await;
    net.save(ctx);
    parse_homework(&events?, tz, AJAX)
}

async fn ajax_events(
    ctx: &mut Context<'_>,
    net: &mut Net,
    base: &Url,
    entry: MoodleEntry,
    saml: Option<&SamlConfig>,
) -> Result<Vec<Value>, Problem> {
    let slot = sesskey_slot(base);
    let mut stored = ctx.session.tokens.get(&slot).cloned();
    let mut reopened = false;
    loop {
        let sesskey = if let Some(key) = stored.take() {
            key
        } else {
            let key = open_session(net, base, entry, saml).await?;
            ctx.session.tokens.insert(slot.clone(), key.clone());
            reopened = true;
            key
        };
        let endpoint = Endpoint::Ajax {
            base,
            sesskey: &sesskey,
        };
        match fetch_events(net, &endpoint, ctx.now).await {
            Ok(events) => return Ok(events),
            Err(CallError::Failed(problem)) => return Err(problem),
            Err(CallError::SessionLost) => {
                ctx.session.tokens.remove(&slot);
                if reopened {
                    return Err(Problem::new(ErrorCode::SessionExpired)
                        .detail("Moodle refuse la session qui vient d'être ouverte"));
                }
            }
        }
    }
}

/// Même hôte (et port) que le Moodle.
fn on_moodle(url: &Url, base: &Url) -> bool {
    authority(url) == authority(base)
}

/// Ouvre une session web sur Moodle avec le SSO de l'université et rend son `sesskey`.
async fn open_session(
    net: &mut Net,
    base: &Url,
    entry: MoodleEntry,
    saml: Option<&SamlConfig>,
) -> Result<String, Problem> {
    let path = match entry {
        MoodleEntry::Cas => "login/index.php?authCAS=CAS",
        MoodleEntry::Shibboleth => "auth/shibboleth/index.php",
    };
    let url = base.join(path).map_err(|_| bad_url(AJAX))?;
    let mut page = net.open_protected(url).await?;
    if entry == MoodleEntry::Shibboleth {
        page = follow_saml(net, base, saml, page).await?;
    }
    session_key(net, base, page).await
}

/// `sesskey` de la page d'arrivée, ou de `/my/` si elle n'en a pas.
async fn session_key(net: &mut Net, base: &Url, mut page: Page) -> Result<String, Problem> {
    for attempt in 0..2 {
        if !on_moodle(&page.url, base) {
            return Err(upstream(
                AJAX,
                "la connexion n'a pas abouti sur le Moodle configuré",
            ));
        }
        match page_state(&page.body) {
            PageState::LoggedIn(key) => return Ok(key),
            // Sans TGC, le CAS renvoie à Moodle sans ticket : la page est celle d'un invité.
            PageState::NotLoggedIn => {
                return Err(Problem::new(ErrorCode::SessionExpired)
                    .detail("Moodle n'a pas reconnu la session de l'université : se reconnecter"));
            }
            PageState::NoSesskey if attempt == 0 => {
                let my = base.join("my/").map_err(|_| bad_url(AJAX))?;
                page = net
                    .browser
                    .get(my)
                    .await
                    .map_err(|e| http_problem(AJAX, &e))?;
            }
            PageState::NoSesskey => break,
        }
    }
    Err(upstream(AJAX, "page Moodle sans `sesskey`"))
}

// ---------------------------------------------------------------------------------------------
// Page Moodle : `sesskey`, connecté ou non
// ---------------------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum PageState {
    LoggedIn(String),
    /// `<body class="… notloggedin">` : page d'invité ou de connexion.
    NotLoggedIn,
    NoSesskey,
}

fn page_state(html: &str) -> PageState {
    if body_is_not_logged_in(html) {
        return PageState::NotLoggedIn;
    }
    sesskey_of(html).map_or(PageState::NoSesskey, PageState::LoggedIn)
}

fn body_is_not_logged_in(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    let Some(start) = lower.find("<body") else {
        return false;
    };
    let end = lower
        .get(start..)
        .and_then(|rest| rest.find('>'))
        .map_or(lower.len(), |e| start + e);
    lower
        .get(start..end)
        .is_some_and(|tag| tag.contains("notloggedin"))
}

/// `"sesskey":"<clé>"` dans le `M.cfg` de la page.
fn sesskey_of(html: &str) -> Option<String> {
    const NEEDLE: &str = "\"sesskey\"";
    let mut rest = html;
    while let Some(at) = rest.find(NEEDLE) {
        rest = rest.get(at + NEEDLE.len()..)?;
        let value = rest
            .trim_start()
            .strip_prefix(':')
            .map(str::trim_start)
            .and_then(|v| v.strip_prefix('"'));
        if let Some(value) = value {
            let key: String = value
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            if key.len() >= 6 && value.get(key.len()..).is_some_and(|r| r.starts_with('"')) {
                return Some(key);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Entrée Shibboleth : fournisseur de service → WAYF → IdP → CAS (TGC) → SAML → Moodle
// ---------------------------------------------------------------------------------------------

/// Hôte autorisé pendant la fédération : Moodle, le CAS de l'université, ou un suffixe configuré.
fn host_allowed(url: &Url, base: &Url, cas_host: Option<&str>, saml: &SamlConfig) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    on_moodle(url, base)
        || cas_host == Some(host)
        || saml
            .allowed_hosts
            .iter()
            .any(|suffix| host == suffix || host.ends_with(&format!(".{suffix}")))
}

fn resolve_action(from: &Url, action: Option<&str>) -> Result<Url, Problem> {
    match action.filter(|a| !a.is_empty()) {
        Some(action) => from
            .join(action)
            .map_err(|_| upstream(AJAX, "formulaire de connexion fédérée illisible")),
        None => Ok(from.clone()),
    }
}

/// Mène la chaîne SAML jusqu'à une page Moodle connectée. Aucune donnée n'est postée hors des
/// hôtes autorisés, et jamais de mot de passe : si un formulaire en demande un, la session de
/// l'université est perdue.
async fn follow_saml(
    net: &mut Net,
    base: &Url,
    saml: Option<&SamlConfig>,
    mut page: Page,
) -> Result<Page, Problem> {
    let Some(saml) = saml else {
        return Err(Problem::new(ErrorCode::Internal)
            .detail("entrée `shibboleth` sans `sso.saml` dans la configuration de l'université"));
    };
    let cas_host = net
        .cas
        .as_ref()
        .and_then(|cas| cas.base.host_str())
        .map(str::to_owned);
    let allowed = |url: &Url| host_allowed(url, base, cas_host.as_deref(), saml);

    for _ in 0..MAX_SAML_STEPS {
        if !allowed(&page.url) {
            return Err(upstream(AJAX, "la fédération sort des hôtes autorisés"));
        }
        if page.stopped_at.is_some() {
            return Err(upstream(AJAX, "redirection hors HTTP pendant la connexion"));
        }
        if on_moodle(&page.url, base) && matches!(page_state(&page.body), PageState::LoggedIn(_)) {
            return Ok(page);
        }
        let forms = forms::forms(&page.body);
        if forms.iter().any(|f| f.has_password) {
            return Err(Problem::new(ErrorCode::SessionExpired)
                .detail("l'université redemande le mot de passe : se reconnecter"));
        }

        let (target, fields) = if let Some(form) = forms.iter().find(|f| f.has("user_idp")) {
            let mut fields: Vec<(String, String)> = form
                .fields
                .iter()
                .filter(|(name, _)| name != "user_idp")
                .cloned()
                .collect();
            fields.push(("user_idp".to_owned(), saml.idp_entity_id.clone()));
            fields.push(
                form.buttons
                    .iter()
                    .find(|(name, _)| name == "Select")
                    .cloned()
                    .unwrap_or_else(|| ("Select".to_owned(), "Sélection".to_owned())),
            );
            (resolve_action(&page.url, form.action.as_deref())?, fields)
        } else if let Some(form) = forms
            .iter()
            .find(|f| f.has("SAMLResponse") || f.has("SAMLRequest"))
        {
            (
                resolve_action(&page.url, form.action.as_deref())?,
                form.fields.clone(),
            )
        } else if let Some(next) = html_redirect(&page.body, &page.url) {
            if !allowed(&next) {
                return Err(upstream(AJAX, "la fédération sort des hôtes autorisés"));
            }
            page = net
                .browser
                .get(next)
                .await
                .map_err(|e| http_problem(AJAX, &e))?;
            continue;
        } else {
            return Err(upstream(
                AJAX,
                "page inattendue pendant la connexion fédérée",
            ));
        };
        if !allowed(&target) {
            return Err(upstream(AJAX, "la fédération sort des hôtes autorisés"));
        }
        page = net
            .browser
            .post_form(target, &fields, &page.url)
            .await
            .map_err(|e| http_problem(AJAX, &e))?;
    }
    Err(upstream(AJAX, "trop d'étapes dans la connexion fédérée"))
}

/// Redirection écrite dans la page : `<meta http-equiv="refresh">` ou `location.href = "…"`.
fn html_redirect(body: &str, from: &Url) -> Option<Url> {
    let target = meta_refresh(body).or_else(|| script_redirect(body))?;
    from.join(&decode_entities(target)).ok()
}

fn meta_refresh(body: &str) -> Option<&str> {
    let lower = body.to_ascii_lowercase();
    for (start, _) in lower.match_indices("<meta") {
        // Sans `>` plus loin, aucune balise suivante ne se ferme non plus : on s'arrête (sinon
        // chaque `<meta` relirait toute la fin de la page).
        let Some(end) = lower
            .get(start..)
            .and_then(|rest| rest.find('>'))
            .map(|e| start + e)
        else {
            break;
        };
        let Some(tag) = lower.get(start..end) else {
            continue;
        };
        if !tag.contains("refresh") {
            continue;
        }
        let Some(at) = tag.find("url=") else {
            continue;
        };
        // `lower` et `body` ont les mêmes positions : seule la casse ASCII diffère.
        let value = body
            .get(start + at + 4..end)?
            .trim_start_matches(['\'', '"'])
            .split(['\'', '"'])
            .next()?
            .trim();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

fn script_redirect(body: &str) -> Option<&str> {
    for needle in ["location.replace(", "location.href", "window.location"] {
        let Some(at) = body.find(needle) else {
            continue;
        };
        let rest = body.get(at + needle.len()..)?.trim_start();
        let rest = rest.trim_start_matches(['=', '(']).trim_start();
        let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let inner = rest.get(1..)?;
        let value = inner.split(quote).next()?.trim();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// moodle-webservice
// ---------------------------------------------------------------------------------------------

/// Devoirs d'un Moodle ouvert par jeton.
///
/// Sans jeton, ou si Moodle le refuse, il faut une connexion au service ([`sign_in`]) : on ne peut
/// pas en refaire une ici sans le mot de passe.
pub(crate) async fn fetch_webservice(
    ctx: &mut Context<'_>,
    base: &Url,
    sign_in: &SignIn,
) -> Result<Vec<Homework>, Problem> {
    let base = &base_of(base);
    let tz = timezone(ctx)?;
    let slot = token_slot(base);
    let Some(token) = ctx.session.tokens.get(&slot).cloned() else {
        return Err(login_required(sign_in));
    };
    let mut net = Net::open(ctx, WEBSERVICE)?;
    let endpoint = Endpoint::Webservice {
        base,
        token: &token,
    };
    let result = fetch_events(&mut net, &endpoint, ctx.now).await;
    net.save(ctx);
    match result {
        Ok(events) => parse_homework(&events, tz, WEBSERVICE),
        Err(CallError::Failed(problem)) => Err(problem),
        Err(CallError::SessionLost) => {
            ctx.session.tokens.remove(&slot);
            Err(login_required(sign_in))
        }
    }
}

/// Ce que l'étudiant doit faire pour obtenir un jeton, selon la configuration.
fn login_required(sign_in: &SignIn) -> Problem {
    match sign_in {
        SignIn::LocalPassword { .. } => Problem::new(ErrorCode::ServiceLoginRequired).detail(
            "ce Moodle a son propre compte : connectez-vous au service pour obtenir un jeton",
        ),
        SignIn::UniversityCas { .. } | SignIn::OwnCas { .. } => no_token_by_sso(),
    }
}

/// `login/token.php` vérifie un mot de passe local : il échoue toujours pour un compte CAS ou
/// Shibboleth. Le jeton par `admin/tool/mobile/launch.php` n'a jamais été essayé sur un vrai
/// Moodle (Docs/Rennes/moodle.md §10.3).
fn no_token_by_sso() -> Problem {
    Problem::new(ErrorCode::NotImplemented).detail(
        "moodle-webservice : pas de jeton avec une connexion par CAS (`login/token.php` demande un \
         mot de passe local) ; utiliser `moodle-ajax` ou `sign-in = local-password`",
    )
}

/// Obtient un jeton Moodle avec l'identifiant et le mot de passe de l'étudiant (`login/token.php`).
///
/// Le mot de passe n'est envoyé qu'une fois, jamais rejoué : un `invalidlogin` répété peut
/// verrouiller le compte Moodle. Seul le jeton est gardé (`SessionData::tokens`), pas le
/// `privatetoken` ni le mot de passe.
pub(crate) async fn sign_in(
    ctx: &mut Context<'_>,
    config: &HomeworkService,
    username: &str,
    password: &str,
) -> Result<(), Problem> {
    let HomeworkService::MoodleWebservice {
        base_url,
        sign_in: method,
        ..
    } = config
    else {
        return Err(Problem::new(ErrorCode::InvalidRequest)
            .detail("ce fournisseur n'a pas de connexion séparée"));
    };
    match method {
        SignIn::LocalPassword { .. } => {}
        SignIn::UniversityCas { .. } | SignIn::OwnCas { .. } => return Err(no_token_by_sso()),
    }
    let url = base_of(base_url)
        .join("login/token.php")
        .map_err(|_| bad_url(WEBSERVICE))?;
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("username", username)
        .append_pair("password", password)
        .append_pair("service", MOBILE_SERVICE)
        .finish();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));

    let mut net = Net::open(ctx, WEBSERVICE)?;
    let reply = net.request(Method::POST, url, headers, Some(body)).await;
    net.save(ctx);
    let (status, _, text) = reply?;
    if !status.is_success() {
        return Err(status_problem(WEBSERVICE, status));
    }
    let response = serde_json::from_str(&text)
        .map_err(|_| upstream(WEBSERVICE, format!("réponse non JSON (HTTP {status})")))?;
    let token = parse_token(&response)?;
    ctx.session.tokens.insert(token_slot(base_url), token);
    Ok(())
}

/// Réponse de `login/token.php` : le jeton, ou un problème sans le message de Moodle.
fn parse_token(response: &Value) -> Result<String, Problem> {
    if let Some(token) = response
        .get("token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        return Ok(token.to_owned());
    }
    let code = error_code(response).map(safe_code).unwrap_or_default();
    Err(match code.as_str() {
        "invalidlogin" => Problem::new(ErrorCode::InvalidCredentials),
        "passwordisexpired" | "restoredaccountresetpassword" | "usernotconfirmed" => {
            Problem::new(ErrorCode::AccountActionRequired).detail(format!(
                "Moodle demande une action sur le compte (`{code}`) : à faire sur son site"
            ))
        }
        "sitemaintenance" | "servicenotavailable" | "enablewsdescription" => {
            Problem::new(ErrorCode::ServiceUnavailable)
                .detail(format!("{WEBSERVICE} : jeton indisponible (`{code}`)"))
        }
        "" => upstream(WEBSERVICE, "réponse sans jeton ni code d'erreur"),
        _ => upstream(WEBSERVICE, format!("erreur Moodle `{code}`")),
    })
}

// ---------------------------------------------------------------------------------------------
// Analyse pure : événements Moodle → devoirs
// ---------------------------------------------------------------------------------------------

/// Événements de `core_calendar_get_action_events_by_timesort` → devoirs, triés par échéance.
///
/// - `id`, un titre et une date (`timesort`, sinon `timestart`) sont obligatoires : sinon
///   `upstream_changed`. Un événement dont la date vaut 0 n'a pas d'échéance : ignoré.
/// - Moodle ne renvoie que les événements encore « à faire » : un devoir (`assign`) ou un quiz y
///   figure donc non rendu (`submitted = false`) ; pour les autres modules, l'information manque.
/// - Le même devoir publié dans plusieurs espaces de cours (même titre, même cours, même minute)
///   n'apparaît qu'une fois.
pub(crate) fn parse_homework(
    events: &[Value],
    tz: Tz,
    provider: &str,
) -> Result<Vec<Homework>, Problem> {
    let mut seen_ids = HashSet::new();
    let mut seen_content = HashSet::new();
    let mut homework = Vec::new();
    for event in events {
        let Some(item) = parse_event(event, tz, provider)? else {
            continue;
        };
        let content = (
            item.title.to_lowercase(),
            item.course.clone().unwrap_or_default().to_lowercase(),
            item.due.timestamp() / 60,
        );
        if seen_ids.insert(item.id.clone()) && seen_content.insert(content) {
            homework.push(item);
        }
    }
    homework.sort_by(|a, b| a.due.cmp(&b.due).then_with(|| a.title.cmp(&b.title)));
    Ok(homework)
}

fn parse_event(event: &Value, tz: Tz, provider: &str) -> Result<Option<Homework>, Problem> {
    if !event.is_object() {
        return Err(upstream(provider, "événement illisible"));
    }
    let id = match event.get("id") {
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::String(text)) if !text.is_empty() => text.clone(),
        _ => return Err(upstream(provider, "événement sans `id`")),
    };
    let timestamp = event
        .get("timesort")
        .or_else(|| event.get("timestart"))
        .and_then(epoch)
        .ok_or_else(|| upstream(provider, "événement sans date (`timesort`)"))?;
    if timestamp <= 0 {
        return Ok(None);
    }
    let due = tz
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|d| d.fixed_offset())
        .ok_or_else(|| upstream(provider, "date d'événement hors limites"))?;
    let title = text_field(event, "activityname")
        .or_else(|| text_field(event, "name").map(|name| clean_name(&name)))
        .ok_or_else(|| upstream(provider, "événement sans titre (`name`)"))?;

    let course = event
        .get("course")
        .and_then(|c| text_field(c, "fullname").or_else(|| text_field(c, "shortname")));
    let description = event
        .get("description")
        .and_then(Value::as_str)
        .map(html_to_text)
        .filter(|text| !text.is_empty());
    // Page du devoir, sinon page de l'action ; une adresse qui n'est pas http(s) est écartée.
    let url = [
        event.get("url"),
        event.get("action").and_then(|a| a.get("url")),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .find(|u| Url::parse(u).is_ok_and(|u| matches!(u.scheme(), "http" | "https")))
    .map(str::to_owned);
    let submitted = match event.get("modulename").and_then(Value::as_str) {
        Some("assign" | "quiz") if event.get("action").is_some_and(Value::is_object) => Some(false),
        _ => None,
    };
    Ok(Some(Homework {
        id,
        title,
        course,
        due,
        description,
        url,
        submitted,
    }))
}

/// Secondes Unix, en nombre ou en chaîne.
fn epoch(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// Champ texte non vide, entités HTML décodées.
fn text_field(object: &Value, key: &str) -> Option<String> {
    let text = decode_entities(object.get(key)?.as_str()?);
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Retire « doit être rendu », « ferme »… que Moodle ajoute au nom de l'activité.
fn clean_name(name: &str) -> String {
    let name = name.trim();
    let total = name.chars().count();
    for suffix in NAME_SUFFIXES {
        let length = suffix.chars().count();
        let Some(keep) = total.checked_sub(length).filter(|keep| *keep > 0) else {
            continue;
        };
        let tail: String = name.chars().skip(keep).collect();
        if tail.to_lowercase() == suffix {
            return name
                .chars()
                .take(keep)
                .collect::<String>()
                .trim_end()
                .to_owned();
        }
    }
    name.to_owned()
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        "laquo" => Some('«'),
        "raquo" => Some('»'),
        _ => {
            let digits = name.strip_prefix('#')?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// `&amp;`, `&#039;`, `&#x27;`… ; une entité inconnue reste telle quelle.
fn decode_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('&') {
        out.push_str(rest.get(..start).unwrap_or_default());
        let tail = rest.get(start..).unwrap_or_default();
        let decoded = tail
            .find(';')
            .filter(|end| *end <= 10)
            .and_then(|end| Some((entity(tail.get(1..end)?)?, end + 1)));
        if let Some((character, length)) = decoded {
            out.push(character);
            rest = tail.get(length..).unwrap_or_default();
        } else {
            out.push('&');
            rest = tail.get(1..).unwrap_or_default();
        }
    }
    out.push_str(rest);
    out
}

/// Texte d'un fragment HTML : balises retirées (un espace pour les blocs), entités décodées,
/// espaces réduits.
fn html_to_text(html: &str) -> String {
    const BLOCKS: [&str; 12] = [
        "br", "p", "div", "li", "ul", "ol", "tr", "td", "h1", "h2", "h3", "h4",
    ];
    let mut text = String::with_capacity(html.len());
    let mut chars = html.chars();
    while let Some(c) = chars.next() {
        if c != '<' {
            text.push(c);
            continue;
        }
        let tag: String = chars.by_ref().take_while(|t| *t != '>').collect();
        let name = tag
            .trim_start_matches('/')
            .split(|t: char| !t.is_ascii_alphanumeric())
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if BLOCKS.contains(&name.as_str()) {
            text.push(' ');
        }
    }
    decode_entities(&text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests;
