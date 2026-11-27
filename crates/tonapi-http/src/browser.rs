//! Client HTTP qui se comporte comme un navigateur : cookies, redirections suivies à la main.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode};
use url::Url;

use crate::cookies::CookieJar;

/// User-Agent stable : certains services (Mon Dossier Web) refusent un UA inconnu.
pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 \
(KHTML, like Gecko) Version/18.0 Safari/605.1.15";

const ACCEPT_HTML: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";
const MAX_REDIRECTS: usize = 15;
const MAX_BODY: usize = 8 * 1024 * 1024;
/// Délai pour établir la connexion TCP et TLS ; le délai total reste `BrowserOptions::timeout`.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("réseau : {0}")]
    Network(String),
    #[error("plus de {MAX_REDIRECTS} redirections")]
    TooManyRedirects,
    #[error("URL refusée : {0}")]
    Refused(String),
    #[error("réponse trop volumineuse")]
    TooLarge,
    #[error("certificat supplémentaire illisible : {0}")]
    Certificate(String),
}

impl From<reqwest::Error> for HttpError {
    fn from(e: reqwest::Error) -> Self {
        // Pas d'URL dans le message : elle peut contenir un ticket.
        HttpError::Network(e.without_url().to_string())
    }
}

/// Réglages du client.
#[derive(Debug, Clone)]
pub struct BrowserOptions {
    /// Certificats PEM ajoutés aux autorités du système.
    pub extra_ca: Vec<PathBuf>,
    /// Refuse tout ce qui n'est pas `https://`. Désactivé seulement dans les tests.
    pub https_only: bool,
    pub timeout: Duration,
    /// `User-Agent` envoyé à la place de [`USER_AGENT`]. Pour un client qui reprend une session
    /// ouverte par un autre programme : certains CAS lient leur ticket au `User-Agent` qui s'est
    /// connecté, et l'effacent quand un autre le présente.
    pub user_agent: Option<&'static str>,
}

impl Default for BrowserOptions {
    fn default() -> Self {
        BrowserOptions {
            extra_ca: Vec::new(),
            https_only: true,
            timeout: Duration::from_secs(20),
            user_agent: None,
        }
    }
}

/// Page obtenue après les redirections.
#[derive(Debug, Clone)]
pub struct Page {
    /// URL finale.
    pub url: Url,
    pub status: StatusCode,
    pub content_type: Option<String>,
    pub body: String,
    /// URL de chaque saut, dans l'ordre.
    pub hops: Vec<Url>,
    /// Redirection vers un schéma non HTTP (ex. `moodlemobile://…`) qui a arrêté la chaîne.
    pub stopped_at: Option<String>,
}

/// Clients déjà construits, par certificats supplémentaires et délai.
type ClientKey = (Vec<PathBuf>, Duration);
static CLIENTS: LazyLock<Mutex<HashMap<ClientKey, reqwest::Client>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn shared_client(options: &BrowserOptions) -> Result<reqwest::Client, HttpError> {
    let mut paths = options.extra_ca.clone();
    paths.sort();
    let key = (paths, options.timeout);
    if let Some(client) = CLIENTS.lock().ok().and_then(|m| m.get(&key).cloned()) {
        return Ok(client);
    }
    // Construit hors du verrou ; une construction en double au démarrage est sans effet.
    let client = Browser::build_client(options)?;
    if let Ok(mut map) = CLIENTS.lock() {
        map.entry(key).or_insert_with(|| client.clone());
    }
    Ok(client)
}

/// Requêtes simultanées vers un même hôte, par processus : protège le service de l'université
/// quand beaucoup d'étudiants arrivent en même temps. Les suivantes attendent leur tour.
const PER_HOST: usize = 32;
/// Attente maximale d'une place ; au-delà, le service est considéré comme saturé.
const HOST_WAIT: Duration = Duration::from_secs(10);

/// Longues interrogations (socket Engine.IO du push esup-otp) simultanées vers un même hôte. Elles
/// restent ouvertes jusqu'à la réponse du téléphone : comptées à part, elles ne prennent pas les
/// places des requêtes ordinaires du même hôte.
const LONG_POLLS_PER_HOST: usize = 16;

/// Réponses lues et requêtes envoyées au fil de l'eau (pièces jointes de la messagerie)
/// simultanées vers un même hôte. Elles peuvent durer, au rythme de l'étudiant : comptées à
/// part, elles ne prennent jamais les places des requêtes ordinaires (W5 de la revue : 32 pièces
/// lues lentement bloquaient sinon la messagerie de tous).
const STREAMED_PER_HOST: usize = 16;

/// Au-delà de ce nombre d'hôtes suivis, ceux qui n'ont plus de requête en cours sont oubliés.
const MAX_HOSTS: usize = 10_000;

type HostMap = HashMap<String, Arc<tokio::sync::Semaphore>>;

static HOSTS: LazyLock<Mutex<HostMap>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static LONG_POLLS: LazyLock<Mutex<HostMap>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static STREAMS: LazyLock<Mutex<HostMap>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// File de places d'une requête.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lane {
    Normal,
    LongPoll,
    Streamed,
}

impl Lane {
    fn capacity(self) -> usize {
        match self {
            Lane::Normal => PER_HOST,
            Lane::LongPoll => LONG_POLLS_PER_HOST,
            Lane::Streamed => STREAMED_PER_HOST,
        }
    }

    fn hosts(self) -> &'static Mutex<HostMap> {
        match self {
            Lane::Normal => &HOSTS,
            Lane::LongPoll => &LONG_POLLS,
            Lane::Streamed => &STREAMS,
        }
    }

    /// Étiquette des métriques.
    fn label(self) -> &'static str {
        match self {
            Lane::Normal => "normal",
            Lane::LongPoll => "long_poll",
            Lane::Streamed => "streamed",
        }
    }
}

/// Hôte d'une URL pour les métriques : nom, et port s'il n'est pas celui du schéma.
fn host_label(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    }
}

/// Mesure d'un échange avec une université, enregistrée à la fin quelle que soit l'issue.
struct Exchange {
    host: String,
    started: Instant,
    /// Classe de statut ; `error` tant qu'aucune réponse n'est arrivée.
    outcome: &'static str,
}

impl Drop for Exchange {
    fn drop(&mut self) {
        let host = self.host.as_str();
        tonapi_metrics::inc(
            "tonapi_upstream_requests_total",
            &[("host", host), ("outcome", self.outcome)],
        );
        tonapi_metrics::observe(
            "tonapi_upstream_duration_seconds",
            &[("host", host)],
            self.started.elapsed().as_secs_f64(),
        );
    }
}

/// Retire les hôtes dont toutes les places sont libres et que personne n'attend.
fn forget_idle_hosts(map: &mut HostMap, capacity: usize) {
    if map.len() > MAX_HOSTS {
        map.retain(|_, s| Arc::strong_count(s) > 1 || s.available_permits() < capacity);
    }
}

async fn host_permit(
    url: &Url,
    lane: Lane,
) -> Result<tokio::sync::OwnedSemaphorePermit, HttpError> {
    let host = format!(
        "{}:{}",
        url.host_str().unwrap_or_default(),
        url.port_or_known_default().unwrap_or(0)
    );
    let capacity = lane.capacity();
    let semaphore = {
        let mut map = lane
            .hosts()
            .lock()
            .map_err(|_| HttpError::Network("verrou des hôtes".to_owned()))?;
        forget_idle_hosts(&mut map, capacity);
        map.entry(host)
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(capacity)))
            .clone()
    };
    let started = Instant::now();
    if let Ok(Ok(permit)) = tokio::time::timeout(HOST_WAIT, semaphore.acquire_owned()).await {
        tonapi_metrics::observe(
            "tonapi_upstream_wait_seconds",
            &[("lane", lane.label())],
            started.elapsed().as_secs_f64(),
        );
        Ok(permit)
    } else {
        tonapi_metrics::inc(
            "tonapi_upstream_saturated_total",
            &[("host", &host_label(url)), ("lane", lane.label())],
        );
        Err(HttpError::Network(
            "trop de requêtes simultanées vers ce service".to_owned(),
        ))
    }
}

/// Navigateur minimal, un par étudiant et par requête.
pub struct Browser {
    client: reqwest::Client,
    pub jar: CookieJar,
    https_only: bool,
    /// `User-Agent` de chaque requête, s'il n'est pas celui du client partagé.
    user_agent: Option<HeaderValue>,
}

impl Browser {
    /// Le client HTTP est partagé par tout le processus pour les mêmes réglages : ses connexions
    /// (TCP, TLS) sont réutilisées d'une requête et d'un étudiant à l'autre. Il ne garde aucun
    /// cookie : ceux de l'étudiant partent de `jar` dans chaque requête.
    pub fn new(options: &BrowserOptions, jar: CookieJar) -> Result<Self, HttpError> {
        Ok(Browser {
            client: shared_client(options)?,
            jar,
            https_only: options.https_only,
            user_agent: options
                .user_agent
                .and_then(|agent| HeaderValue::from_str(agent).ok()),
        })
    }

    /// Le TLS passe par la bibliothèque du système (OpenSSL, Security.framework) : certains CAS
    /// n'acceptent que des suites TLS 1.2 sans échange de clé éphémère, que rustls refuse.
    fn build_client(options: &BrowserOptions) -> Result<reqwest::Client, HttpError> {
        Ok(Self::client_builder(options)?.build()?)
    }

    fn client_builder(options: &BrowserOptions) -> Result<reqwest::ClientBuilder, HttpError> {
        let mut certs = Vec::new();
        for path in &options.extra_ca {
            let pem = std::fs::read(path)
                .map_err(|e| HttpError::Certificate(format!("{} : {e}", path.display())))?;
            certs.extend(
                reqwest::Certificate::from_pem_bundle(&pem)
                    .map_err(|e| HttpError::Certificate(format!("{} : {e}", path.display())))?,
            );
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT_LANGUAGE,
            HeaderValue::from_static("fr-FR,fr;q=0.9,en;q=0.5"),
        );
        Ok(reqwest::Client::builder()
            .tls_backend_native()
            .tls_certs_merge(certs)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(USER_AGENT)
            .default_headers(headers)
            .tls_version_min(reqwest::tls::Version::TLS_1_2)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(options.timeout))
    }

    fn now() -> i64 {
        chrono::Utc::now().timestamp()
    }

    /// Passe une URL en `https` (sauf en test), refuse les autres schémas.
    fn secure(&self, mut url: Url) -> Result<Url, HttpError> {
        match url.scheme() {
            "https" => Ok(url),
            "http" if !self.https_only => Ok(url),
            "http" => {
                // Certains services redirigent en clair (Partage) : on reste en https.
                url.set_scheme("https")
                    .map_err(|()| HttpError::Refused(url.scheme().to_owned()))?;
                Ok(url)
            }
            other => Err(HttpError::Refused(format!("schéma {other}"))),
        }
    }

    /// Une requête, sans suivre les redirections. Les cookies reçus sont enregistrés.
    pub async fn send(
        &mut self,
        method: Method,
        url: Url,
        headers: HeaderMap,
        body: Option<String>,
        timeout: Option<Duration>,
    ) -> Result<(StatusCode, HeaderMap, String), HttpError> {
        let trace_url = url.clone();
        let (status, headers, bytes) = self.send_bytes(method, url, headers, body, timeout).await?;
        let text = into_text(bytes);
        trace_exchange(&trace_url, status, &headers, &text);
        Ok((status, headers, text))
    }

    /// Comme [`Browser::send`], pour une longue interrogation (socket du push) : comptée dans une
    /// file à part, elle ne prive pas les autres requêtes du même hôte de leurs places.
    pub async fn send_long_poll(
        &mut self,
        method: Method,
        url: Url,
        headers: HeaderMap,
        timeout: Duration,
    ) -> Result<(StatusCode, HeaderMap, String), HttpError> {
        let trace_url = url.clone();
        let (status, headers, bytes) = self
            .send_in(Lane::LongPoll, method, url, headers, None, Some(timeout))
            .await?;
        let text = into_text(bytes);
        trace_exchange(&trace_url, status, &headers, &text);
        Ok((status, headers, text))
    }

    /// Comme [`Browser::send`], corps brut (fichier PDF…), jamais tracé.
    pub async fn send_bytes(
        &mut self,
        method: Method,
        url: Url,
        headers: HeaderMap,
        body: Option<String>,
        timeout: Option<Duration>,
    ) -> Result<(StatusCode, HeaderMap, Vec<u8>), HttpError> {
        self.send_in(Lane::Normal, method, url, headers, body, timeout)
            .await
    }

    async fn send_in(
        &mut self,
        lane: Lane,
        method: Method,
        url: Url,
        headers: HeaderMap,
        body: Option<String>,
        timeout: Option<Duration>,
    ) -> Result<(StatusCode, HeaderMap, Vec<u8>), HttpError> {
        let url = self.secure(url)?;
        // Tenu jusqu'à la fin de la lecture du corps.
        let _permit = host_permit(&url, lane).await?;
        let mut request = self.client.request(method, url.clone()).headers(headers);
        if let Some(agent) = &self.user_agent {
            request = request.header(header::USER_AGENT, agent.clone());
        }
        if let Some(cookie) = self.jar.header_for(&url, Self::now()) {
            request = request.header(header::COOKIE, cookie);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        let mut exchange = Exchange {
            host: host_label(&url),
            started: Instant::now(),
            outcome: "error",
        };
        let mut response = request.send().await?;
        let status = response.status();
        exchange.outcome = tonapi_metrics::status_class(status.as_u16());
        let headers = response.headers().clone();
        for value in headers.get_all(header::SET_COOKIE) {
            if let Ok(value) = value.to_str() {
                self.jar.store(&url, value, Self::now());
            }
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BODY as u64)
        {
            return Err(HttpError::TooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > MAX_BODY {
                return Err(HttpError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((status, headers, bytes))
    }

    /// Une requête dont la réponse se lit au fil de l'eau ([`Streamed::chunk`]), sans suivre les
    /// redirections : une pièce jointe, jamais gardée entière en mémoire. `body` part lui aussi au
    /// fil de l'eau (un fichier envoyé). `head` borne l'attente des en-têtes ([`HeadWait`]) ; la
    /// lecture du corps n'a pas de délai total, à l'appelant de borner l'attente entre deux
    /// morceaux. Les cookies reçus sont enregistrés. La place de l'hôte, prise dans une file à part
    /// des requêtes ordinaires, est tenue jusqu'à l'abandon de la réponse.
    pub async fn send_streamed(
        &mut self,
        method: Method,
        url: Url,
        headers: HeaderMap,
        body: Option<reqwest::Body>,
        head: HeadWait,
    ) -> Result<Streamed, HttpError> {
        let url = self.secure(url)?;
        let permit = host_permit(&url, Lane::Streamed).await?;
        // Le délai du client (total) remplacé : seul celui des en-têtes compte ici.
        let mut request = self
            .client
            .request(method, url.clone())
            .headers(headers)
            .timeout(STREAMED_CEILING);
        if let Some(agent) = &self.user_agent {
            request = request.header(header::USER_AGENT, agent.clone());
        }
        if let Some(cookie) = self.jar.header_for(&url, Self::now()) {
            request = request.header(header::COOKIE, cookie);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        let exchange = Exchange {
            host: host_label(&url),
            started: Instant::now(),
            outcome: "error",
        };
        let timed_out = || HttpError::Network("délai dépassé".to_owned());
        let response = match head {
            HeadWait::FromNow(wait) => tokio::time::timeout(wait, request.send())
                .await
                .map_err(|_| timed_out())??,
            HeadWait::AfterBody(sent, wait) => {
                let sending = request.send();
                tokio::pin!(sending);
                tokio::select! {
                    answered = &mut sending => answered?,
                    () = async {
                        sent.wait().await;
                        tokio::time::sleep(wait).await;
                    } => return Err(timed_out()),
                }
            }
        };
        let status = response.status();
        let headers = response.headers().clone();
        for value in headers.get_all(header::SET_COOKIE) {
            if let Ok(value) = value.to_str() {
                self.jar.store(&url, value, Self::now());
            }
        }
        let mut exchange = exchange;
        exchange.outcome = tonapi_metrics::status_class(status.as_u16());
        Ok(Streamed {
            status,
            headers,
            response,
            _permit: permit,
            _exchange: exchange,
        })
    }

    /// `GET` en suivant les redirections.
    pub async fn get(&mut self, url: Url) -> Result<Page, HttpError> {
        self.navigate(Method::GET, url, None, None).await
    }

    /// `POST` d'un formulaire, puis redirections.
    pub async fn post_form(
        &mut self,
        url: Url,
        fields: &[(String, String)],
        referer: &Url,
    ) -> Result<Page, HttpError> {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields)
            .finish();
        self.navigate(Method::POST, url, Some(body), Some(referer))
            .await
    }

    async fn navigate(
        &mut self,
        mut method: Method,
        mut url: Url,
        mut body: Option<String>,
        referer: Option<&Url>,
    ) -> Result<Page, HttpError> {
        let mut hops = Vec::new();
        for _ in 0..=MAX_REDIRECTS {
            url = self.secure(url)?;
            let mut headers = HeaderMap::new();
            headers.insert(header::ACCEPT, HeaderValue::from_static(ACCEPT_HTML));
            if body.is_some() {
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/x-www-form-urlencoded"),
                );
                if let Ok(origin) = HeaderValue::from_str(&url.origin().ascii_serialization()) {
                    headers.insert(header::ORIGIN, origin);
                }
                if let Some(referer) = referer.and_then(|r| HeaderValue::from_str(r.as_str()).ok())
                {
                    headers.insert(header::REFERER, referer);
                }
            }
            let (status, response_headers, text) = self
                // Le corps (identifiants) ne part qu'au premier saut : jamais recopié.
                .send(method.clone(), url.clone(), headers, body.take(), None)
                .await?;
            hops.push(url.clone());

            let location = response_headers
                .get(header::LOCATION)
                .and_then(|l| l.to_str().ok());
            let Some(location) = location.filter(|_| status.is_redirection()) else {
                let content_type = response_headers
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                return Ok(Page {
                    url,
                    status,
                    content_type,
                    body: text,
                    hops,
                    stopped_at: None,
                });
            };
            let next = url
                .join(location)
                .map_err(|_| HttpError::Refused("Location illisible".to_owned()))?;
            if !matches!(next.scheme(), "http" | "https") {
                return Ok(Page {
                    url,
                    status,
                    content_type: None,
                    body: text,
                    hops,
                    stopped_at: Some(location.to_owned()),
                });
            }
            let keeps_method = matches!(
                status,
                StatusCode::TEMPORARY_REDIRECT | StatusCode::PERMANENT_REDIRECT
            );
            if keeps_method && method == Method::POST {
                // Rejouer un POST d'identifiants vers une autre adresse : jamais.
                return Err(HttpError::Refused("307/308 après un POST".to_owned()));
            }
            method = Method::GET;
            body = None;
            url = next;
        }
        Err(HttpError::TooManyRedirects)
    }
}

/// L'attente des en-têtes d'une requête au fil de l'eau ([`Browser::send_streamed`]).
pub enum HeadWait {
    /// Comptée dès l'envoi : une lecture, ou un corps déjà entier.
    FromNow(Duration),
    /// Comptée une fois le corps parti en entier ([`BodySent::done`]) : un fichier envoyé au
    /// rythme de l'étudiant, que le service ne lit qu'avant de répondre. Pendant l'envoi, à
    /// l'appelant de borner l'attente entre deux morceaux.
    AfterBody(BodySent, Duration),
}

/// Le signal « le corps de la requête est parti en entier », donné par qui le produit.
#[derive(Clone, Default)]
pub struct BodySent(Arc<tokio::sync::Notify>);

impl BodySent {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Le dernier octet est parti (ou le corps s'est arrêté).
    pub fn done(&self) {
        self.0.notify_one();
    }

    async fn wait(&self) {
        self.0.notified().await;
    }
}

/// Plafond d'une requête au fil de l'eau : l'appelant borne l'attente entre deux morceaux, ceci
/// n'est qu'un garde-fou contre une réponse sans fin.
const STREAMED_CEILING: Duration = Duration::from_secs(60 * 60);

/// Réponse lue au fil de l'eau ([`Browser::send_streamed`]).
pub struct Streamed {
    pub status: StatusCode,
    pub headers: HeaderMap,
    response: reqwest::Response,
    _permit: tokio::sync::OwnedSemaphorePermit,
    _exchange: Exchange,
}

impl Streamed {
    /// Longueur annoncée du corps tel qu'il sera lu : absente quand le corps est décompressé en
    /// route (la longueur annoncée serait celle du corps compressé).
    #[must_use]
    pub fn content_length(&self) -> Option<u64> {
        self.response.content_length()
    }

    /// Morceau suivant du corps ; `None` à la fin.
    pub async fn chunk(&mut self) -> Result<Option<bytes::Bytes>, HttpError> {
        Ok(self.response.chunk().await?)
    }

    /// Le corps entier, `max` octets au plus (le reste est abandonné).
    pub async fn text_up_to(mut self, max: usize) -> Result<String, HttpError> {
        let mut bytes = Vec::new();
        while let Some(chunk) = self.chunk().await? {
            let room = max.saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
            if bytes.len() >= max {
                break;
            }
        }
        Ok(into_text(bytes))
    }
}

/// Corps en texte. Un corps UTF-8 valide (le cas courant) est réutilisé sans copie ; sinon les
/// octets invalides sont remplacés par U+FFFD, comme `String::from_utf8_lossy`.
fn into_text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// Trace de diagnostic (`RUST_LOG=tonapi_http=debug`) : hôte, chemin, statut, cible d'une
/// redirection et noms des cookies posés. Jamais de valeur de cookie, de paramètre ni de corps.
fn trace_exchange(url: &Url, status: StatusCode, headers: &HeaderMap, body: &str) {
    if !tracing::enabled!(tracing::Level::DEBUG) {
        return;
    }
    let location = headers
        .get(header::LOCATION)
        .and_then(|l| l.to_str().ok())
        .and_then(|l| url.join(l).ok())
        .map(|next| format!(" → {}", describe(&next)))
        .unwrap_or_default();
    let cookies: Vec<&str> = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split('=').next())
        .map(str::trim)
        .collect();
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .unwrap_or("-");
    let title = page_title(body)
        .map(|t| format!(" titre={t:?}"))
        .unwrap_or_default();
    tracing::debug!(
        "{} {}{location} [{content_type}] cookies={cookies:?}{title}",
        status.as_u16(),
        describe(url),
    );
}

/// `<title>` d'une page, court et sans adresse ni numéro.
fn page_title(body: &str) -> Option<String> {
    let lower = body.get(..body.len().min(64 * 1024))?.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = start + lower[start..].find('>')? + 1;
    let close = open_end + lower[open_end..].find("</title>")?;
    let raw = body.get(open_end..close)?;
    let words: Vec<&str> = raw
        .split_whitespace()
        .map(|w| {
            let digits = w.chars().filter(char::is_ascii_digit).count();
            if w.contains('@') || digits >= 6 {
                "<…>"
            } else {
                w
            }
        })
        .collect();
    let title: String = words.join(" ").chars().take(100).collect();
    (!title.is_empty()).then_some(title)
}

/// Adresse sans donnée personnelle : les segments qui ressemblent à un identifiant sont masqués
/// et la requête est réduite aux noms de ses paramètres.
#[must_use]
pub fn describe(url: &Url) -> String {
    let segments: Vec<String> = url
        .path()
        .split('/')
        .map(|segment| {
            let digits = segment.chars().filter(char::is_ascii_digit).count();
            if segment.contains('@')
                || segment.contains("%40")
                || segment.len() >= 24
                || digits >= 6
            {
                "<…>".to_owned()
            } else {
                segment.to_owned()
            }
        })
        .collect();
    let names: Vec<String> = url
        .query_pairs()
        .map(|(name, _)| name.into_owned())
        .collect();
    let query = if names.is_empty() {
        String::new()
    } else {
        format!("?{}", names.join("&"))
    };
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    format!(
        "{}{port}{}{query}",
        url.host_str().unwrap_or("?"),
        segments.join("/")
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use reqwest::Method;
    use reqwest::header::HeaderMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use url::Url;

    use std::sync::Arc;

    use super::{
        BodySent, Browser, BrowserOptions, HeadWait, HostMap, HttpError, LONG_POLLS_PER_HOST, Lane,
        MAX_BODY, MAX_HOSTS, PER_HOST, STREAMED_PER_HOST, describe, forget_idle_hosts, host_permit,
    };
    use crate::cookies::CookieJar;

    /// Faux serveur qui répond par le `User-Agent` reçu.
    async fn agent_echo() -> Url {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut request = Vec::new();
                let mut buffer = [0_u8; 1024];
                while !request.ends_with(b"\r\n\r\n") {
                    let read = stream.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                }
                let head = String::from_utf8_lossy(&request).to_string();
                let agent = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("user-agent")
                            .then(|| value.trim().to_owned())
                    })
                    .unwrap_or_default();
                let answer = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{agent}",
                    agent.len()
                );
                stream.write_all(answer.as_bytes()).await.unwrap();
            }
        });
        url
    }

    #[tokio::test]
    async fn le_user_agent_est_celui_de_tonapi_ou_celui_que_le_client_donne() {
        let url = agent_echo().await;
        let ask = async |user_agent: Option<&'static str>| {
            let options = BrowserOptions {
                https_only: false,
                user_agent,
                ..BrowserOptions::default()
            };
            let mut browser = Browser::new(&options, CookieJar::default()).unwrap();
            browser.get(url.clone()).await.unwrap().body
        };
        assert_eq!(ask(None).await, super::USER_AGENT);
        // Un client qui reprend la session d'un autre programme se présente comme lui.
        assert_eq!(ask(Some("node")).await, "node");
        // Le réglage ne déborde pas sur le navigateur suivant, qui partage le même client HTTP.
        assert_eq!(ask(None).await, super::USER_AGENT);
    }

    /// Faux serveur HTTP/1.1 brut : une seule réponse, écrite par `respond` après lecture de la requête.
    async fn raw_server<F, Fut>(respond: F) -> Url
    where
        F: FnOnce(TcpStream) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            while !request.ends_with(b"\r\n\r\n") {
                let read = stream.read(&mut buffer).await.unwrap();
                if read == 0 {
                    return;
                }
                request.extend_from_slice(&buffer[..read]);
            }
            respond(stream).await;
        });
        url
    }

    async fn fetch(url: Url) -> Result<usize, HttpError> {
        let options = BrowserOptions {
            https_only: false,
            timeout: Duration::from_secs(10),
            ..BrowserOptions::default()
        };
        let mut browser = Browser::new(&options, CookieJar::default())?;
        let (_, _, bytes) = tokio::time::timeout(
            Duration::from_secs(5),
            browser.send_bytes(Method::GET, url, HeaderMap::new(), None, None),
        )
        .await
        .expect("la lecture aurait dû s'arrêter")?;
        Ok(bytes.len())
    }

    #[tokio::test]
    async fn un_content_length_trop_grand_est_refuse_sans_lire_le_corps() {
        let url = raw_server(|mut stream| async move {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                MAX_BODY + 1
            );
            stream.write_all(head.as_bytes()).await.unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
        })
        .await;
        assert!(matches!(fetch(url).await, Err(HttpError::TooLarge)));
    }

    #[tokio::test]
    async fn un_corps_sans_taille_est_coupe_au_plafond() {
        let url = raw_server(|mut stream| async move {
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            let chunk = vec![b'a'; 0x10000];
            while stream.write_all(b"10000\r\n").await.is_ok()
                && stream.write_all(&chunk).await.is_ok()
                && stream.write_all(b"\r\n").await.is_ok()
            {}
        })
        .await;
        assert!(matches!(fetch(url).await, Err(HttpError::TooLarge)));
    }

    #[tokio::test]
    async fn un_corps_sous_le_plafond_est_lu_en_entier() {
        let url = raw_server(|mut stream| async move {
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nbonjour")
                .await
                .unwrap();
        })
        .await;
        assert_eq!(fetch(url).await.unwrap(), 7);
    }

    #[test]
    fn les_hotes_inactifs_sont_oublies_au_dela_du_plafond() {
        let mut map = HostMap::new();
        let new = || Arc::new(tokio::sync::Semaphore::new(PER_HOST));
        for i in 0..MAX_HOSTS {
            map.insert(format!("h{i}:443"), new());
        }
        let occupe = new();
        let _permit = occupe.clone().try_acquire_owned().unwrap();
        map.insert("occupe:443".to_owned(), occupe);
        let attendu = new();
        let _attente = attendu.clone();
        map.insert("attendu:443".to_owned(), attendu);

        forget_idle_hosts(&mut map, PER_HOST);
        let mut restants: Vec<_> = map.keys().cloned().collect();
        restants.sort();
        assert_eq!(restants, ["attendu:443", "occupe:443"]);

        // Sous le plafond, rien n'est retiré.
        map.insert("libre:443".to_owned(), new());
        forget_idle_hosts(&mut map, PER_HOST);
        assert_eq!(map.len(), 3);
    }

    /// Les longues interrogations du push prenaient les places ordinaires de
    /// l'hôte OTP. Elles ont leur propre file.
    #[tokio::test]
    async fn les_longues_interrogations_ont_leur_file() {
        let url = Url::parse("https://otp-file.exemple.test/sockets/").unwrap();
        let mut held = Vec::new();
        for _ in 0..LONG_POLLS_PER_HOST {
            held.push(host_permit(&url, Lane::LongPoll).await.unwrap());
        }
        // File des longues interrogations pleine : la requête ordinaire passe tout de suite.
        let normal =
            tokio::time::timeout(Duration::from_millis(100), host_permit(&url, Lane::Normal)).await;
        assert!(matches!(normal, Ok(Ok(_))));
        let extra = tokio::time::timeout(
            Duration::from_millis(100),
            host_permit(&url, Lane::LongPoll),
        )
        .await;
        assert!(
            extra.is_err(),
            "la file des longues interrogations est bornée"
        );
    }

    /// Les pièces jointes lues lentement prenaient les places ordinaires de l'hôte du webmail :
    /// 32 suffisaient à bloquer la messagerie de tous. Elles ont leur propre file.
    #[tokio::test]
    async fn les_transferts_au_fil_de_l_eau_ont_leur_file() {
        let url = Url::parse("https://webmail-file.exemple.test/service/home/~/").unwrap();
        let mut held = Vec::new();
        for _ in 0..STREAMED_PER_HOST {
            held.push(host_permit(&url, Lane::Streamed).await.unwrap());
        }
        let normal =
            tokio::time::timeout(Duration::from_millis(100), host_permit(&url, Lane::Normal)).await;
        assert!(matches!(normal, Ok(Ok(_))));
        let extra = tokio::time::timeout(
            Duration::from_millis(100),
            host_permit(&url, Lane::Streamed),
        )
        .await;
        assert!(extra.is_err(), "la file des transferts est bornée");
    }

    /// Faux servlet d'envoi : lit tout le corps (`length` octets) avant de répondre.
    async fn upload_servlet(length: usize) -> Url {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}/upload", listener.local_addr().unwrap())).unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut seen = Vec::new();
                    let mut buffer = [0_u8; 1024];
                    let body_start = loop {
                        let read = stream.read(&mut buffer).await.unwrap_or(0);
                        if read == 0 {
                            return;
                        }
                        seen.extend_from_slice(&buffer[..read]);
                        if let Some(at) = seen.windows(4).position(|w| w == b"\r\n\r\n") {
                            break at + 4;
                        }
                    };
                    let mut body = seen.len() - body_start;
                    while body < length {
                        let read = stream.read(&mut buffer).await.unwrap_or(0);
                        if read == 0 {
                            return;
                        }
                        body += read;
                    }
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
                        )
                        .await;
                });
            }
        });
        url
    }

    /// Un fichier envoyé en trois morceaux, à 400 ms d'écart.
    async fn slow_upload(
        url: Url,
        head: HeadWait,
        sent: Option<BodySent>,
    ) -> Result<u16, HttpError> {
        use futures_util::StreamExt;
        let chunks = futures_util::stream::iter(0..3).then(move |index| {
            let sent = sent.clone();
            async move {
                if index > 0 {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                }
                if index == 2
                    && let Some(sent) = &sent
                {
                    sent.done();
                }
                Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"abcd"))
            }
        });
        let options = BrowserOptions {
            https_only: false,
            ..BrowserOptions::default()
        };
        let mut browser = Browser::new(&options, CookieJar::default())?;
        let mut headers = HeaderMap::new();
        headers.insert(reqwest::header::CONTENT_LENGTH, "12".parse().unwrap());
        let streamed = browser
            .send_streamed(
                Method::POST,
                url,
                headers,
                Some(reqwest::Body::wrap_stream(chunks)),
                head,
            )
            .await?;
        Ok(streamed.status.as_u16())
    }

    /// Le servlet ne répond qu'une fois le fichier lu : compter l'attente des en-têtes dès
    /// l'envoi faisait échouer tout envoi plus long qu'elle. Comptée du dernier octet, un envoi
    /// lent aboutit.
    #[tokio::test]
    async fn l_attente_d_un_envoi_compte_du_dernier_octet() {
        let url = upload_servlet(12).await;
        let wait = Duration::from_millis(500);
        let sent = BodySent::new();
        let after = slow_upload(
            url.clone(),
            HeadWait::AfterBody(sent.clone(), wait),
            Some(sent),
        )
        .await;
        assert_eq!(after.unwrap(), 200);
        let from_now = slow_upload(url, HeadWait::FromNow(wait), None).await;
        assert!(
            matches!(from_now, Err(HttpError::Network(_))),
            "{from_now:?}"
        );
    }

    /// Banc (ignoré) : ancien `from_utf8_lossy().into_owned()` contre `into_text`, sur des corps
    /// UTF-8 valides de 200 Ko (page ordinaire) et de 8 Mo (plafond).
    #[test]
    #[ignore = "banc : mesure, pas de vérification"]
    fn banc_decodage_du_corps() {
        for taille in [200 * 1024, MAX_BODY] {
            let corps = "<p>é accentué, texte ordinaire</p>\n"
                .repeat(taille / 38)
                .into_bytes();
            let (mut ancien, mut nouveau) = (Vec::new(), Vec::new());
            for _ in 0..101 {
                let copie = corps.clone();
                let t = std::time::Instant::now();
                let a = String::from_utf8_lossy(&copie).into_owned();
                ancien.push(t.elapsed().as_secs_f64() * 1e6);
                let t = std::time::Instant::now();
                let n = super::into_text(copie);
                nouveau.push(t.elapsed().as_secs_f64() * 1e6);
                assert_eq!(a, n);
            }
            let stat = |v: &mut Vec<f64>| {
                v.sort_by(f64::total_cmp);
                (v[50], v[95])
            };
            println!(
                "{} octets : ancien (médiane, p95 µs) = {:?} ; into_text = {:?}",
                corps.len(),
                stat(&mut ancien),
                stat(&mut nouveau)
            );
        }
    }

    #[test]
    fn into_text_donne_exactement_le_resultat_de_from_utf8_lossy() {
        let cas: [&[u8]; 7] = [
            b"",
            b"ascii",
            "é accentué — 日本語 🎓".as_bytes(),
            b"\xff\xfe invalide",
            b"coupe au milieu \xc3",
            b"\xed\xa0\x80 surrogate",
            b"ok \xf0\x9f\x8e fin",
        ];
        for octets in cas {
            assert_eq!(
                super::into_text(octets.to_vec()),
                String::from_utf8_lossy(octets).into_owned(),
                "{octets:?}"
            );
        }
    }

    #[test]
    fn le_client_exige_tls_1_2_et_borne_la_connexion() {
        let builder = Browser::client_builder(&BrowserOptions::default()).unwrap();
        let debug = format!("{builder:?}");
        assert!(
            debug.contains("tls_version_min: Version(Tls1_2)"),
            "{debug}"
        );
        assert!(debug.contains("connect_timeout: 5s"), "{debug}");
    }

    #[test]
    fn une_adresse_decrite_ne_garde_ni_valeur_ni_identifiant() {
        let url = Url::parse(
            "https://ent.example.test/uPortal/api/people/camille.demo@etu.example.test.json?ticket=ST-123-secret&service=x",
        )
        .unwrap();
        let text = describe(&url);
        assert_eq!(
            text,
            "ent.example.test/uPortal/api/people/<…>?ticket&service"
        );
        let feed = Url::parse("https://ade.example.test/jsp/custom/modules/plannings/direct_cal.jsp?data=0123456789abcdef0123456789abcdef,1").unwrap();
        assert!(!describe(&feed).contains("0123"));
        let id = Url::parse("https://scodoc.example.test/etudiant/22000123/releve").unwrap();
        assert_eq!(describe(&id), "scodoc.example.test/etudiant/<…>/releve");
    }

    #[test]
    fn le_titre_d_une_page_est_lu_sans_adresse() {
        let page = "<html><head><TITLE>\n  Bonjour camille.demo@example.test  </TITLE></head>";
        assert_eq!(super::page_title(page).as_deref(), Some("Bonjour <…>"));
        assert_eq!(super::page_title("{}"), None);
    }
}
