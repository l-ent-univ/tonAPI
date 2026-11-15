//! Journal d'accès et métriques des requêtes reçues.
//!
//! Une ligne par requête (cible `tonapi::access`) : identifiant de requête, méthode, route modèle
//! (`/v1/documents/{id}`, jamais le chemin réel), statut, durée, code d'erreur, université et
//! fournisseur. Jamais d'identifiant d'étudiant, de jeton, de paramètre de requête ni d'en-tête.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{MatchedPath, Request};
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use tonapi_core::ErrorCode;

/// En-tête de corrélation, repris du répartiteur s'il est sûr, sinon créé.
pub const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// Université concernée par la réponse (identifiant de configuration), posée par le gestionnaire.
#[derive(Clone, Debug)]
pub struct University(pub String);

/// Fournisseur qui a répondu.
#[derive(Clone, Copy, Debug)]
pub struct Provider(pub &'static str);

/// Préfixe propre au processus : deux instances ne produisent pas les mêmes identifiants.
static PREFIX: LazyLock<String> = LazyLock::new(|| {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    #[allow(clippy::cast_possible_truncation)] // seuls les bits de poids faible servent
    let mixed = (nanos as u64) ^ (u64::from(std::process::id()) << 32);
    format!("{:08x}", mixed & 0xffff_ffff)
});
static NEXT: AtomicU64 = AtomicU64::new(1);

/// Identifiant reçu acceptable : court, sans caractère qui casserait un journal.
fn usable(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn request_id(request: &Request) -> String {
    request
        .headers()
        .get(&REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .filter(|id| usable(id))
        .map_or_else(
            || format!("{}-{:x}", *PREFIX, NEXT.fetch_add(1, Ordering::Relaxed)),
            str::to_owned,
        )
}

/// Requête en cours dans la jauge `tonapi_http_requests_in_flight`. Retirée au `Drop` : une
/// requête abandonnée par le client (futur annulé) ne laisse pas la jauge gonflée.
struct InFlight;

impl InFlight {
    fn enter() -> Self {
        tonapi_metrics::add("tonapi_http_requests_in_flight", &[], 1);
        InFlight
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        tonapi_metrics::add("tonapi_http_requests_in_flight", &[], -1);
    }
}

/// Intergiciel le plus externe : mesure toute la requête, garde et CORS compris.
pub async fn log(request: Request, next: Next) -> Response {
    let _in_flight = InFlight::enter();
    let started = Instant::now();
    let id = request_id(&request);
    let method = request.method().clone();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("(inconnue)", MatchedPath::as_str)
        .to_owned();
    let mut response = next.run(request).await;
    let elapsed = started.elapsed();
    let status = response.status().as_u16();
    let code = response.extensions().get::<ErrorCode>().copied();
    let university = response
        .extensions()
        .get::<University>()
        .map(|u| u.0.clone());
    let provider = response.extensions().get::<Provider>().map(|p| p.0);
    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert(REQUEST_ID, value);
    }

    let duration_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    let error = code.map(ErrorCode::as_str);
    let university_label = university.as_deref().unwrap_or("");
    let provider_label = provider.unwrap_or("");
    // `/readyz` à 503 pendant un arrêt est attendu : ni alerte ni erreur comptée.
    let probe = route == "/readyz";
    let alarming = !probe
        && (status >= 500
            || matches!(
                code,
                Some(ErrorCode::UpstreamChanged | ErrorCode::ServiceUnavailable)
            ));
    if alarming {
        tracing::warn!(
            target: "tonapi::access",
            request_id = %id,
            method = %method,
            route = %route,
            status,
            duration_ms,
            error = error.unwrap_or(""),
            university = university_label,
            provider = provider_label,
            "requête"
        );
    } else {
        tracing::info!(
            target: "tonapi::access",
            request_id = %id,
            method = %method,
            route = %route,
            status,
            duration_ms,
            error = error.unwrap_or(""),
            university = university_label,
            provider = provider_label,
            "requête"
        );
    }

    let status_text = status.to_string();
    tonapi_metrics::inc(
        "tonapi_requests_total",
        &[("route", &route), ("status", &status_text)],
    );
    tonapi_metrics::observe(
        "tonapi_request_duration_seconds",
        &[("route", &route)],
        elapsed.as_secs_f64(),
    );
    if let Some(code) = error.filter(|_| !probe) {
        tonapi_metrics::inc(
            "tonapi_errors_total",
            &[("code", code), ("university", university_label)],
        );
    }
    response
}

/// Taille d'un jeton émis.
pub fn token_issued(token: &str) {
    #[allow(clippy::cast_precision_loss)] // quelques Ko : exact en f64
    tonapi_metrics::observe("tonapi_session_token_bytes", &[], token.len() as f64);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiant_recu_filtre() {
        assert!(usable("abc-123_x.y"));
        assert!(!usable(""));
        assert!(!usable(&"a".repeat(65)));
        assert!(!usable("a b"));
        assert!(!usable("a\"b"));
        assert!(!usable("é"));
    }
}
