//! Profil et composante de l'étudiant.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tonapi_core::{ErrorCode, EstablishmentChoice, Problem, Profile, ServiceStatus};
use tonapi_providers::Fetched;
use utoipa::ToSchema;

use super::{establishment_choice, service_statuses};
use crate::AppState;
use crate::auth::AuthSession;
use crate::error::{ApiError, invalid};
use crate::respond::{DataResponse, attach_session, no_store, respond};

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UniversityRef {
    id: String,
    name: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Me {
    /// Profil ; réduit à l'identifiant quand l'université ne fournit pas de profil.
    profile: Profile,
    university: UniversityRef,
    establishment: Option<EstablishmentChoice>,
    /// Services de l'étudiant, compte tenu de sa composante.
    services: Vec<ServiceStatus>,
}

/// Profil, composante et services de l'étudiant connecté.
#[utoipa::path(
    get,
    path = "/v1/me",
    tag = "étudiant",
    security(("session" = [])),
    responses(
        (status = 200, description = "Succès", body = DataResponse<Me>),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn me(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
) -> Result<Response, ApiError> {
    respond(&state, session, None, async |ctx| {
        let (profile, provider) = match tonapi_providers::profile(ctx).await {
            Ok(fetched) => (fetched.data, fetched.provider),
            Err(_) => (
                Profile {
                    username: ctx.session.username.clone(),
                    display_name: None,
                    first_name: None,
                    last_name: None,
                    email: None,
                    student_number: None,
                    program: None,
                },
                "session",
            ),
        };
        let pick = ctx.session.establishment.as_ref();
        Ok(Fetched {
            data: Me {
                profile,
                university: UniversityRef {
                    id: ctx.university.university.id.clone(),
                    name: ctx.university.university.name.clone(),
                },
                establishment: establishment_choice(ctx.university, pick),
                services: service_statuses(
                    &ctx.university.services_for(pick.map(|p| p.id.as_str())),
                ),
            },
            provider,
        })
    })
    .await
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetEstablishment {
    /// Composante choisie ; `null` revient au choix automatique.
    #[schema(example = "iut-beziers")]
    establishment: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EstablishmentUpdate {
    establishment: Option<EstablishmentChoice>,
    services: Vec<ServiceStatus>,
}

/// Corrige la composante de l'étudiant. Le nouveau jeton revient dans l'en-tête `TonAPI-Session`.
#[utoipa::path(
    put,
    path = "/v1/me/establishment",
    tag = "étudiant",
    security(("session" = [])),
    request_body = SetEstablishment,
    responses(
        (status = 200, description = "Succès", body = EstablishmentUpdate, headers(("TonAPI-Session" = String, description = "Nouveau jeton"))),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`unknown_establishment`", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn set_establishment(
    State(state): State<AppState>,
    AuthSession(mut session): AuthSession,
    payload: Result<Json<SetEstablishment>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(request) = payload.map_err(|e| invalid(e.body_text()))?;
    let registry = state.registry.load_full();
    let university = registry
        .get(&session.university)
        .ok_or_else(|| Problem::new(ErrorCode::UnknownUniversity))?;
    session.establishment =
        tonapi_providers::pick_establishment(university, request.establishment.as_deref())?;
    let pick = session.establishment.as_ref();
    let mut response = Json(EstablishmentUpdate {
        establishment: establishment_choice(university, pick),
        services: service_statuses(&university.services_for(pick.map(|p| p.id.as_str()))),
    })
    .into_response();
    no_store(&mut response);
    attach_session(&state, &session, &mut response)?;
    Ok(response)
}
