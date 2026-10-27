//! Serveur HTTP de tonAPI : routes `/v1`, spécification OpenAPI et documentation Scalar.

mod auth;
mod error;
mod limit;
mod reload;
mod respond;
mod routes;

use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use axum::Router;
use axum::http::{HeaderName, Method, header};
use tonapi_config::Registry;
use tonapi_session::SessionKeys;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::{Scalar, Servable};

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
}

/// État partagé par toutes les requêtes.
#[derive(Clone)]
pub struct AppState {
    /// Universités, remplacées d'un bloc au rechargement.
    pub registry: Arc<ArcSwap<Registry>>,
    pub keys: Arc<SessionKeys>,
    pub settings: Arc<Settings>,
    login_limiter: Arc<limit::LoginLimiter>,
}

impl AppState {
    #[must_use]
    pub fn new(registry: Registry, keys: SessionKeys, settings: Settings) -> Self {
        AppState {
            registry: Arc::new(ArcSwap::from_pointee(registry)),
            keys: Arc::new(keys),
            settings: Arc::new(settings),
            login_limiter: Arc::default(),
        }
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

/// Application complète : routes, `/openapi.json`, `/docs`, CORS, traces.
pub fn app(state: AppState) -> Router {
    let (router, api) = api_router().split_for_parts();
    let spec = api.clone();
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::PUT])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .expose_headers([HeaderName::from_static(SESSION_HEADER)]);
    router
        .route(
            "/openapi.json",
            axum::routing::get(move || async move { axum::Json(spec) }),
        )
        .merge(Scalar::with_url("/docs", api))
        .fallback(error::not_found)
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
