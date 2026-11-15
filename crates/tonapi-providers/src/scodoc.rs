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
//! Source des formats : `docs/recherche/montpellier/scodoc.md` et `docs/recherche/rennes/scodoc.md`.

use std::fmt::Display;
use std::time::Duration;

use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde_json::Value;
use tonapi_config::{ScoDocGateway, SignIn};
use tonapi_core::{AbsenceReport, ErrorCode, GradeReport, Problem};
use tonapi_http::cas::{self, CasError, CasServer, Credentials, Outcome};
use tonapi_http::{CookieJar, Page};
use tonapi_session::StoredCookie;
use url::Url;

use crate::Context;
use crate::common::{self, university_tz};
use crate::net::{Net, http_problem, upstream};

mod parse;

use parse::{Reply, classify_reply, parse_absences, parse_grades};

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
    common::token_key("scodoc", base)
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
    let tz = university_tz(ctx)?;
    parse_grades(&fetch_data(ctx, config).await?, tz)
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
