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

use chrono::{DateTime, Duration, Utc};
use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use tonapi_config::{HomeworkService, MoodleEntry, SamlConfig, SignIn, Sso};
use tonapi_core::{ErrorCode, Homework, Problem};
use tonapi_http::Page;
use url::Url;

use crate::Context;
use crate::common::{root, token_key, university_tz};
use crate::net::{Net, http_problem, upstream};

mod parse;
mod saml;

use parse::{PageState, ajax_data, page_state, parse_homework, parse_token, webservice_data};
use saml::follow_saml;

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

// Emplacements dans la session

fn sesskey_slot(base: &Url) -> String {
    token_key("moodle.sesskey", base)
}

fn token_slot(base: &Url) -> String {
    token_key("moodle.token", base)
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

// Appels : erreurs et transport

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

// moodle-ajax

/// Devoirs d'un Moodle ouvert par la session web de l'étudiant.
///
/// Le `sesskey` est gardé dans la session avec les cookies de Moodle. S'il est refusé, la session
/// du service est rouverte par le SSO de l'université et l'appel refait **une seule fois**.
pub(crate) async fn fetch_ajax(
    ctx: &mut Context<'_>,
    base: &Url,
    entry: MoodleEntry,
) -> Result<Vec<Homework>, Problem> {
    let base = &root(base);
    let tz = university_tz(ctx)?;
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
    url.host_str().unwrap_or_default() == base.host_str().unwrap_or_default()
        && url.port() == base.port()
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
// moodle-webservice

/// Devoirs d'un Moodle ouvert par jeton.
///
/// Sans jeton, ou si Moodle le refuse, il faut une connexion au service ([`sign_in`]) : on ne peut
/// pas en refaire une ici sans le mot de passe.
pub(crate) async fn fetch_webservice(
    ctx: &mut Context<'_>,
    base: &Url,
    sign_in: &SignIn,
) -> Result<Vec<Homework>, Problem> {
    let base = &root(base);
    let tz = university_tz(ctx)?;
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
/// Moodle (Docs/recherche/Rennes/moodle.md §10.3).
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
    let url = root(base_url)
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

#[cfg(test)]
mod tests;
