//! Protections communes à toutes les requêtes : délai maximal, limite par adresse IP sur les
//! routes de connexion (limiteur dans `limit::ip`), en-têtes de sécurité.

use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{HeaderValue, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tonapi_core::{ErrorCode, Problem};

use crate::AppState;
use crate::error::ApiError;
use crate::limit::client_ip;

/// Délai maximal d'une requête. L'appel groupé enchaîne plusieurs services d'un même
/// fournisseur (chacun borné dans `routes/overview.rs`), le second facteur attend la notification.
const DEADLINE: Duration = Duration::from_secs(30);
const LONG_DEADLINE: Duration = Duration::from_secs(60);
const MFA_DEADLINE: Duration = Duration::from_secs(120);
/// Routes comptées par adresse : celles qui envoient un mot de passe ou un code. `refresh` et
/// `logout` n'en font pas partie (un campus entier peut partager une adresse).
const COUNTED: [&str; 3] = ["/v1/auth/login", "/v1/auth/mfa", "/v1/auth/service-login"];

pub async fn protect(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    if COUNTED.contains(&path.as_str())
        && let Some(ip) = client_ip(&request, &state.settings.trusted_proxies)
        && !state.ip_limiter.allow(ip, Instant::now())
    {
        tonapi_metrics::inc("tonapi_rate_limited_total", &[("limiter", "ip")]);
        let mut response = ApiError::from(Problem::new(ErrorCode::RateLimited).detail(
            "trop de demandes de connexion depuis cette adresse : réessayer dans une minute",
        ))
        .into_response();
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        return response;
    }
    let deadline = if path == "/v1/auth/mfa" {
        MFA_DEADLINE
    } else if path == "/v1/overview" || path.starts_with("/v1/documents/") {
        LONG_DEADLINE
    } else {
        DEADLINE
    };
    let mut response = match tokio::time::timeout(deadline, next.run(request)).await {
        Ok(response) => response,
        Err(_) => ApiError::from(Problem::new(ErrorCode::ServiceUnavailable).detail(format!(
            "le service de l'université n'a pas répondu en {} s",
            deadline.as_secs()
        )))
        .into_response(),
    };
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response
}
