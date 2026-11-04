//! Client HTTP qui se comporte comme un navigateur : cookies, redirections suivies à la main.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

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
}

impl Default for BrowserOptions {
    fn default() -> Self {
        BrowserOptions {
            extra_ca: Vec::new(),
            https_only: true,
            timeout: Duration::from_secs(20),
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

/// Au-delà de ce nombre d'hôtes suivis, ceux qui n'ont plus de requête en cours sont oubliés.
const MAX_HOSTS: usize = 10_000;

type HostMap = HashMap<String, Arc<tokio::sync::Semaphore>>;

static HOSTS: LazyLock<Mutex<HostMap>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static LONG_POLLS: LazyLock<Mutex<HostMap>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// File de places d'une requête.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lane {
    Normal,
    LongPoll,
}

impl Lane {
    fn capacity(self) -> usize {
        match self {
            Lane::Normal => PER_HOST,
            Lane::LongPoll => LONG_POLLS_PER_HOST,
        }
    }

    fn hosts(self) -> &'static Mutex<HostMap> {
        match self {
            Lane::Normal => &HOSTS,
            Lane::LongPoll => &LONG_POLLS,
        }
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
    match tokio::time::timeout(HOST_WAIT, semaphore.acquire_owned()).await {
        Ok(Ok(permit)) => Ok(permit),
        _ => Err(HttpError::Network(
            "trop de requêtes simultanées vers ce service".to_owned(),
        )),
    }
}

/// Navigateur minimal, un par étudiant et par requête.
pub struct Browser {
    client: reqwest::Client,
    pub jar: CookieJar,
    https_only: bool,
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
        let text = String::from_utf8_lossy(&bytes).into_owned();
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
        let text = String::from_utf8_lossy(&bytes).into_owned();
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
        if let Some(cookie) = self.jar.header_for(&url, Self::now()) {
            request = request.header(header::COOKIE, cookie);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        let mut response = request.send().await?;
        let status = response.status();
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
        Browser, BrowserOptions, HostMap, HttpError, LONG_POLLS_PER_HOST, Lane, MAX_BODY,
        MAX_HOSTS, PER_HOST, describe, forget_idle_hosts, host_permit,
    };
    use crate::cookies::CookieJar;

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

    /// Audit 2026-11-04 : les longues interrogations du push prenaient les places ordinaires de
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
