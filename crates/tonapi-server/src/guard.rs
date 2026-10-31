//! Protections communes à toutes les requêtes : délai maximal, limite par adresse IP sur les
//! routes de connexion, en-têtes de sécurité.
//!
//! La limite par IP empêche qu'un afflux de connexions inventées occupe les places réservées à
//! une université (`tonapi-http`, 32 requêtes simultanées par hôte) et fasse bannir l'IP de
//! tonAPI par son CAS. Elle est propre à chaque processus, comme le limiteur par compte.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderValue, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tonapi_core::{ErrorCode, Problem};

use crate::AppState;
use crate::error::ApiError;

/// Délai maximal d'une requête ; l'attente d'une notification de second facteur dure plus.
const DEADLINE: Duration = Duration::from_secs(30);
const MFA_DEADLINE: Duration = Duration::from_secs(120);
/// Requêtes vers `/v1/auth/*` par adresse IP et par fenêtre.
const AUTH_PER_IP: u32 = 20;
const AUTH_WINDOW: Duration = Duration::from_secs(60);
/// Au-delà, les fenêtres terminées sont retirées.
const MAX_IPS: usize = 100_000;

/// Compteur à fenêtre fixe par adresse IP.
#[derive(Default)]
pub struct IpLimiter {
    windows: Mutex<HashMap<IpAddr, (Instant, u32)>>,
}

impl IpLimiter {
    fn allow(&self, ip: IpAddr, now: Instant) -> bool {
        let mut map = self
            .windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if map.len() >= MAX_IPS {
            map.retain(|_, (start, _)| now.duration_since(*start) < AUTH_WINDOW);
        }
        let (start, count) = map.entry(ip).or_insert((now, 0));
        if now.duration_since(*start) >= AUTH_WINDOW {
            *start = now;
            *count = 0;
        }
        *count += 1;
        *count <= AUTH_PER_IP
    }
}

/// Adresse du client : la connexion TCP, ou derrière un répartiteur de confiance
/// (`TONAPI_TRUST_PROXY=1`) la dernière adresse de `X-Forwarded-For`, ajoutée par lui.
fn client_ip(request: &Request, trust_proxy: bool) -> Option<IpAddr> {
    if trust_proxy
        && let Some(ip) = request
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse().ok())
    {
        return Some(ip);
    }
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip())
}

pub async fn protect(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    if path.starts_with("/v1/auth/")
        && let Some(ip) = client_ip(&request, state.settings.trust_proxy)
        && !state.ip_limiter.allow(ip, Instant::now())
    {
        return ApiError::from(Problem::new(ErrorCode::RateLimited).detail(format!(
            "plus de {AUTH_PER_IP} demandes de connexion par minute depuis cette adresse"
        )))
        .into_response();
    }
    let deadline = if path == "/v1/auth/mfa" {
        MFA_DEADLINE
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fenetre_par_adresse() {
        let limiter = IpLimiter::default();
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        let other: IpAddr = "192.0.2.2".parse().unwrap();
        let now = Instant::now();
        for _ in 0..AUTH_PER_IP {
            assert!(limiter.allow(ip, now));
        }
        assert!(!limiter.allow(ip, now));
        assert!(limiter.allow(other, now));
        assert!(limiter.allow(ip, now + AUTH_WINDOW), "nouvelle fenêtre");
    }
}
