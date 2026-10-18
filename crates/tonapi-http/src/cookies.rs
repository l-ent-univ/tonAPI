//! Pot de cookies : ce qu'un navigateur garderait, sérialisable dans le jeton de session.
//!
//! Règles (RFC 6265, simplifiées) : `Domain` sans point initial ; sans `Domain`, le cookie reste
//! attaché à l'hôte exact ; chemin comparé par préfixe de segment (`/cas` couvre `/cas/login-…`) ;
//! `Max-Age` prime sur `Expires` ; un cookie expiré ou à `Max-Age` ≤ 0 est supprimé.

use chrono::DateTime;
use tonapi_session::StoredCookie;
use url::Url;

#[derive(Debug, Clone, Default)]
pub struct CookieJar {
    cookies: Vec<StoredCookie>,
}

impl CookieJar {
    #[must_use]
    pub fn from_stored(cookies: Vec<StoredCookie>) -> Self {
        CookieJar { cookies }
    }

    /// Cookies encore valables à `now` (secondes Unix).
    #[must_use]
    pub fn to_stored(&self, now: i64) -> Vec<StoredCookie> {
        self.cookies
            .iter()
            .filter(|c| c.expires.is_none_or(|e| e > now))
            .cloned()
            .collect()
    }

    /// Vrai si un cookie `name` est posé pour `host`.
    #[must_use]
    pub fn contains(&self, host: &str, name: &str) -> bool {
        self.cookies
            .iter()
            .any(|c| c.name == name && domain_matches(c, host))
    }

    /// Vrai si au moins un cookie est posé pour `host`.
    #[must_use]
    pub fn has_cookies_for(&self, host: &str) -> bool {
        self.cookies.iter().any(|c| domain_matches(c, host))
    }

    /// Oublie les cookies envoyés à `host`.
    pub fn remove_host(&mut self, host: &str) {
        self.cookies.retain(|c| !domain_matches(c, host));
    }

    /// Enregistre un en-tête `Set-Cookie` reçu de `url`.
    pub fn store(&mut self, url: &Url, header: &str, now: i64) {
        let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
            return;
        };
        let mut parts = header.split(';');
        let Some((name, value)) = parts.next().and_then(|p| p.split_once('=')) else {
            return;
        };
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let mut cookie = StoredCookie {
            domain: host.clone(),
            path: default_path(url),
            name: name.to_owned(),
            value: value.trim().to_owned(),
            host_only: true,
            expires: None,
        };
        let mut max_age = None;
        for attribute in parts {
            let (key, val) = attribute
                .split_once('=')
                .map_or((attribute.trim(), ""), |(k, v)| (k.trim(), v.trim()));
            match key.to_ascii_lowercase().as_str() {
                "domain" if !val.is_empty() => {
                    let domain = val.trim_start_matches('.').to_ascii_lowercase();
                    if host != domain && !host.ends_with(&format!(".{domain}")) {
                        return; // domaine étranger : refusé
                    }
                    cookie.domain = domain;
                    cookie.host_only = false;
                }
                "path" if val.starts_with('/') => val.clone_into(&mut cookie.path),
                "max-age" => max_age = val.parse::<i64>().ok(),
                "expires" if cookie.expires.is_none() => cookie.expires = parse_expires(val),
                _ => {}
            }
        }
        if let Some(seconds) = max_age {
            cookie.expires = Some(now + seconds);
        }
        self.cookies.retain(|c| {
            !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
        });
        if cookie.expires.is_none_or(|e| e > now) {
            self.cookies.push(cookie);
        }
    }

    /// Valeur de l'en-tête `Cookie` pour `url`, chemins les plus longs d'abord.
    #[must_use]
    pub fn header_for(&self, url: &Url, now: i64) -> Option<String> {
        let host = url.host_str()?.to_ascii_lowercase();
        let path = url.path();
        let mut matching: Vec<&StoredCookie> = self
            .cookies
            .iter()
            .filter(|c| c.expires.is_none_or(|e| e > now))
            .filter(|c| domain_matches(c, &host) && path_matches(&c.path, path))
            .collect();
        if matching.is_empty() {
            return None;
        }
        matching.sort_by_key(|c| std::cmp::Reverse(c.path.len()));
        Some(
            matching
                .iter()
                .map(|c| format!("{}={}", c.name, c.value))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }
}

fn domain_matches(cookie: &StoredCookie, host: &str) -> bool {
    if cookie.host_only {
        cookie.domain == host
    } else {
        host == cookie.domain || host.ends_with(&format!(".{}", cookie.domain))
    }
}

fn path_matches(cookie_path: &str, request_path: &str) -> bool {
    request_path == cookie_path
        || (request_path.starts_with(cookie_path)
            && (cookie_path.ends_with('/')
                || request_path.as_bytes().get(cookie_path.len()) == Some(&b'/')))
}

/// Chemin par défaut : le « dossier » de l'URL (RFC 6265 §5.1.4).
fn default_path(url: &Url) -> String {
    let path = url.path();
    match path.rfind('/') {
        Some(0) | None => "/".to_owned(),
        Some(i) => path[..i].to_owned(),
    }
}

/// `Wed, 21 Oct 2026 07:28:00 GMT`, ou la variante à tirets `Wed, 21-Oct-2026 07:28:00 GMT`.
fn parse_expires(value: &str) -> Option<i64> {
    let normalized = value.replace('-', " ");
    DateTime::parse_from_rfc2822(&normalized)
        .or_else(|_| DateTime::parse_from_rfc2822(&normalized.replace("GMT", "+0000")))
        .ok()
        .map(|d| d.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn chemins_de_la_passerelle_de_montpellier() {
        let mut jar = CookieJar::default();
        let cas = url("https://cas.umontpellier.fr/pre-otp/login?service=x");
        jar.store(&cas, "TGC=tgt; Path=/pre-otp/; Secure; HttpOnly", 0);
        jar.store(&cas, "connect.sid=s; Path=/cas; Max-Age=2592000", 0);

        assert_eq!(
            jar.header_for(&url("https://cas.umontpellier.fr/pre-otp/login"), 0)
                .as_deref(),
            Some("TGC=tgt")
        );
        assert_eq!(
            jar.header_for(
                &url("https://cas.umontpellier.fr/cas/login-abc?service=x"),
                0
            )
            .as_deref(),
            Some("connect.sid=s")
        );
        assert_eq!(
            jar.header_for(&url("https://cas.umontpellier.fr/castor"), 0),
            None
        );
        assert_eq!(
            jar.header_for(&url("https://ent.umontpellier.fr/pre-otp/"), 0),
            None
        );
    }

    #[test]
    fn domaine_expiration_et_remplacement() {
        let mut jar = CookieJar::default();
        let page = url("https://sp.partage.renater.fr/a/b");
        jar.store(&page, "S=1; Domain=.renater.fr; Path=/", 0);
        jar.store(&page, "X=1; Domain=exemple.com", 0);
        assert!(
            jar.header_for(&url("https://wayf.renater.fr/"), 0)
                .is_some()
        );
        assert!(!jar.contains("exemple.com", "X"));

        jar.store(&page, "S=2; Domain=renater.fr; Path=/", 0);
        assert_eq!(
            jar.header_for(&url("https://renater.fr/"), 0).as_deref(),
            Some("S=2")
        );

        jar.store(&page, "S=; Domain=renater.fr; Path=/; Max-Age=0", 10);
        assert_eq!(jar.header_for(&url("https://renater.fr/"), 10), None);

        jar.store(&page, "E=1; Expires=Wed, 21 Oct 2015 07:28:00 GMT", 0);
        assert!(jar.to_stored(1_445_412_480).iter().all(|c| c.name != "E"));
    }

    #[test]
    fn chemin_par_defaut_et_hote_exact() {
        let mut jar = CookieJar::default();
        jar.store(
            &url("https://notes.iut.fr/services/doAuth.php"),
            "PHPSESSID=p",
            0,
        );
        assert!(
            jar.header_for(&url("https://notes.iut.fr/services/data.php"), 0)
                .is_some()
        );
        assert!(jar.header_for(&url("https://notes.iut.fr/"), 0).is_none());
        assert!(
            jar.header_for(&url("https://autre.notes.iut.fr/services/x"), 0)
                .is_none()
        );
    }
}
