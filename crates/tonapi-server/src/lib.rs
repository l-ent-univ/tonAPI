//! Serveur HTTP de tonAPI : routes `/v1`, spécification OpenAPI et documentation Scalar.

mod access;
mod auth;
mod error;
mod guard;
mod limit;
mod reload;
mod respond;
mod routes;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use arc_swap::ArcSwap;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tonapi_config::Registry;
use tonapi_session::SessionKeys;
use tower_http::cors::{Any, CorsLayer};
use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::{Scalar, Servable};

pub use limit::{AUTH_PER_IP as DEFAULT_AUTH_PER_IP, Cidr};
pub use reload::watch;

/// En-tête qui porte un jeton mis à jour (composante changée, cookies renouvelés…).
pub const SESSION_HEADER: &str = "tonapi-session";

/// Réglages du serveur.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Dossier des universités.
    pub universities_dir: PathBuf,
    /// Durée de vie d'une session, en secondes.
    pub session_ttl_secs: i64,
    /// Répartiteurs de confiance : seules leurs connexions font lire `X-Forwarded-For`.
    pub trusted_proxies: Vec<Cidr>,
    /// Sert `/docs` et `/openapi.json`.
    pub docs: bool,
    /// Demandes de connexion par adresse IP et par minute.
    pub auth_per_ip: u32,
    /// `GET /v1/universities` et `GET /v1/universities/{id}` portent `Cache-Control: public,
    /// max-age=300`. Faux en dev : la configuration y est rechargée à chaud et doit se voir tout
    /// de suite.
    pub cache_catalog: bool,
}

/// État partagé par toutes les requêtes.
#[derive(Clone)]
pub struct AppState {
    /// Universités, remplacées d'un bloc au rechargement.
    pub registry: Arc<ArcSwap<Registry>>,
    pub keys: Arc<SessionKeys>,
    pub settings: Arc<Settings>,
    login_limiter: Arc<limit::LoginLimiter>,
    ip_limiter: Arc<limit::IpLimiter>,
    account_slots: Arc<limit::AccountSlots>,
    draining: Arc<AtomicBool>,
}

/// Disponibilité de l'instance, lue par `/readyz`.
#[derive(Clone)]
pub struct Readiness(Arc<AtomicBool>);

impl Readiness {
    /// Arrêt annoncé : `/readyz` répond 503 pour que le répartiteur retire l'instance, alors que
    /// les requêtes continuent d'être servies.
    pub fn drain(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn draining(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

impl AppState {
    #[must_use]
    pub fn new(registry: Registry, keys: SessionKeys, settings: Settings) -> Self {
        let ip_limiter = Arc::new(limit::IpLimiter::new(settings.auth_per_ip));
        reload::report_loaded(registry.len());
        AppState {
            registry: Arc::new(ArcSwap::from_pointee(registry)),
            keys: Arc::new(keys),
            settings: Arc::new(settings),
            login_limiter: Arc::default(),
            ip_limiter,
            account_slots: Arc::default(),
            draining: Arc::default(),
        }
    }

    /// Poignée de disponibilité, à garder par le code qui gère l'arrêt.
    #[must_use]
    pub fn readiness(&self) -> Readiness {
        Readiness(self.draining.clone())
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "tonAPI",
        description = "Vue unifiée des données universitaires d'un étudiant : emploi du temps, notes, \
absences, examens, devoirs, mail, applications, documents. L'étudiant se connecte avec son compte universitaire ; \
la session est un jeton chiffré qu'il garde, le serveur ne stocke rien.\n\n\
Essai sans compte : université `demo`, identifiant `demo`, mot de passe `demo`.",
        license(name = "AGPL-3.0-only", identifier = "AGPL-3.0-only"),
    ),
    servers((url = "/", description = "Ce serveur")),
    modifiers(&SecurityAddon),
    tags(
        (name = "universités", description = "Universités et services proposés"),
        (name = "session", description = "Connexion et jeton de session"),
        (name = "étudiant", description = "Profil et composante"),
        (name = "données", description = "Données de l'étudiant connecté"),
        (name = "service", description = "Santé et documentation"),
    )
)]
struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "session",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("tonAPI")
                    .description(Some(
                        "Jeton renvoyé par `POST /v1/auth/login`. Un jeton mis à jour revient dans \
l'en-tête `TonAPI-Session` : le client doit alors remplacer l'ancien.",
                    ))
                    .build(),
            ),
        );
    }
}

fn api_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi()).merge(routes::router())
}

/// Spécification OpenAPI complète.
#[must_use]
pub fn openapi() -> utoipa::openapi::OpenApi {
    api_router().into_openapi()
}

/// Application complète : routes, `/openapi.json`, `/docs`, CORS, journal d'accès.
pub fn app(state: AppState) -> Router {
    let (router, api) = api_router().split_for_parts();
    let spec = api.clone();
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::PUT])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .expose_headers([HeaderName::from_static(SESSION_HEADER), access::REQUEST_ID])
        // Le navigateur retient la réponse au contrôle préalable 10 minutes au lieu de le refaire
        // avant chaque appel.
        .max_age(Duration::from_secs(600));
    let docs = state.settings.docs;
    let router = if docs {
        router
            .route(
                "/openapi.json",
                axum::routing::get(move || async move { axum::Json(spec) }),
            )
            .merge(Scalar::with_url("/docs", api))
    } else {
        router
    };
    router
        .fallback(error::not_found)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            guard::protect,
        ))
        .with_state(state)
        .layer(cors)
        .layer(axum::middleware::from_fn(access::log))
}

/// Longueur minimale du jeton de `/metrics` quand l'écouteur n'est pas sur la boucle locale.
pub const METRICS_TOKEN_MIN_LEN: usize = 32;

/// Refuse une exposition des métriques sans protection : si `addr` n'est pas une adresse de
/// bouclage, `TONAPI_METRICS_TOKEN` doit être défini et faire au moins
/// [`METRICS_TOKEN_MIN_LEN`] caractères. Les métriques révèlent le trafic, les universités
/// servies et les hôtes contactés : derrière un port publié ou un réseau partagé, toute machine
/// qui les atteint les lirait.
///
/// # Errors
/// Message en français, prêt à être affiché au démarrage.
pub fn check_metrics_exposure(addr: &str, token: Option<&str>) -> Result<(), String> {
    if is_loopback(addr) {
        return Ok(());
    }
    match token {
        Some(token) if token.chars().count() >= METRICS_TOKEN_MIN_LEN => Ok(()),
        Some(_) => Err(format!(
            "TONAPI_METRICS_TOKEN doit faire au moins {METRICS_TOKEN_MIN_LEN} caractères : \
             TONAPI_METRICS_ADDR={addr} n'est pas une adresse de bouclage"
        )),
        None => Err(format!(
            "TONAPI_METRICS_ADDR={addr} n'est pas une adresse de bouclage : définir \
             TONAPI_METRICS_TOKEN (au moins {METRICS_TOKEN_MIN_LEN} caractères) ou écouter sur \
             127.0.0.1"
        )),
    }
}

/// Adresse `ip:port` de bouclage, ou `localhost:port`.
fn is_loopback(addr: &str) -> bool {
    if let Ok(socket) = addr.parse::<std::net::SocketAddr>() {
        return socket.ip().to_canonical().is_loopback();
    }
    addr.rsplit_once(':')
        .is_some_and(|(host, _)| host.eq_ignore_ascii_case("localhost"))
}

/// Égalité sans arrêt au premier octet différent : la durée ne dépend que de la longueur du
/// jeton présenté, pas de la position de la première différence.
fn same_secret(expected: &[u8], presented: &[u8]) -> bool {
    let mut diff = expected.len() ^ presented.len();
    for i in 0..expected.len().max(presented.len()) {
        let a = expected.get(i).copied().unwrap_or(0);
        let b = presented.get(i).copied().unwrap_or(0);
        diff |= usize::from(a ^ b);
    }
    std::hint::black_box(diff) == 0
}

/// Jeton de l'en-tête `Authorization: Bearer …`.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then_some(token.trim_start())
}

async fn scrape(State(token): State<Option<Arc<str>>>, headers: HeaderMap) -> Response {
    if let Some(expected) = &token
        && !bearer(&headers).is_some_and(|given| same_secret(expected.as_bytes(), given.as_bytes()))
    {
        let mut response = StatusCode::UNAUTHORIZED.into_response();
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        return response;
    }
    // Formatage et lecture de `/proc` hors des fils qui servent les connexions.
    match tokio::task::spawn_blocking(tonapi_metrics::render).await {
        Ok(text) => (
            [(
                header::CONTENT_TYPE,
                "text/plain; version=0.0.4; charset=utf-8",
            )],
            text,
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Application interne : `GET /metrics` au format texte de Prometheus. Elle n'est jamais montée
/// sur le routeur public ([`app`] répond 404) ; à servir sur `TONAPI_METRICS_ADDR`.
///
/// Avec `token`, la requête doit porter `Authorization: Bearer <jeton>` (401 sans détail
/// sinon). Sans jeton, l'écouteur ne doit être joignable que depuis la machine : voir
/// [`check_metrics_exposure`].
pub fn metrics_app(token: Option<&str>) -> Router {
    let token: Option<Arc<str>> = token.filter(|t| !t.is_empty()).map(Arc::from);
    Router::new()
        .route("/metrics", axum::routing::get(scrape))
        .with_state(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposition_des_metriques_hors_boucle_locale_exige_un_jeton() {
        let long = "x".repeat(METRICS_TOKEN_MIN_LEN);
        let short = "x".repeat(METRICS_TOKEN_MIN_LEN - 1);
        // Bouclage : le jeton est facultatif.
        for addr in [
            "127.0.0.1:9090",
            "[::1]:9090",
            "localhost:9090",
            "[::ffff:127.0.0.1]:9090",
        ] {
            assert!(check_metrics_exposure(addr, None).is_ok(), "{addr}");
        }
        // Autre adresse : jeton obligatoire, d'au moins 32 caractères.
        for addr in [
            "0.0.0.0:9090",
            "[::]:9090",
            "10.0.0.5:9090",
            "metrics.interne:9090",
        ] {
            let sans = check_metrics_exposure(addr, None).unwrap_err();
            assert!(sans.contains("TONAPI_METRICS_TOKEN"), "{sans}");
            assert!(
                check_metrics_exposure(addr, Some(&short)).is_err(),
                "{addr}"
            );
            assert!(check_metrics_exposure(addr, Some(&long)).is_ok(), "{addr}");
        }
        // Le message n'écrit jamais le jeton.
        let message = check_metrics_exposure("0.0.0.0:9090", Some(&short)).unwrap_err();
        assert!(!message.contains(&short));
    }

    #[test]
    fn comparaison_du_jeton() {
        assert!(same_secret(b"abc", b"abc"));
        assert!(!same_secret(b"abc", b"abd"));
        assert!(!same_secret(b"abc", b"abcd"));
        assert!(!same_secret(b"abcd", b"abc"));
        assert!(!same_secret(b"abc", b""));
        assert!(same_secret(b"", b""));
    }
}
