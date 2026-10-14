//! Réponses d'erreur `application/problem+json`.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tonapi_core::{ErrorCode, Problem};

/// Erreur renvoyée par un gestionnaire.
#[derive(Debug)]
pub struct ApiError(pub Problem);

impl From<Problem> for ApiError {
    fn from(problem: Problem) -> Self {
        ApiError(problem)
    }
}

impl From<ErrorCode> for ApiError {
    fn from(code: ErrorCode) -> Self {
        ApiError(Problem::new(code))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let unauthorized = status == StatusCode::UNAUTHORIZED;
        let mut response = (status, axum::Json(self.0)).into_response();
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        if unauthorized {
            headers.insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

/// Requête mal formée (JSON, paramètres).
pub fn invalid(detail: impl std::fmt::Display) -> ApiError {
    ApiError(Problem::new(ErrorCode::InvalidRequest).detail(detail.to_string()))
}

pub async fn not_found() -> ApiError {
    ApiError(Problem::new(ErrorCode::NotFound).detail("route inconnue ; voir /docs"))
}
