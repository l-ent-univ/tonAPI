//! Pot de cookies : ce qu'un navigateur garderait, sérialisable dans le jeton de session.
//!
//! Règles (RFC 6265, simplifiées) : `Domain` sans point initial ; sans `Domain`, le cookie reste
//! attaché à l'hôte exact ; chemin comparé par préfixe de segment (`/cas` couvre `/cas/login-…`) ;
//! `Max-Age` prime sur `Expires` ; un cookie expiré ou à `Max-Age` ≤ 0 est supprimé.

use chrono::DateTime;
use tonapi_session::StoredCookie;
use url::{Host, Url};

/// Cookies gardés par domaine : un hôte hostile ne fait pas grossir sans fin la session de
/// l'étudiant, qui voyage dans un en-tête.
const MAX_PER_DOMAIN: usize = 50;
/// Taille maximale d'un cookie (nom + valeur), comme les navigateurs.
const MAX_COOKIE_BYTES: usize = 4096;

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
        if name.is_empty() || name.len() + value.trim().len() > MAX_COOKIE_BYTES {
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
                    // `Domain=fr` couvrirait tout un TLD ; une adresse IP n'a pas de domaine parent.
                    if !domain.contains('.') || !matches!(url.host(), Some(Host::Domain(_))) {
                        return;
                    }
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
            cookie.expires = Some(now.saturating_add(seconds));
        }
        let alive = cookie.expires.is_none_or(|e| e > now);
        let existing = self.cookies.iter().position(|c| same_cookie(c, &cookie));
        match (existing, alive) {
            // Remplacé en place : un cookie reposé à l'identique ne déplace rien, la session reste
            // égale à l'originale et son jeton n'est pas renvoyé au client.
            (Some(index), true) => self.cookies[index] = cookie,
            (Some(_), false) => self.cookies.retain(|c| !same_cookie(c, &cookie)),
            (None, true) => {
                let same_domain = self.cookies.iter().filter(|c| c.domain == cookie.domain);
                if same_domain.count() < MAX_PER_DOMAIN {
                    self.cookies.push(cookie);
                }
            }
            (None, false) => {}
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

/// Même cookie au sens de la RFC 6265 : nom, domaine et chemin identiques.
fn same_cookie(a: &StoredCookie, b: &StoredCookie) -> bool {
    a.name == b.name && a.domain == b.domain && a.path == b.path
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
    fn domain_sans_point_ou_sur_une_adresse_ip_est_refuse() {
        let mut jar = CookieJar::default();
        let page = url("https://cas.univ.fr/login");
        jar.store(&page, "A=1; Domain=fr", 0);
        jar.store(&page, "B=1; Domain=.fr", 0);
        assert!(!jar.has_cookies_for("univ.fr"));
        assert!(!jar.has_cookies_for("autre.fr"));
        jar.store(&page, "C=1; Domain=univ.fr", 0);
        assert!(jar.contains("ent.univ.fr", "C"));

        let ip = url("https://192.0.2.7/login");
        jar.store(&ip, "D=1; Domain=192.0.2.7", 0);
        jar.store(&ip, "E=1; Domain=0.2.7", 0);
        assert!(!jar.has_cookies_for("192.0.2.7"));
        jar.store(&ip, "F=1", 0);
        assert!(jar.contains("192.0.2.7", "F"));
    }

    /// Un `Set-Cookie` identique à ce que la session contient déjà ne la modifie pas : sinon le
    /// jeton chiffré repartirait chez le client à presque chaque réponse.
    #[test]
    fn un_set_cookie_identique_laisse_la_session_egale() {
        let page = url("https://cas.univ.fr/cas/login");
        let mut jar = CookieJar::default();
        jar.store(&page, "TGC=tgt; Path=/cas; Secure; HttpOnly", 0);
        jar.store(&page, "JSESSIONID=s1; Path=/cas", 0);
        jar.store(
            &page,
            "lang=fr; Domain=univ.fr; Path=/; Expires=Wed, 21 Oct 2026 07:28:00 GMT",
            0,
        );
        let original = jar.to_stored(0);

        // Chaque cookie, premier, du milieu et dernier, est reposé à l'identique.
        for header in [
            "TGC=tgt; Path=/cas; Secure; HttpOnly",
            "JSESSIONID=s1; Path=/cas",
            "lang=fr; Domain=univ.fr; Path=/; Expires=Wed, 21 Oct 2026 07:28:00 GMT",
        ] {
            jar.store(&page, header, 0);
            assert!(jar.to_stored(0) == original, "{header}");
        }

        // Une valeur nouvelle remplace en place : même position, une seule entrée.
        jar.store(&page, "TGC=autre; Path=/cas", 0);
        let apres = jar.to_stored(0);
        assert_eq!(apres.len(), original.len());
        assert_eq!(apres[0].name, "TGC");
        assert_eq!(apres[0].value, "autre");
        assert_eq!(apres[1..], original[1..]);
    }

    #[test]
    fn au_plus_cinquante_cookies_par_domaine() {
        let mut jar = CookieJar::default();
        let page = url("https://hostile.example.test/");
        for i in 0..80 {
            jar.store(&page, &format!("c{i}=v"), 0);
        }
        assert_eq!(jar.to_stored(0).len(), MAX_PER_DOMAIN);
        let autre = url("https://sain.example.test/");
        jar.store(&autre, "ok=1", 0);
        assert!(jar.contains("sain.example.test", "ok"));
        // Remplacer ou supprimer un cookie existant reste possible au plafond.
        jar.store(&page, "c3=nouveau", 0);
        let header = jar.header_for(&page, 0).unwrap();
        assert_eq!(header.matches("c3=nouveau").count(), 1);
        jar.store(&page, "c4=; Max-Age=0", 0);
        assert!(!jar.contains("hostile.example.test", "c4"));
        jar.store(&page, "c80=v", 0);
        assert!(jar.contains("hostile.example.test", "c80"));
    }

    #[test]
    fn un_cookie_de_plus_de_4096_octets_est_ignore() {
        let mut jar = CookieJar::default();
        let page = url("https://notes.iut.fr/");
        let grand = "x".repeat(MAX_COOKIE_BYTES);
        jar.store(&page, &format!("gros={grand}"), 0);
        assert!(!jar.contains("notes.iut.fr", "gros"));
        let limite = "x".repeat(MAX_COOKIE_BYTES - "ok".len());
        jar.store(&page, &format!("ok={limite}"), 0);
        assert!(jar.contains("notes.iut.fr", "ok"));
    }

    #[test]
    fn un_max_age_gigantesque_ne_deborde_pas() {
        let mut jar = CookieJar::default();
        let page = url("https://notes.iut.fr/");
        jar.store(&page, "A=1; Max-Age=9223372036854775807", 1_000);
        jar.store(&page, "B=1; Max-Age=-9223372036854775808", 1_000);
        assert!(jar.contains("notes.iut.fr", "A"));
        assert!(!jar.contains("notes.iut.fr", "B"));
        assert_eq!(jar.to_stored(2_000).len(), 1);
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
