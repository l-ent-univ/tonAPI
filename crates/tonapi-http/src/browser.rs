//! Client HTTP qui se comporte comme un navigateur : cookies, redirections suivies à la main.

use std::path::PathBuf;
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

/// Navigateur minimal, un par étudiant et par requête.
pub struct Browser {
    client: reqwest::Client,
    pub jar: CookieJar,
    https_only: bool,
}

impl Browser {
    /// Le TLS passe par la bibliothèque du système (OpenSSL, Security.framework) : certains CAS
    /// n'acceptent que des suites TLS 1.2 sans échange de clé éphémère, que rustls refuse.
    pub fn new(options: &BrowserOptions, jar: CookieJar) -> Result<Self, HttpError> {
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
        let client = reqwest::Client::builder()
            .tls_backend_native()
            .tls_certs_merge(certs)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(USER_AGENT)
            .default_headers(headers)
            .timeout(options.timeout)
            .build()?;
        Ok(Browser {
            client,
            jar,
            https_only: options.https_only,
        })
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
        let url = self.secure(url)?;
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
        let response = request.send().await?;
        let status = response.status();
        let headers = response.headers().clone();
        for value in headers.get_all(header::SET_COOKIE) {
            if let Ok(value) = value.to_str() {
                self.jar.store(&url, value, Self::now());
            }
        }
        let bytes = response.bytes().await?;
        if bytes.len() > MAX_BODY {
            return Err(HttpError::TooLarge);
        }
        Ok((
            status,
            headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        ))
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
                .send(method.clone(), url.clone(), headers, body.clone(), None)
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
