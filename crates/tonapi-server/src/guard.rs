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

/// Plage d'adresses (`10.0.0.0/8`, `2001:db8::/32`) ou adresse seule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    net: IpAddr,
    prefix: u8,
}

impl Cidr {
    /// Lit `adresse` ou `adresse/longueur`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Cidr> {
        let text = text.trim();
        let (addr, prefix) = if let Some((addr, len)) = text.split_once('/') {
            (addr.parse::<IpAddr>().ok()?, len.parse::<u8>().ok()?)
        } else {
            let addr = text.parse::<IpAddr>().ok()?;
            (addr, if addr.is_ipv4() { 32 } else { 128 })
        };
        let max = if addr.is_ipv4() { 32 } else { 128 };
        (prefix <= max).then_some(Cidr { net: addr, prefix })
    }

    fn contains(&self, ip: IpAddr) -> bool {
        let mask = |bits: u32, prefix: u8| {
            if prefix == 0 {
                0
            } else {
                u128::MAX << (bits - u32::from(prefix))
            }
        };
        match (self.net, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let m = mask(32, self.prefix);
                u128::from(u32::from(net)) & m == u128::from(u32::from(ip)) & m
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let m = mask(128, self.prefix);
                u128::from(net) & m == u128::from(ip) & m
            }
            _ => false,
        }
    }
}

/// Adresse du client. Sans répartiteur de confiance, celle de la connexion TCP. Si la
/// connexion vient d'un répartiteur déclaré (`TONAPI_TRUSTED_PROXIES`), `X-Forwarded-For` est
/// lu de droite à gauche et la première adresse non fiable est retenue : un client qui forge
/// l'en-tête en contactant directement le serveur ne change pas son adresse.
fn client_ip(request: &Request, trusted: &[Cidr]) -> Option<IpAddr> {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip())?;
    let is_trusted = |ip: IpAddr| trusted.iter().any(|c| c.contains(ip));
    if !is_trusted(peer) {
        return Some(peer);
    }
    let forwarded = request
        .headers()
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter_map(|v| {
            v.parse::<IpAddr>()
                .ok()
                .or_else(|| v.parse::<SocketAddr>().ok().map(|a| a.ip()))
        })
        .collect::<Vec<_>>();
    Some(
        forwarded
            .into_iter()
            .rev()
            .find(|ip| !is_trusted(*ip))
            .unwrap_or(peer),
    )
}

pub async fn protect(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    if COUNTED.contains(&path.as_str())
        && let Some(ip) = client_ip(&request, &state.settings.trusted_proxies)
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
    fn plages_de_proxys() {
        let lan = Cidr::parse("10.0.0.0/8").unwrap();
        assert!(lan.contains("10.1.2.3".parse().unwrap()));
        assert!(!lan.contains("11.0.0.1".parse().unwrap()));
        let one = Cidr::parse("192.0.2.7").unwrap();
        assert!(one.contains("192.0.2.7".parse().unwrap()));
        assert!(!one.contains("192.0.2.8".parse().unwrap()));
        let v6 = Cidr::parse("2001:db8::/32").unwrap();
        assert!(v6.contains("2001:db8:ffff::1".parse().unwrap()));
        assert!(!v6.contains("10.0.0.1".parse().unwrap()));
        assert!(
            Cidr::parse("0.0.0.0/0")
                .unwrap()
                .contains("8.8.8.8".parse().unwrap())
        );
        assert_eq!(Cidr::parse("10.0.0.0/33"), None);
        assert_eq!(Cidr::parse("pas une adresse"), None);
    }

    fn request(peer: &str, forwarded: Option<&str>) -> Request {
        let mut builder = Request::builder().uri("/v1/auth/login");
        if let Some(f) = forwarded {
            builder = builder.header("x-forwarded-for", f);
        }
        let mut request = builder.body(axum::body::Body::empty()).unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
        request
    }

    #[test]
    fn x_forwarded_for_seulement_depuis_un_proxy_de_confiance() {
        let trusted = [Cidr::parse("10.0.0.0/8").unwrap()];
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        // Client direct qui forge l'en-tête : son adresse TCP compte.
        let forged = request("203.0.113.9:4000", Some("198.51.100.1"));
        assert_eq!(client_ip(&forged, &trusted), Some(ip("203.0.113.9")));
        // Derrière le répartiteur : première adresse non fiable en partant de la droite.
        let proxied = request("10.0.0.2:4000", Some("198.51.100.1, 203.0.113.5, 10.0.0.3"));
        assert_eq!(client_ip(&proxied, &trusted), Some(ip("203.0.113.5")));
        // Aucun répartiteur déclaré : l'en-tête est ignoré.
        assert_eq!(client_ip(&proxied, &[]), Some(ip("10.0.0.2")));
    }

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
