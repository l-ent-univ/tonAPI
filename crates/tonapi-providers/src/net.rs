//! Accès réseau commun à tous les fournisseurs.
//!
//! Un fournisseur ne crée jamais son propre client HTTP : il ouvre un [`Net`] depuis le
//! [`Context`], fait ses requêtes, puis appelle [`Net::save`] pour que les cookies reçus (session
//! du service, TGC renouvelé) repartent dans le jeton de l'étudiant.
//!
//! Règles, vérifiées en revue :
//! - jamais de mot de passe ici : un service derrière le CAS s'ouvre avec le TGC de la session ;
//! - aucune donnée de l'étudiant (corps de réponse, URL avec ticket) dans un message d'erreur ;
//! - une réponse illisible donne `upstream_changed`, un serveur injoignable `service_unavailable`.

use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde_json::Value;
use tonapi_config::Sso;
use tonapi_core::{ErrorCode, Problem};
use tonapi_http::cas::{self, CasError, CasServer, Outcome};
use tonapi_http::{Browser, BrowserOptions, CookieJar, HttpError, Page};
use url::Url;

use crate::Context;

/// Réglages réseau d'une requête. `https_only` n'est désactivé que dans les tests (faux serveurs).
#[derive(Debug, Clone, Copy)]
pub struct NetSettings {
    pub https_only: bool,
}

impl Default for NetSettings {
    fn default() -> Self {
        NetSettings { https_only: true }
    }
}

/// Navigateur chargé des cookies de la session, et CAS de l'université.
pub(crate) struct Net {
    pub browser: Browser,
    pub cas: Option<CasServer>,
    provider: &'static str,
}

/// Erreur réseau → problème, sans détail sensible.
pub(crate) fn http_problem(provider: &str, error: &HttpError) -> Problem {
    Problem::new(ErrorCode::ServiceUnavailable).detail(format!("{provider} : {error}"))
}

/// Réponse inattendue : le format du service a probablement changé.
pub(crate) fn upstream(provider: &str, what: impl std::fmt::Display) -> Problem {
    Problem::new(ErrorCode::UpstreamChanged).detail(format!("{provider} : {what}"))
}

impl Net {
    pub fn open(ctx: &Context<'_>, provider: &'static str) -> Result<Self, Problem> {
        let establishment = ctx.session.establishment.as_ref().map(|e| e.id.as_str());
        let options = BrowserOptions {
            extra_ca: ctx.university.extra_ca(establishment),
            https_only: ctx.net.https_only,
            ..BrowserOptions::default()
        };
        let browser = Browser::new(
            &options,
            CookieJar::from_stored(ctx.session.cookies.clone()),
        )
        .map_err(|e| Problem::new(ErrorCode::Internal).detail(e.to_string()))?;
        let cas = match &ctx.university.university.sso {
            Sso::Cas(cas) => Some(CasServer::new(cas.base_url.clone(), cas.flavor)),
            Sso::Demo => None,
        };
        Ok(Net {
            browser,
            cas,
            provider,
        })
    }

    /// Remet les cookies dans la session de l'étudiant.
    pub fn save(self, ctx: &mut Context<'_>) {
        ctx.session.cookies = self.browser.jar.to_stored(ctx.now.timestamp());
    }

    /// Ouvre une page protégée par le CAS de l'université, sans mot de passe (TGC de la session).
    /// Si le CAS redemande le mot de passe ou un second facteur : `session_expired`.
    pub async fn open_protected(&mut self, url: Url) -> Result<Page, Problem> {
        let Some(cas) = self.cas.clone() else {
            return self
                .browser
                .get(url)
                .await
                .map_err(|e| http_problem(self.provider, &e));
        };
        match cas::open(&mut self.browser, &cas, url, None).await {
            Ok(Outcome::SignedIn { page, .. }) => Ok(page),
            Ok(Outcome::SecondFactor(_)) => Err(Problem::new(ErrorCode::SessionExpired)
                .detail("l'université redemande le second facteur : se reconnecter")),
            Err(CasError::NotAuthenticated) => Err(Problem::new(ErrorCode::SessionExpired)),
            Err(CasError::Http(e)) => Err(http_problem(self.provider, &e)),
            Err(e) => Err(upstream(self.provider, e)),
        }
    }

    /// Une requête sans suivre de redirection. Renvoie statut, en-têtes et corps.
    pub async fn request(
        &mut self,
        method: Method,
        url: Url,
        headers: HeaderMap,
        body: Option<String>,
    ) -> Result<(reqwest::StatusCode, HeaderMap, String), Problem> {
        self.browser
            .send(method, url, headers, body, None)
            .await
            .map_err(|e| http_problem(self.provider, &e))
    }

    /// `GET` d'un fichier : statut, en-têtes et corps brut.
    pub async fn download(
        &mut self,
        url: Url,
        headers: HeaderMap,
    ) -> Result<(reqwest::StatusCode, HeaderMap, Vec<u8>), Problem> {
        self.browser
            .send_bytes(Method::GET, url, headers, None, None)
            .await
            .map_err(|e| http_problem(self.provider, &e))
    }

    /// Requête dont la réponse doit être du JSON.
    pub async fn json(
        &mut self,
        method: Method,
        url: Url,
        mut headers: HeaderMap,
        body: Option<String>,
    ) -> Result<(reqwest::StatusCode, Value), Problem> {
        headers
            .entry(header::ACCEPT)
            .or_insert(HeaderValue::from_static("application/json"));
        let (status, _, text) = self.request(method, url, headers, body).await?;
        let value = serde_json::from_str(&text)
            .map_err(|_| upstream(self.provider, format!("réponse non JSON (HTTP {status})")))?;
        Ok((status, value))
    }
}
