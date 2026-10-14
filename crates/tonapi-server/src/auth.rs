//! Lecture du jeton `Authorization: Bearer`.

use axum::extract::FromRequestParts;
use axum::http::header;
use axum::http::request::Parts;
use chrono::Utc;
use tonapi_core::{ErrorCode, Problem};
use tonapi_session::{SessionData, SessionError};

use crate::AppState;
use crate::error::ApiError;

/// Session de l'étudiant, déchiffrée et non expirée.
pub struct AuthSession(pub SessionData);

impl FromRequestParts<AppState> for AuthSession {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, ApiError>> + Send {
        std::future::ready(open(parts, state))
    }
}

fn open(parts: &Parts, state: &AppState) -> Result<AuthSession, ApiError> {
    let token = parts
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .map(str::trim)
        .ok_or(ErrorCode::InvalidToken)?;
    match state.keys.open(token, Utc::now().timestamp()) {
        Ok(data) if data.pending => Err(Problem::new(ErrorCode::InvalidToken)
            .detail("connexion non terminée : valider le second facteur")
            .into()),
        Ok(data) => Ok(AuthSession(data)),
        Err(SessionError::Expired) => Err(ErrorCode::SessionExpired.into()),
        Err(_) => Err(ErrorCode::InvalidToken.into()),
    }
}
