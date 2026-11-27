//! Le webmail Zimbra d'un étudiant, ouvert avec sa session CAS : la connexion (SAML, CAS,
//! Partage), puis les requêtes SOAP, REST (pièces jointes) et l'envoi de fichiers avec la session
//! du webmail obtenue. Repris de `establishZimbraContext`, `zimbraSoapRequest` et du servlet
//! d'envoi de Tafac (`server/entAuthApp.js`, `server/mailUpload.js`).
//!
//! # Connexion
//!
//! Webmail → fournisseur de service → fournisseur d'identité → CAS → … → webmail, avec une **copie**
//! des cookies de la session : ceux posés en route ne reviennent jamais dans la session de
//! l'étudiant. Chaque saut est vérifié **avant** d'être demandé : `https` seulement, et un hôte de
//! la liste des domaines de la connexion du webmail (ou un de leurs sous-domaines), cette liste
//! seule. L'origine finale du webmail est vérifiée de même, une fois passée en `https` : toutes
//! les requêtes suivantes, avec les cookies, le jeton CSRF et les fichiers de l'étudiant, y vont.
//!
//! Les formulaires qui se soumettent seuls (le POST SAML) et les redirections de page (`<meta>`,
//! `location = …`) sont suivis ; leur cible passe la même vérification. Vingt sauts au plus,
//! quinze secondes chacun, quarante-cinq en tout. La page de connexion du CAS (champ `execution`)
//! dit que la session CAS est finie.
//!
//! # Requêtes
//!
//! SOAP en JSON (`application/json`) : les valeurs sont encodées en JSON, aucune injection XML
//! possible. Une session refusée (`AUTH_REQUIRED`, `AUTH_EXPIRED`, HTTP 401, 403 ou 3xx, une page
//! là où devait être du JSON) se distingue des autres échecs : seule elle justifie une nouvelle
//! connexion. Rien de ce module n'écrit un jeton, un cookie, une adresse avec paramètres ou un
//! corps dans un message d'erreur.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};
use tonapi_http::{Browser, BrowserOptions, CookieJar, HttpError, RequestBody, Streamed, forms};
use tonapi_session::StoredCookie;
use url::Url;

use crate::NetSettings;

/// Cookie de session de Zimbra.
pub const TOKEN_COOKIE: &str = "ZM_AUTH_TOKEN";
/// Sauts au plus d'une connexion.
const MAX_HOPS: usize = 20;
/// Délai d'une requête au webmail (un saut, une requête SOAP, l'en-tête d'une pièce jointe).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Délai total d'une connexion.
const SIGN_IN_DEADLINE: Duration = Duration::from_secs(45);
/// Délai de la fin de session à la déconnexion.
pub const LOGOUT_TIMEOUT: Duration = Duration::from_secs(4);
const ACCEPT_HTML: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

// ── Liste des domaines ──────────────────────────────────────────────────────

/// Un domaine de la liste tel qu'il est écrit : minuscules, sans point aux bords.
fn domain(written: &str) -> Option<String> {
    let domain = written.trim().trim_matches('.').to_ascii_lowercase();
    (!domain.is_empty()
        && domain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
    .then_some(domain)
}

/// Domaines où la connexion au webmail peut passer.
#[derive(Debug, Clone)]
pub struct SignInDomains {
    /// Un domaine et ses sous-domaines.
    domains: Vec<String>,
    /// Un nom sans point (`localhost`) : cet hôte seul, jamais un domaine de premier niveau entier.
    hosts: Vec<String>,
    https_only: bool,
}

impl SignInDomains {
    /// `written` : les domaines de la connexion du webmail, et les hôtes du webmail et du CAS.
    /// `https_only` n'est faux que dans les tests (faux serveurs en `http`).
    #[must_use]
    pub fn new<'a>(written: impl IntoIterator<Item = &'a str>, https_only: bool) -> Self {
        let (mut domains, mut hosts): (Vec<String>, Vec<String>) = written
            .into_iter()
            .filter_map(domain)
            .partition(|d| d.contains('.'));
        domains.sort();
        domains.dedup();
        hosts.sort();
        hosts.dedup();
        SignInDomains {
            domains,
            hosts,
            https_only,
        }
    }

    /// L'adresse peut être demandée : `https`, et un hôte égal à un domaine de la liste ou qui
    /// finit par `.` + ce domaine (`evil-univ-rennes.fr` ne passe pas pour `univ-rennes.fr`).
    #[must_use]
    pub fn allows(&self, url: &Url) -> bool {
        let scheme_ok = url.scheme() == "https" || (!self.https_only && url.scheme() == "http");
        let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
            return false;
        };
        scheme_ok
            && url.username().is_empty()
            && url.password().is_none()
            && (self.hosts.contains(&host)
                || self
                    .domains
                    .iter()
                    .any(|d| host == *d || host.ends_with(&format!(".{d}"))))
    }
}

// ── Lecture des pages de la connexion ───────────────────────────────────────

/// Indice ASCII, sans tenir compte de la casse, de `needle` dans `haystack` à partir de `from`.
fn find_ci(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    let hay = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || from.saturating_add(needle.len()) > hay.len() {
        return None;
    }
    (from..=hay.len() - needle.len())
        .find(|&at| hay[at..at + needle.len()].eq_ignore_ascii_case(needle))
}

/// `extractZimbraCsrfToken` : `csrfToken` suivi de `=` ou `:`, puis une valeur entre guillemets
/// d'au moins huit caractères.
fn csrf_token(html: &str) -> Option<String> {
    let mut from = 0;
    while let Some(at) = find_ci(html, "csrftoken", from) {
        from = at + 1;
        let rest = html[at + "csrftoken".len()..]
            .trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '\u{a0}');
        let Some(rest) = rest.strip_prefix(['=', ':']) else {
            continue;
        };
        let rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '\u{a0}');
        if !rest.starts_with(['"', '\'']) {
            continue;
        }
        let value: String = rest[1..]
            .chars()
            .take_while(|c| !matches!(c, '"' | '\''))
            .collect();
        let closed = rest[1 + value.len()..].starts_with(['"', '\'']);
        if closed && value.chars().count() >= 8 {
            return Some(value);
        }
    }
    None
}

/// La page de connexion du CAS : un champ `execution`.
fn is_cas_login_form(html: &str) -> bool {
    forms::forms(html).iter().any(|form| {
        form.value("execution")
            .is_some_and(|value| !value.is_empty())
    })
}

/// `extractAutoSubmitForm` : un formulaire qui se soumet seul (`.submit()` dans la page) → son
/// action et ses champs.
fn auto_submit_form(html: &str, page: &Url) -> Option<(Url, Vec<(String, String)>)> {
    let submits = {
        let mut from = 0;
        let mut found = false;
        while let Some(at) = find_ci(html, ".submit", from) {
            from = at + 1;
            let rest = html[at + ".submit".len()..].trim_start();
            if let Some(rest) = rest.strip_prefix('(')
                && rest.trim_start().starts_with(')')
            {
                found = true;
                break;
            }
        }
        found
    };
    if !submits {
        return None;
    }
    let form = forms::forms(html)
        .into_iter()
        .find(|form| form.action.as_deref().is_some_and(|a| !a.is_empty()))?;
    let action = page.join(form.action.as_deref()?).ok()?;
    Some((action, form.fields))
}

/// Valeur entre guillemets (ou jusqu'à une espace) au début de `text`.
fn quoted_or_bare(text: &str) -> &str {
    let text = text.trim_start();
    match text.chars().next() {
        Some(quote @ ('"' | '\'')) => {
            let inner = &text[1..];
            &inner[..inner.find(quote).unwrap_or(inner.len())]
        }
        _ => {
            &text[..text
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '>'))
                .unwrap_or(text.len())]
        }
    }
}

/// `extractHtmlRedirect` : `<meta http-equiv="refresh" content="0; url=…">`, ou une affectation
/// de `location` en JavaScript.
fn page_redirect(html: &str, page: &Url) -> Option<Url> {
    let mut from = 0;
    while let Some(at) = find_ci(html, "<meta", from) {
        from = at + 1;
        let end = html[at..].find('>').map_or(html.len(), |e| at + e);
        let tag = &html[at..end];
        if find_ci(tag, "refresh", 0).is_none() {
            continue;
        }
        let Some(content) = find_ci(tag, "url=", 0) else {
            continue;
        };
        let target: String = quoted_or_bare(&tag[content + 4..])
            .chars()
            .filter(|c| !matches!(c, '"' | '\''))
            .collect();
        if !target.is_empty() {
            return page.join(&target).ok();
        }
    }
    for marker in ["location.replace", "location.href", "location"] {
        let mut from = 0;
        while let Some(at) = find_ci(html, marker, from) {
            from = at + 1;
            let rest = html[at + marker.len()..].trim_start();
            let rest = if marker == "location.replace" {
                match rest.strip_prefix('(') {
                    Some(rest) => rest.trim_start(),
                    None => continue,
                }
            } else {
                match rest.strip_prefix('=') {
                    Some(rest) if !rest.starts_with('=') => rest.trim_start(),
                    _ => continue,
                }
            };
            if !rest.starts_with(['"', '\'']) {
                continue;
            }
            let target = quoted_or_bare(rest);
            if !target.is_empty() {
                return page.join(target).ok();
            }
        }
    }
    None
}

// ── Connexion ───────────────────────────────────────────────────────────────

/// Pourquoi la connexion au webmail a échoué. Le texte ne contient jamais d'adresse complète, de
/// cookie ni de jeton.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignInError {
    /// Le CAS redemande le mot de passe : la session CAS est finie.
    CasOver,
    /// Un saut ou l'origine finale sort de la liste des domaines (son hôte, pour le journal).
    LeftDomains(String),
    /// Plus de vingt sauts.
    TooManyHops,
    /// Le délai total est dépassé.
    Deadline,
    /// Une page qu'on ne sait pas suivre (statut, hôte).
    Unexpected(u16, String),
    /// Réseau, délai d'un saut, réponse illisible.
    Network(String),
}

impl std::fmt::Display for SignInError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignInError::CasOver => f.write_str("CAS session over"),
            SignInError::LeftDomains(host) => {
                write!(f, "Webmail sign-in left the allowed domains ({host})")
            }
            SignInError::TooManyHops => {
                f.write_str("too many redirects while signing in to the webmail")
            }
            SignInError::Deadline => f.write_str("webmail sign-in took too long"),
            SignInError::Unexpected(status, host) => {
                write!(
                    f,
                    "webmail sign-in stopped on an unexpected page ({status} at {host})"
                )
            }
            SignInError::Network(why) => write!(f, "webmail sign-in: {why}"),
        }
    }
}

/// Ce qu'une connexion au webmail demande.
pub struct SignIn<'a> {
    /// L'entrée publique du webmail, d'où part la connexion.
    pub entry: &'a Url,
    /// Où la connexion peut passer.
    pub domains: &'a SignInDomains,
    /// Hôte du CAS de l'université.
    pub cas_host: &'a str,
    /// Cookies de la session de l'étudiant (le TGC) : copiés, jamais rendus.
    pub cookies: Vec<StoredCookie>,
    pub net: NetSettings,
    /// Certificats supplémentaires de l'université.
    pub extra_ca: Vec<PathBuf>,
}

fn host_of(url: &Url) -> String {
    url.host_str().unwrap_or_default().to_ascii_lowercase()
}

/// L'origine `https` d'une adresse (la dernière redirection de Partage est en `http`).
fn https_origin(url: &Url, https_only: bool) -> Option<Url> {
    let mut origin = Url::parse(&url.origin().ascii_serialization()).ok()?;
    if https_only || origin.scheme() == "https" {
        origin.set_scheme("https").ok()?;
    }
    Some(origin)
}

fn html_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static(ACCEPT_HTML));
    headers
}

/// Connexion au webmail. Une seule à la fois par session : à l'appelant de partager celle en
/// cours entre ses requêtes.
pub async fn sign_in(request: SignIn<'_>) -> Result<Webmail, SignInError> {
    let options = BrowserOptions {
        extra_ca: request.extra_ca,
        https_only: request.net.https_only,
        user_agent: request.net.user_agent,
        ..BrowserOptions::default()
    };
    let mut browser = Browser::new(&options, CookieJar::from_stored(request.cookies))
        .map_err(|e| SignInError::Network(e.to_string()))?;
    let walked = tokio::time::timeout(
        SIGN_IN_DEADLINE,
        walk(
            &mut browser,
            request.entry,
            request.domains,
            request.cas_host,
        ),
    )
    .await
    .map_err(|_| SignInError::Deadline)??;
    let (origin, csrf) = walked;
    Ok(Webmail {
        origin,
        csrf,
        jar: Mutex::new(browser.jar),
        options,
        created_at: Instant::now(),
    })
}

async fn walk(
    browser: &mut Browser,
    entry: &Url,
    domains: &SignInDomains,
    cas_host: &str,
) -> Result<(Url, Option<String>), SignInError> {
    let https_only = domains.https_only;
    let mut url = entry.clone();
    let mut method = Method::GET;
    let mut body: Option<String> = None;
    let mut headers = html_headers();
    let landed = |url: &Url, browser: &Browser| {
        let host = host_of(url);
        !host.is_empty() && browser.jar.contains(&host, TOKEN_COOKIE)
    };
    for _ in 0..MAX_HOPS {
        if !domains.allows(&url) {
            return Err(SignInError::LeftDomains(host_of(&url)));
        }
        let (status, response_headers, text) = browser
            .send(
                method.clone(),
                url.clone(),
                headers.clone(),
                body.clone(),
                Some(REQUEST_TIMEOUT),
            )
            .await
            .map_err(|e| SignInError::Network(network_words(&e)))?;
        let location = response_headers
            .get(header::LOCATION)
            .and_then(|l| l.to_str().ok());
        if status.is_redirection()
            && let Some(location) = location
        {
            let next = url
                .join(location)
                .map_err(|_| SignInError::Network("unreadable Location".to_owned()))?;
            // Arrivé sur le webmail avec son cookie : inutile de charger l'application. Son
            // origine, en https, doit être de la liste (W4).
            if landed(&next, browser) && host_of(&next) != cas_host {
                let origin = https_origin(&next, https_only)
                    .ok_or_else(|| SignInError::LeftDomains(host_of(&next)))?;
                if !domains.allows(&origin) {
                    return Err(SignInError::LeftDomains(host_of(&origin)));
                }
                return Ok((origin, None));
            }
            let to_get = status == reqwest::StatusCode::SEE_OTHER
                || (matches!(status.as_u16(), 301 | 302) && method == Method::POST);
            if to_get {
                method = Method::GET;
                body = None;
                headers = html_headers();
            }
            url = next;
            continue;
        }

        if landed(&url, browser) {
            let origin = https_origin(&url, https_only)
                .ok_or_else(|| SignInError::LeftDomains(host_of(&url)))?;
            if !domains.allows(&origin) {
                return Err(SignInError::LeftDomains(host_of(&origin)));
            }
            return Ok((origin, csrf_token(&text)));
        }
        if host_of(&url) == cas_host && is_cas_login_form(&text) {
            return Err(SignInError::CasOver);
        }
        if let Some((action, fields)) = auto_submit_form(&text, &url) {
            let referer = url.clone();
            let mut post = html_headers();
            post.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/x-www-form-urlencoded"),
            );
            if let Ok(origin) = HeaderValue::from_str(&referer.origin().ascii_serialization()) {
                post.insert(header::ORIGIN, origin);
            }
            if let Ok(value) = HeaderValue::from_str(referer.as_str()) {
                post.insert(header::REFERER, value);
            }
            body = Some(
                url::form_urlencoded::Serializer::new(String::new())
                    .extend_pairs(&fields)
                    .finish(),
            );
            method = Method::POST;
            headers = post;
            url = action;
            continue;
        }
        if let Some(next) = page_redirect(&text, &url) {
            url = next;
            method = Method::GET;
            body = None;
            headers = html_headers();
            continue;
        }
        return Err(SignInError::Unexpected(status.as_u16(), host_of(&url)));
    }
    Err(SignInError::TooManyHops)
}

/// Erreur réseau sans adresse.
fn network_words(error: &HttpError) -> String {
    match error {
        HttpError::Network(why) if why.contains("timed out") || why.contains("délai") => {
            "timeout".to_owned()
        }
        other => other.to_string(),
    }
}

// ── Session du webmail ──────────────────────────────────────────────────────

/// Une session ouverte sur le webmail : son origine, son jeton CSRF quand la page en donnait
/// un, ses cookies. Gardée en mémoire par l'appelant, jamais écrite ailleurs.
pub struct Webmail {
    origin: Url,
    csrf: Option<String>,
    jar: Mutex<CookieJar>,
    options: BrowserOptions,
    created_at: Instant,
}

impl std::fmt::Debug for Webmail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Ni cookie ni jeton.
        f.debug_struct("Webmail")
            .field("origin", &self.origin.as_str())
            .field("csrf", &self.csrf.is_some())
            .finish_non_exhaustive()
    }
}

/// Pourquoi une requête au webmail a échoué.
#[derive(Debug, Clone, Default)]
pub struct SoapError {
    /// Le webmail refuse la session : une nouvelle connexion peut aider, rien d'autre.
    pub refused: bool,
    /// Code de la faute Zimbra tel qu'il est reçu (`mail.SEND_ABORTED_ADDRESS_FAILURE`…), pour
    /// la réponse qui en dépend. Jamais journalisé tel quel : voir [`SoapError::code_for_log`].
    pub code: Option<String>,
    /// Arguments de la faute (`Detail.Error.a`) : une adresse refusée… Jamais journalisés.
    pub args: Vec<Value>,
    /// Statut HTTP, quand une réponse est arrivée.
    pub status: Option<u16>,
    /// Nom de la requête (`SearchRequest`…).
    pub request: String,
    /// Délai dépassé, réseau…, quand il n'y a pas eu de réponse.
    pub network: Option<String>,
}

impl SoapError {
    /// Le code, nettoyé pour un journal : lettres, chiffres, `.` et `_`, 64 au plus.
    #[must_use]
    pub fn code_for_log(&self) -> String {
        match &self.code {
            Some(code)
                if !code.is_empty()
                    && code.len() <= 64
                    && code
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') =>
            {
                code.clone()
            }
            Some(_) => "unknown".to_owned(),
            None => match (self.status, &self.network) {
                (Some(status), _) => format!("HTTP {status}"),
                (None, Some(why)) => why.clone(),
                (None, None) => "no answer".to_owned(),
            },
        }
    }

    /// La faute a ce code (`NO_SUCH_MSG` trouve `mail.NO_SUCH_MSG`).
    #[must_use]
    pub fn code_is(&self, suffix: &str) -> bool {
        self.code
            .as_deref()
            .is_some_and(|code| code == suffix || code.ends_with(&format!(".{suffix}")))
    }
}

impl std::fmt::Display for SoapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = if self.refused {
            "rejected the session"
        } else {
            "failed"
        };
        write!(
            f,
            "Zimbra {} {what} ({})",
            self.request,
            self.code_for_log()
        )
    }
}

/// `isZimbraSessionRefusal` : une faute `AUTH_REQUIRED` ou `AUTH_EXPIRED`, un 401, un 403, une
/// redirection, ou une page là où devait être le JSON de SOAP.
#[must_use]
pub fn is_session_refusal(code: &str, status: u16, json: bool) -> bool {
    if code.contains("AUTH_REQUIRED") || code.contains("AUTH_EXPIRED") {
        return true;
    }
    if status == 401 || status == 403 || (300..400).contains(&status) {
        return true;
    }
    (200..300).contains(&status) && !json
}

/// L'enveloppe SOAP d'une requête.
fn envelope(name: &str, namespace: &str, request: Value, csrf: Option<&str>) -> Value {
    let mut context = json!({ "_jsns": "urn:zimbra" });
    if let Some(token) = csrf {
        context["csrfToken"] = Value::String(token.to_owned());
    }
    let mut body = match request {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    body.insert("_jsns".to_owned(), Value::String(namespace.to_owned()));
    let mut payload = json!({ "Header": { "context": context }, "Body": {} });
    payload["Body"][name] = Value::Object(body);
    payload
}

/// Lecture d'une réponse SOAP : `Body.<Nom>Response` (ou `{}` s'il manque), ou l'échec.
fn read_soap_answer(name: &str, status: u16, text: &str) -> Result<Value, SoapError> {
    let payload: Option<Value> = serde_json::from_str(text).ok();
    let fault = payload.as_ref().and_then(|p| p.pointer("/Body/Fault"));
    let body = payload.as_ref().and_then(|p| p.get("Body"));
    if fault.is_some() || !(200..300).contains(&status) || body.is_none() {
        let code = fault
            .and_then(|f| f.pointer("/Detail/Error/Code"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let args = fault
            .and_then(|f| f.pointer("/Detail/Error/a"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let shown = code.clone().unwrap_or_else(|| format!("HTTP {status}"));
        return Err(SoapError {
            refused: is_session_refusal(&shown, status, payload.is_some()),
            code,
            args,
            status: Some(status),
            request: name.to_owned(),
            network: None,
        });
    }
    let answer = name.strip_suffix("Request").map(|n| format!("{n}Response"));
    Ok(answer
        .and_then(|answer| body.and_then(|b| b.get(&answer)).cloned())
        .unwrap_or_else(|| json!({})))
}

/// Pourquoi une pièce jointe n'a pas pu être lue.
#[derive(Debug)]
pub enum PartError {
    /// Le webmail refuse la session.
    Refused,
    /// Réseau, délai des en-têtes.
    Network(String),
}

impl Webmail {
    /// Une session sans cookie ni jeton, à `origin` : pour les tests des clients qui gardent une
    /// session du webmail (aucune requête ne la connecte).
    #[must_use]
    pub fn detached(origin: Url, net: NetSettings) -> Self {
        Webmail {
            origin,
            csrf: None,
            jar: Mutex::new(CookieJar::default()),
            options: BrowserOptions {
                https_only: net.https_only,
                user_agent: net.user_agent,
                ..BrowserOptions::default()
            },
            created_at: Instant::now(),
        }
    }

    /// Origine du webmail (`https://…`).
    #[must_use]
    pub fn origin(&self) -> &Url {
        &self.origin
    }

    /// La connexion a trouvé un jeton CSRF (pour le journal : jamais sa valeur).
    #[must_use]
    pub fn has_csrf(&self) -> bool {
        self.csrf.is_some()
    }

    /// Depuis quand la session est ouverte.
    #[must_use]
    pub fn age(&self) -> Duration {
        self.created_at.elapsed()
    }

    /// Un navigateur avec une copie des cookies de la session.
    fn browser(&self) -> Result<Browser, HttpError> {
        let jar = self.jar.lock().map(|jar| jar.clone()).unwrap_or_default();
        Browser::new(&self.options, jar)
    }

    /// Les cookies que le webmail a posés pendant la requête, gardés pour la suivante.
    fn keep(&self, browser: Browser) {
        if let Ok(mut jar) = self.jar.lock() {
            *jar = browser.jar;
        }
    }

    fn url(&self, path: &str) -> Option<Url> {
        self.origin.join(path).ok()
    }

    fn referer(&self) -> Option<HeaderValue> {
        HeaderValue::from_str(&format!("{}/", self.origin.as_str().trim_end_matches('/'))).ok()
    }

    /// Requête SOAP `name` (`SearchRequest`…) dans l'espace `namespace` (`urn:zimbraMail`,
    /// `urn:zimbraAccount`) : `Body.<Nom>Response`, ou `{}` quand il manque.
    pub async fn soap(
        &self,
        name: &str,
        namespace: &str,
        request: Value,
        timeout: Duration,
    ) -> Result<Value, SoapError> {
        let failed = |network: String| SoapError {
            request: name.to_owned(),
            network: Some(network),
            ..SoapError::default()
        };
        let url = self
            .url(&format!("/service/soap/{name}"))
            .ok_or_else(|| failed("unusable address".to_owned()))?;
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        if let Some(referer) = self.referer() {
            headers.insert(header::REFERER, referer);
        }
        if let Some(token) = self
            .csrf
            .as_deref()
            .and_then(|t| HeaderValue::from_str(t).ok())
        {
            headers.insert(HeaderName::from_static("x-zimbra-csrf-token"), token);
        }
        let body = envelope(name, namespace, request, self.csrf.as_deref()).to_string();
        let mut browser = self.browser().map_err(|e| failed(network_words(&e)))?;
        let sent = browser
            .send(Method::POST, url, headers, Some(body), Some(timeout))
            .await;
        self.keep(browser);
        let (status, _, text) = sent.map_err(|e| failed(network_words(&e)))?;
        read_soap_answer(name, status.as_u16(), &text)
    }

    /// `zimbraRestInbox` : la boîte de réception par l'API REST (pas de contrôle CSRF), `limit`
    /// messages au plus : la liste `m`.
    pub async fn rest_inbox(&self, limit: usize) -> Result<Vec<Value>, SoapError> {
        let name = "REST inbox";
        let failed = |network: String| SoapError {
            request: name.to_owned(),
            network: Some(network),
            ..SoapError::default()
        };
        let mut url = self
            .url("/service/home/~/inbox")
            .ok_or_else(|| failed("unusable address".to_owned()))?;
        url.query_pairs_mut()
            .append_pair("fmt", "json")
            .append_pair("limit", &limit.to_string());
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        if let Some(referer) = self.referer() {
            headers.insert(header::REFERER, referer);
        }
        let mut browser = self.browser().map_err(|e| failed(network_words(&e)))?;
        let sent = browser
            .send(Method::GET, url, headers, None, Some(REQUEST_TIMEOUT))
            .await;
        self.keep(browser);
        let (status, _, text) = sent.map_err(|e| failed(network_words(&e)))?;
        let status = status.as_u16();
        if status == 401 || status == 403 || (300..400).contains(&status) {
            return Err(SoapError {
                refused: true,
                status: Some(status),
                request: name.to_owned(),
                ..SoapError::default()
            });
        }
        let payload: Option<Value> = serde_json::from_str(&text).ok();
        match payload {
            Some(Value::Object(map)) if (200..300).contains(&status) => Ok(map
                .get("m")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()),
            _ => Err(SoapError {
                status: Some(status),
                request: name.to_owned(),
                ..SoapError::default()
            }),
        }
    }

    /// Une partie d'un message (`/service/home/~/?auth=co&id=…&part=…`), lue au fil de l'eau.
    /// `id` et `part` sont déjà vérifiés par l'appelant.
    pub async fn part(&self, id: &str, part: &str) -> Result<Streamed, PartError> {
        let mut url = self
            .url("/service/home/~/")
            .ok_or_else(|| PartError::Network("unusable address".to_owned()))?;
        url.query_pairs_mut()
            .append_pair("auth", "co")
            .append_pair("id", id)
            .append_pair("part", part);
        let mut headers = HeaderMap::new();
        if let Some(referer) = self.referer() {
            headers.insert(header::REFERER, referer);
        }
        let mut browser = self
            .browser()
            .map_err(|e| PartError::Network(network_words(&e)))?;
        let sent = browser
            .send_streamed(Method::GET, url, headers, None, REQUEST_TIMEOUT)
            .await;
        self.keep(browser);
        let streamed = sent.map_err(|e| PartError::Network(network_words(&e)))?;
        let status = streamed.status.as_u16();
        if status == 401 || status == 403 || (300..400).contains(&status) {
            return Err(PartError::Refused);
        }
        Ok(streamed)
    }

    /// Un fichier envoyé au servlet de Zimbra (`/service/upload?fmt=raw,extended`), au fil de
    /// l'eau, comme le client web de Zimbra l'envoie. `content_type` et `disposition` sont déjà
    /// nettoyés par l'appelant. → statut et texte de la réponse (64 Kio au plus).
    pub async fn upload(
        &self,
        body: RequestBody,
        length: u64,
        content_type: &str,
        disposition: &str,
    ) -> Result<(u16, String), PartError> {
        let url = self
            .url("/service/upload?fmt=raw,extended")
            .ok_or_else(|| PartError::Network("unusable address".to_owned()))?;
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("text/html,*/*"));
        let value = |text: &str| {
            HeaderValue::from_str(text).map_err(|_| PartError::Network("invalid header".to_owned()))
        };
        headers.insert(header::CONTENT_TYPE, value(content_type)?);
        headers.insert(header::CONTENT_DISPOSITION, value(disposition)?);
        headers.insert(header::CONTENT_LENGTH, value(&length.to_string())?);
        headers.insert(
            HeaderName::from_static("x-requested-with"),
            HeaderValue::from_static("XMLHttpRequest"),
        );
        if let Some(referer) = self.referer() {
            headers.insert(header::REFERER, referer);
        }
        if let Some(token) = self
            .csrf
            .as_deref()
            .and_then(|t| HeaderValue::from_str(t).ok())
        {
            headers.insert(HeaderName::from_static("x-zimbra-csrf-token"), token);
        }
        let mut browser = self
            .browser()
            .map_err(|e| PartError::Network(network_words(&e)))?;
        let sent = browser
            .send_streamed(Method::POST, url, headers, Some(body), REQUEST_TIMEOUT)
            .await;
        self.keep(browser);
        let streamed = sent.map_err(|e| PartError::Network(network_words(&e)))?;
        let status = streamed.status.as_u16();
        if status == 401 || status == 403 || (300..400).contains(&status) {
            return Err(PartError::Refused);
        }
        let text = tokio::time::timeout(REQUEST_TIMEOUT, streamed.text_up_to(64 * 1024))
            .await
            .map_err(|_| PartError::Network("timeout".to_owned()))?
            .map_err(|e| PartError::Network(network_words(&e)))?;
        Ok((status, text))
    }

    /// L'adresse qui ouvre le webmail déjà connecté dans le navigateur
    /// (`/service/preauth?authtoken=…&isredirect=1`). Elle contient le jeton : ne jamais la
    /// journaliser. `None` sans cookie de session.
    #[must_use]
    pub fn preauth_url(&self) -> Option<Url> {
        let jar = self.jar.lock().ok()?;
        let token = jar
            .value(&host_of(&self.origin), TOKEN_COOKIE, now())?
            .to_owned();
        drop(jar);
        let mut url = self.url("/service/preauth")?;
        url.query_pairs_mut()
            .append_pair("authtoken", &token)
            .append_pair("isredirect", "1");
        Some(url)
    }

    /// Fin de la session du webmail (`EndSessionRequest`, `logoff`) : Zimbra invalide le jeton.
    /// Au mieux, quatre secondes ; tout échec est ignoré.
    pub async fn end_session(&self) {
        let _ = self
            .soap(
                "EndSessionRequest",
                "urn:zimbraAccount",
                json!({ "logoff": "1" }),
                LOGOUT_TIMEOUT,
            )
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn rennes() -> SignInDomains {
        SignInDomains::new(
            [
                "partage.renater.fr",
                "univ-rennes1.fr",
                "univ-rennes.fr",
                "partage.univ-rennes.fr",
                "sso-cas.univ-rennes.fr",
            ],
            true,
        )
    }

    #[test]
    fn liste_des_domaines() {
        let domains = rennes();
        assert!(domains.allows(&url("https://partage.univ-rennes.fr/")));
        assert!(domains.allows(&url("https://idp.univ-rennes1.fr/idp/profile/SAML2")));
        assert!(domains.allows(&url("https://PARTAGE.RENATER.FR/Shibboleth.sso/SAML2/POST")));
        for refused in [
            "http://partage.univ-rennes.fr/",
            "https://evil-univ-rennes.fr/",
            "https://univ-rennes.fr.evil.example/",
            "https://renater.fr/",
            "https://user:pass@partage.univ-rennes.fr/",
            "ftp://partage.univ-rennes.fr/",
        ] {
            assert!(!domains.allows(&url(refused)), "{refused}");
        }
        // Les faux serveurs des tests sont en http.
        let tests = SignInDomains::new(["127.0.0.1.example"], false);
        assert!(tests.allows(&url("http://x.127.0.0.1.example/")));
        // Un domaine mal écrit n'entre pas dans la liste ; un nom sans point ne vaut que pour
        // lui-même.
        let odd = SignInDomains::new(["fr", " . ", "a b.fr", "localhost"], true);
        assert!(!odd.allows(&url("https://x.fr/")));
        assert!(!odd.allows(&url("https://a.localhost/")));
        assert!(odd.allows(&url("https://localhost/")));
    }

    #[test]
    fn recherche_sans_casse_aux_bords() {
        assert_eq!(find_ci("", "csrftoken", 0), None);
        assert_eq!(find_ci("abc", "abcd", 0), None);
        assert_eq!(find_ci("xABc", "abc", 0), Some(1));
        assert_eq!(find_ci("abc", "abc", 1), None);
        assert_eq!(find_ci("abc", "c", 5), None);
    }

    #[test]
    fn jeton_csrf() {
        assert_eq!(
            csrf_token("<script>window.csrfToken = \"0_abcdef12\";</script>").as_deref(),
            Some("0_abcdef12")
        );
        assert_eq!(
            csrf_token("{CSRFTOKEN:'abcdefgh'}").as_deref(),
            Some("abcdefgh")
        );
        assert_eq!(csrf_token("csrfToken = 'court'"), None);
        assert_eq!(csrf_token("pas de jeton"), None);
    }

    #[test]
    fn formulaire_soumis_seul() {
        let page = url("https://idp.univ-rennes1.fr/idp/profile/SAML2/Redirect/SSO");
        let html = r#"<html><body onload="document.forms[0].submit()">
            <form action="https&#x3a;//partage.renater.fr/Shibboleth.sso/SAML2/POST" method="post">
            <input type="hidden" name="RelayState" value="ss&#x3a;mem"/>
            <input type="hidden" name="SAMLResponse" value="PHNhbWw+"/>
            </form></body></html>"#;
        let (action, fields) = auto_submit_form(html, &page).unwrap();
        assert_eq!(
            action.as_str(),
            "https://partage.renater.fr/Shibboleth.sso/SAML2/POST"
        );
        assert!(
            fields
                .iter()
                .any(|(n, v)| n == "SAMLResponse" && v == "PHNhbWw+")
        );
        assert!(
            fields
                .iter()
                .any(|(n, v)| n == "RelayState" && v == "ss:mem")
        );
        // Sans `.submit()`, rien n'est envoyé.
        assert!(auto_submit_form(&html.replace(".submit()", ""), &page).is_none());
    }

    #[test]
    fn redirections_de_page() {
        let page = url("https://partage.univ-rennes.fr/a/b");
        let meta = r#"<meta http-equiv="refresh" content="0; url=/mail?x=1">"#;
        assert_eq!(
            page_redirect(meta, &page).unwrap().as_str(),
            "https://partage.univ-rennes.fr/mail?x=1"
        );
        let script =
            "<script>window.location.href = 'https://partage.univ-rennes.fr/zimbra/';</script>";
        assert_eq!(
            page_redirect(script, &page).unwrap().as_str(),
            "https://partage.univ-rennes.fr/zimbra/"
        );
        let replace = "<script>location.replace(\"/next\")</script>";
        assert_eq!(
            page_redirect(replace, &page).unwrap().as_str(),
            "https://partage.univ-rennes.fr/next"
        );
        assert!(page_redirect("<p>if (location == 'x')</p>", &page).is_none());
    }

    #[test]
    fn page_de_connexion_du_cas() {
        let form = r#"<form method="post"><input name="username"><input type="hidden" name="execution" value="e1s1"></form>"#;
        assert!(is_cas_login_form(form));
        assert!(!is_cas_login_form("<form><input name=\"username\"></form>"));
    }

    #[test]
    fn refus_de_session() {
        assert!(is_session_refusal("service.AUTH_REQUIRED", 500, true));
        assert!(is_session_refusal("service.AUTH_EXPIRED", 200, true));
        assert!(is_session_refusal("HTTP 401", 401, false));
        assert!(is_session_refusal("HTTP 302", 302, false));
        assert!(is_session_refusal("HTTP 200", 200, false));
        assert!(!is_session_refusal("mail.NO_SUCH_MSG", 500, true));
        assert!(!is_session_refusal("HTTP 503", 503, false));
        assert!(!is_session_refusal("HTTP 200", 200, true));
    }

    #[test]
    fn reponses_soap() {
        let ok = read_soap_answer(
            "SearchRequest",
            200,
            r#"{"Body":{"SearchResponse":{"c":[]}}}"#,
        )
        .unwrap();
        assert_eq!(ok, json!({"c": []}));
        // Réponse sans le nom attendu : {}.
        assert_eq!(
            read_soap_answer("NoOpRequest", 200, r#"{"Body":{}}"#).unwrap(),
            json!({})
        );
        let fault = read_soap_answer(
            "SendMsgRequest",
            500,
            r#"{"Body":{"Fault":{"Detail":{"Error":{"Code":"mail.SEND_ABORTED_ADDRESS_FAILURE","a":[{"n":"invalid","_content":"x@y.fr"}]}}}}}"#,
        )
        .unwrap_err();
        assert!(!fault.refused);
        assert!(fault.code_is("SEND_ABORTED_ADDRESS_FAILURE"));
        assert_eq!(fault.args.len(), 1);
        // Les arguments ne sont jamais écrits.
        assert!(!fault.to_string().contains("x@y.fr"));
        let refused = read_soap_answer("SearchRequest", 200, "<html>connexion</html>").unwrap_err();
        assert!(refused.refused);
        assert_eq!(
            refused.to_string(),
            "Zimbra SearchRequest rejected the session (HTTP 200)"
        );
        let auth = read_soap_answer(
            "SearchRequest",
            500,
            r#"{"Body":{"Fault":{"Detail":{"Error":{"Code":"service.AUTH_EXPIRED"}}}}}"#,
        )
        .unwrap_err();
        assert!(auth.refused);
        let weird = SoapError {
            code: Some("x@y <script>".to_owned()),
            ..SoapError::default()
        };
        assert_eq!(weird.code_for_log(), "unknown");
    }

    #[test]
    fn enveloppe_json() {
        let body = envelope(
            "SearchRequest",
            "urn:zimbraMail",
            json!({"query": "in:inbox\"<x>"}),
            Some("jeton"),
        );
        assert_eq!(body["Header"]["context"]["csrfToken"], "jeton");
        assert_eq!(body["Body"]["SearchRequest"]["_jsns"], "urn:zimbraMail");
        assert_eq!(body["Body"]["SearchRequest"]["query"], "in:inbox\"<x>");
        let plain = envelope("GetInfoRequest", "urn:zimbraAccount", json!({}), None);
        assert!(plain["Header"]["context"].get("csrfToken").is_none());
    }

    #[test]
    fn origine_https() {
        assert_eq!(
            https_origin(
                &url("http://partage.univ-rennes1.fr/service/preauth?x=1"),
                true
            )
            .unwrap()
            .as_str(),
            "https://partage.univ-rennes1.fr/"
        );
        assert_eq!(
            https_origin(&url("http://127.0.0.1:8080/x"), false)
                .unwrap()
                .as_str(),
            "http://127.0.0.1:8080/"
        );
    }
}
