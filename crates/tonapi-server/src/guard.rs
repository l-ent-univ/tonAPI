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

/// Délai maximal d'une requête. L'appel groupé enchaîne plusieurs services d'un même
/// fournisseur (chacun borné dans `routes/overview.rs`), le second facteur attend la notification.
const DEADLINE: Duration = Duration::from_secs(30);
const LONG_DEADLINE: Duration = Duration::from_secs(60);
const MFA_DEADLINE: Duration = Duration::from_secs(120);
/// Routes comptées par adresse : celles qui envoient un mot de passe ou un code. `refresh` et
/// `logout` n'en font pas partie (un campus entier peut partager une adresse).
const COUNTED: [&str; 3] = ["/v1/auth/login", "/v1/auth/mfa", "/v1/auth/service-login"];
/// Demandes par adresse et par fenêtre, par défaut. Réglable : `TONAPI_AUTH_PER_IP`.
pub const AUTH_PER_IP: u32 = 120;
const AUTH_WINDOW: Duration = Duration::from_secs(60);
/// Au-delà, les fenêtres terminées sont retirées (au plus une fois toutes les 10 s).
const MAX_IPS: usize = 100_000;
const PURGE_EVERY: Duration = Duration::from_secs(10);

/// Fenêtres en cours (début, nombre de demandes) et date de la dernière purge.
type Windows = (HashMap<IpAddr, (Instant, u32)>, Option<Instant>);

/// Compteur à fenêtre fixe par adresse IP (IPv6 regroupée par /64, la taille d'un abonnement).
pub struct IpLimiter {
    per_window: u32,
    windows: Mutex<Windows>,
}

impl Default for IpLimiter {
    fn default() -> Self {
        IpLimiter::new(AUTH_PER_IP)
    }
}

impl IpLimiter {
    #[must_use]
    pub fn new(per_window: u32) -> Self {
        IpLimiter {
            per_window,
            windows: Mutex::new((HashMap::new(), None)),
        }
    }

    fn allow(&self, ip: IpAddr, now: Instant) -> bool {
        let ip = match ip {
            IpAddr::V6(v6) => {
                let mut s = v6.segments();
                s[4..].fill(0);
                IpAddr::from(s)
            }
            v4 @ IpAddr::V4(_) => v4,
        };
        let mut guard = self
            .windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (map, last_purge) = &mut *guard;
        if map.len() >= MAX_IPS && last_purge.is_none_or(|t| now.duration_since(t) >= PURGE_EVERY) {
            map.retain(|_, (start, _)| now.duration_since(*start) < AUTH_WINDOW);
            *last_purge = Some(now);
        }
        let (start, count) = map.entry(ip).or_insert((now, 0));
        if now.duration_since(*start) >= AUTH_WINDOW {
            *start = now;
            *count = 0;
        }
        *count = count.saturating_add(1);
        *count <= self.per_window
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
            .map(str::trim)
            .and_then(|v| {
                v.parse::<IpAddr>()
                    .ok()
                    .or_else(|| v.parse::<SocketAddr>().ok().map(|a| a.ip()))
            })
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
    if COUNTED.contains(&path.as_str())
        && let Some(ip) = client_ip(&request, state.settings.trust_proxy)
        && !state.ip_limiter.allow(ip, Instant::now())
    {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv6_regroupee_par_64() {
        let limiter = IpLimiter::new(1);
        let now = Instant::now();
        assert!(limiter.allow("2001:db8:1:2::1".parse().unwrap(), now));
        assert!(!limiter.allow("2001:db8:1:2::ffff".parse().unwrap(), now));
        assert!(limiter.allow("2001:db8:1:3::1".parse().unwrap(), now));
    }

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
