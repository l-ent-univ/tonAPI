//! Connexion, renouvellement et déconnexion.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use tonapi_core::{ErrorCode, EstablishmentChoice, Problem};
use utoipa::ToSchema;

use super::establishment_choice;
use crate::AppState;
use crate::auth::AuthSession;
use crate::error::{ApiError, invalid};
use crate::respond::no_store;

/// Identifiants de l'étudiant. Le mot de passe sert à la connexion puis est oublié.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginRequest {
    #[schema(example = "demo")]
    university: String,
    #[schema(example = "demo")]
    username: String,
    #[schema(example = "demo", format = Password)]
    password: String,
    /// Composante choisie par l'étudiant. Par défaut : détectée, sinon celle par défaut.
    establishment: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    /// Jeton à envoyer dans `Authorization: Bearer`.
    token: String,
    expires_at: DateTime<FixedOffset>,
    university: String,
    establishment: Option<EstablishmentChoice>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenResponse {
    token: String,
    expires_at: DateTime<FixedOffset>,
}

fn timestamp(secs: i64) -> DateTime<FixedOffset> {
    DateTime::from_timestamp(secs, 0)
        .unwrap_or_default()
        .fixed_offset()
}

/// Connexion avec le compte universitaire.
#[utoipa::path(
    post,
    path = "/v1/auth/login",
    tag = "session",
    security(()),
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Succès", body = LoginResponse),
        (status = 400, description = "`invalid_request` : paramètres invalides", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "`invalid_credentials`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`unknown_university`, `unknown_establishment`", body = Problem, content_type = "application/problem+json"),
        (status = 501, description = "`not_implemented` : connexion pas encore prise en charge", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn login(
    State(state): State<AppState>,
    payload: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(request) = payload.map_err(|e| invalid(e.body_text()))?;
    let registry = state.registry.load_full();
    let university = registry.get(&request.university).ok_or_else(|| {
        Problem::new(ErrorCode::UnknownUniversity)
            .detail(format!("`{}` n'est pas servie", request.university))
    })?;
    let session = tonapi_providers::login(
        university,
        &request.username,
        &request.password,
        request.establishment.as_deref(),
        Utc::now(),
        state.settings.session_ttl_secs,
    )
    .await?;
    let token = state.keys.seal(&session).map_err(|_| ErrorCode::Internal)?;
    let mut response = Json(LoginResponse {
        token,
        expires_at: timestamp(session.expires_at),
        university: session.university.clone(),
        establishment: establishment_choice(university, session.establishment.as_ref()),
    })
    .into_response();
    no_store(&mut response);
    Ok(response)
}

/// Prolonge la session et renvoie un nouveau jeton.
#[utoipa::path(
    post,
    path = "/v1/auth/refresh",
    tag = "session",
    security(("session" = [])),
    responses(
        (status = 200, description = "Succès", body = TokenResponse),
        (status = 401, description = "`invalid_token`, `session_expired`", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn refresh(
    State(state): State<AppState>,
    AuthSession(mut session): AuthSession,
) -> Result<Response, ApiError> {
    session.expires_at = Utc::now().timestamp() + state.settings.session_ttl_secs;
    let token = state.keys.seal(&session).map_err(|_| ErrorCode::Internal)?;
    let mut response = Json(TokenResponse {
        token,
        expires_at: timestamp(session.expires_at),
    })
    .into_response();
    no_store(&mut response);
    Ok(response)
}

/// Déconnexion. Le client oublie son jeton ; rien n'est gardé côté serveur.
#[utoipa::path(
    post,
    path = "/v1/auth/logout",
    tag = "session",
    security(("session" = [])),
    responses(
        (status = 204, description = "Session terminée"),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn logout(AuthSession(_session): AuthSession) -> StatusCode {
    StatusCode::NO_CONTENT
}
