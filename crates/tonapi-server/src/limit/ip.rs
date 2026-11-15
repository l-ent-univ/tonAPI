//! Limite par adresse IP sur les routes de connexion, répartiteurs de confiance et adresse du
//! client.
//!
//! La limite par IP empêche qu'un afflux de connexions inventées occupe les places réservées à
//! une université (`tonapi-http`, 32 requêtes simultanées par hôte) et fasse bannir l'IP de
//! tonAPI par son CAS. Elle est propre à chaque processus, comme le limiteur par compte.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request};

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

    pub(crate) fn allow(&self, ip: IpAddr, now: Instant) -> bool {
        // `::ffff:a.b.c.d` (serveur à l'écoute sur `[::]`) redevient une IPv4 : sinon le masque
        // /64 met tous les clients IPv4 dans le même compartiment.
        let ip = match ip.to_canonical() {
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
        let allowed = *count <= self.per_window;
        let entries = map.len();
        drop(guard);
        tonapi_metrics::set(
            "tonapi_limiter_entries",
            &[("limiter", "ip")],
            i64::try_from(entries).unwrap_or(i64::MAX),
        );
        allowed
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
        if prefix > max {
            return None;
        }
        // `::ffff:10.0.0.0/104` désigne la plage IPv4 `10.0.0.0/8`.
        match addr {
            IpAddr::V6(v6) if prefix >= 96 => {
                v6.to_ipv4_mapped()
                    .map_or(Some(Cidr { net: addr, prefix }), |v4| {
                        Some(Cidr {
                            net: IpAddr::V4(v4),
                            prefix: prefix - 96,
                        })
                    })
            }
            _ => Some(Cidr { net: addr, prefix }),
        }
    }

    fn contains(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
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
pub(crate) fn client_ip(request: &Request, trusted: &[Cidr]) -> Option<IpAddr> {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_canonical())?;
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
                .map(|ip| ip.to_canonical())
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

    /// Audit 2026-11-04 : à l'écoute sur `[::]`, chaque client IPv4 arrive en `::ffff:a.b.c.d`.
    /// Le masque /64 les mettait tous dans un seul compartiment.
    #[test]
    fn ipv4_mappee_comptee_par_adresse() {
        let limiter = IpLimiter::new(1);
        let now = Instant::now();
        assert!(limiter.allow("::ffff:192.0.2.1".parse().unwrap(), now));
        assert!(limiter.allow("::ffff:192.0.2.2".parse().unwrap(), now));
        assert!(
            !limiter.allow("192.0.2.1".parse().unwrap(), now),
            "même client"
        );

        let lan = Cidr::parse("10.0.0.0/8").unwrap();
        assert!(lan.contains("::ffff:10.0.0.2".parse().unwrap()));
        let mapped = Cidr::parse("::ffff:10.0.0.0/104").unwrap();
        assert!(mapped.contains("10.9.9.9".parse().unwrap()));
        let trusted = [lan];
        let proxied = request("[::ffff:10.0.0.2]:4000", Some("203.0.113.5"));
        assert_eq!(
            client_ip(&proxied, &trusted),
            Some("203.0.113.5".parse().unwrap())
        );
        let direct = request("[::ffff:203.0.113.9]:4000", None);
        assert_eq!(
            client_ip(&direct, &trusted),
            Some("203.0.113.9".parse().unwrap())
        );
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
