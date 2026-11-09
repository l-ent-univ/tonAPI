//! Universités servies et santé du serveur.

use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use tonapi_config::{PortalConfig, ResolvedUniversity, Sso, Status};
use tonapi_core::{ErrorCode, Problem, ServiceStatus};
use utoipa::ToSchema;

use super::service_statuses;
use crate::AppState;
use crate::error::ApiError;

#[derive(Serialize, ToSchema)]
pub struct Health {
    status: &'static str,
    universities: usize,
}

/// Santé du serveur.
#[utoipa::path(get, path = "/healthz", tag = "service", security(()), responses((status = 200, description = "Succès", body = Health)))]
pub async fn health(State(state): State<AppState>) -> Json<Health> {
    Json(Health {
        status: "ok",
        universities: state.registry.load().len(),
    })
}

#[derive(Serialize, ToSchema)]
pub struct Ready {
    status: &'static str,
}

/// Disponibilité de l'instance pour le répartiteur : 503 pendant l'arrêt. Ne contacte aucune
/// université.
#[utoipa::path(get, path = "/readyz", tag = "service", security(()), responses(
    (status = 200, description = "Prête à recevoir des requêtes", body = Ready),
    (status = 503, description = "`service_unavailable` : arrêt en cours ou aucune université chargée", body = Problem, content_type = "application/problem+json"),
))]
pub async fn ready(State(state): State<AppState>) -> Result<Json<Ready>, ApiError> {
    if state.readiness().draining() {
        return Err(not_ready("arrêt en cours"));
    }
    if state.registry.load().is_empty() {
        return Err(not_ready("aucune université chargée"));
    }
    Ok(Json(Ready { status: "ready" }))
}

fn not_ready(detail: &str) -> ApiError {
    let mut problem = Problem::new(ErrorCode::ServiceUnavailable).detail(detail);
    problem.status = 503;
    problem.into()
}

/// Connexion attendue par l'université.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoginInfo {
    /// `cas` ou `demo`.
    kind: &'static str,
    /// L'université peut demander un second facteur.
    mfa: bool,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EstablishmentSummary {
    id: String,
    name: String,
    /// Composante retenue quand rien d'autre ne correspond.
    default: bool,
    /// `active`, `beta` ou `planned`.
    status: &'static str,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UniversitySummary {
    id: String,
    name: String,
    short_name: Option<String>,
    /// `active`, `beta` ou `planned`.
    status: &'static str,
    login: LoginInfo,
    establishments: Vec<EstablishmentSummary>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EstablishmentDetail {
    id: String,
    name: String,
    default: bool,
    status: &'static str,
    /// Services d'un étudiant de cette composante.
    services: Vec<ServiceStatus>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UniversityDetail {
    id: String,
    name: String,
    short_name: Option<String>,
    status: &'static str,
    /// Fuseau horaire IANA des données.
    timezone: String,
    login: LoginInfo,
    /// Logiciel du portail qui sert les applications et le profil : `uportal4-prolongation`,
    /// `uportal5`, `tiles-portal`, ou absent.
    portal: Option<&'static str>,
    /// Liens utiles (`website`, `passwordReset`…).
    links: Vec<LinkInfo>,
    /// Services communs à toute l'université.
    services: Vec<ServiceStatus>,
    establishments: Vec<EstablishmentDetail>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LinkInfo {
    kind: &'static str,
    url: String,
}

fn status(s: Status) -> &'static str {
    match s {
        Status::Active => "active",
        Status::Beta => "beta",
        Status::Planned => "planned",
    }
}

fn login(u: &ResolvedUniversity) -> LoginInfo {
    match &u.university.sso {
        Sso::Cas(cas) => LoginInfo {
            kind: "cas",
            mfa: cas.mfa.is_some(),
        },
        Sso::Demo => LoginInfo {
            kind: "demo",
            mfa: false,
        },
    }
}

fn summary(u: &ResolvedUniversity) -> UniversitySummary {
    UniversitySummary {
        id: u.university.id.clone(),
        name: u.university.name.clone(),
        short_name: u.university.short_name.clone(),
        status: status(u.university.status),
        login: login(u),
        establishments: u
            .establishments
            .values()
            .map(|e| EstablishmentSummary {
                id: e.id.clone(),
                name: e.name.clone(),
                default: e.default,
                status: status(e.status),
            })
            .collect(),
    }
}

/// Universités servies.
#[utoipa::path(
    get,
    path = "/v1/universities",
    tag = "universités",
    security(()),
    responses((status = 200, description = "Succès", body = Vec<UniversitySummary>))
)]
pub async fn list(State(state): State<AppState>) -> Json<Vec<UniversitySummary>> {
    Json(state.registry.load().iter().map(summary).collect())
}

/// Détail d'une université : services, composantes, liens.
#[utoipa::path(
    get,
    path = "/v1/universities/{id}",
    tag = "universités",
    security(()),
    params(("id" = String, Path, description = "Identifiant de l'université", example = "demo")),
    responses(
        (status = 200, description = "Succès", body = UniversityDetail),
        (status = 404, description = "`service_absent` (avec `link` si le service est un simple lien) ou ressource inconnue", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<UniversityDetail>, ApiError> {
    let registry = state.registry.load();
    let u = registry.get(&id).ok_or_else(|| {
        Problem::new(ErrorCode::UnknownUniversity).detail(format!("`{id}` n'est pas servie"))
    })?;
    let l = &u.university.links;
    let links = [
        ("website", &l.website),
        ("accountHelp", &l.account_help),
        ("passwordReset", &l.password_reset),
        ("accountActivation", &l.account_activation),
    ]
    .into_iter()
    .filter_map(|(kind, url)| {
        url.as_ref().map(|url| LinkInfo {
            kind,
            url: url.to_string(),
        })
    })
    .collect();
    Ok(Json(UniversityDetail {
        id: u.university.id.clone(),
        name: u.university.name.clone(),
        short_name: u.university.short_name.clone(),
        status: status(u.university.status),
        timezone: u.university.timezone.clone(),
        login: login(u),
        portal: match u.university.portal {
            PortalConfig::Uportal4Prolongation { .. } => Some("uportal4-prolongation"),
            PortalConfig::Uportal5 { .. } => Some("uportal5"),
            PortalConfig::TilesPortal { .. } => Some("tiles-portal"),
            PortalConfig::None => None,
        },
        links,
        services: service_statuses(&u.university.services),
        establishments: u
            .establishments
            .values()
            .map(|e| EstablishmentDetail {
                id: e.id.clone(),
                name: e.name.clone(),
                default: e.default,
                status: status(e.status),
                services: service_statuses(&u.services_for(Some(&e.id))),
            })
            .collect(),
    }))
}
