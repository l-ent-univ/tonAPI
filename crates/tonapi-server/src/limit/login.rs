//! Limite des échecs de connexion par compte : protège l'étudiant d'un verrouillage par son
//! université quand un client réessaie en boucle un mauvais mot de passe.
//!
//! Un essai réserve sa place **avant** d'appeler l'université : des requêtes parallèles ne
//! passent pas toutes le contrôle avant que le premier échec soit compté. La table est propre à
//! chaque processus ; avec plusieurs instances, la limite réelle est multipliée par leur nombre.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tonapi_core::{ErrorCode, Problem};

const WINDOW: Duration = Duration::from_secs(15 * 60);
const MAX_FAILURES: usize = 5;
/// Au-delà, les comptes sans échec récent ni essai en cours sont retirés de la table.
const MAX_KEYS: usize = 100_000;

#[derive(Default)]
struct Entry {
    failures: Vec<Instant>,
    in_flight: usize,
}

#[derive(Default)]
pub struct LoginLimiter {
    entries: Mutex<HashMap<String, Entry>>,
    /// Dernière purge complète : au plus une toutes les 10 s, même table pleine.
    last_purge: Mutex<Option<Instant>>,
}

const PURGE_EVERY: Duration = Duration::from_secs(10);

/// Essai en cours ; libère sa place quand il est abandonné. [`Attempt::failed`] compte l'échec.
pub struct Attempt<'a> {
    limiter: &'a LoginLimiter,
    key: String,
}

impl Attempt<'_> {
    /// L'université a refusé les identifiants ou le code.
    pub fn failed(self) {
        self.limiter.with(|map| {
            map.entry(self.key.clone())
                .or_default()
                .failures
                .push(Instant::now());
        });
    }
}

impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        self.limiter.with(|map| {
            if let Some(entry) = map.get_mut(&self.key) {
                entry.in_flight = entry.in_flight.saturating_sub(1);
            }
        });
    }
}

/// Clé de limite : préfixe, université et identifiant normalisé (espaces retirés, minuscules).
#[must_use]
pub fn key(prefix: &str, university: &str, username: &str) -> String {
    format!("{prefix}:{university}:{}", username.trim().to_lowercase())
}

impl LoginLimiter {
    fn with<T>(&self, f: impl FnOnce(&mut HashMap<String, Entry>) -> T) -> T {
        let mut map = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut map)
    }

    fn purge_due(&self, now: Instant) -> bool {
        let mut last = self
            .last_purge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let due = last.is_none_or(|t| now.duration_since(t) >= PURGE_EVERY);
        if due {
            *last = Some(now);
        }
        due
    }

    /// Réserve un essai, ou refuse si le compte a déjà trop d'échecs et d'essais en cours.
    pub fn begin(&self, key: &str) -> Result<Attempt<'_>, Problem> {
        let now = Instant::now();
        let (allowed, entries) = self.with(|map| {
            if map.len() >= MAX_KEYS && self.purge_due(now) {
                map.retain(|_, e| {
                    e.failures.retain(|t| now.duration_since(*t) < WINDOW);
                    e.in_flight > 0 || !e.failures.is_empty()
                });
            }
            let entry = map.entry(key.to_owned()).or_default();
            entry.failures.retain(|t| now.duration_since(*t) < WINDOW);
            let allowed = entry.failures.len() + entry.in_flight < MAX_FAILURES;
            if allowed {
                entry.in_flight += 1;
            }
            (allowed, map.len())
        });
        tonapi_metrics::set(
            "tonapi_limiter_entries",
            &[("limiter", "login")],
            i64::try_from(entries).unwrap_or(i64::MAX),
        );
        if !allowed {
            tonapi_metrics::inc("tonapi_rate_limited_total", &[("limiter", "login")]);
            return Err(Problem::new(ErrorCode::RateLimited).detail(format!(
                "{MAX_FAILURES} échecs de connexion en {} minutes : réessayez plus tard",
                WINDOW.as_secs() / 60
            )));
        }
        Ok(Attempt {
            limiter: self,
            key: key.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cinq_echecs_bloquent_le_compte_seulement() {
        let limiter = LoginLimiter::default();
        for _ in 0..MAX_FAILURES {
            limiter.begin("u:a").unwrap().failed();
        }
        assert_eq!(
            limiter.begin("u:a").err().unwrap().code,
            ErrorCode::RateLimited
        );
        assert!(limiter.begin("u:b").is_ok());
    }

    #[test]
    fn essais_paralleles_comptes_avant_la_reponse() {
        let limiter = LoginLimiter::default();
        let running: Vec<_> = (0..MAX_FAILURES)
            .map(|_| limiter.begin("u:a").unwrap())
            .collect();
        assert!(limiter.begin("u:a").is_err(), "le 6e attend une place");
        drop(running);
        assert!(
            limiter.begin("u:a").is_ok(),
            "un essai réussi libère sa place"
        );
    }

    #[test]
    fn identifiant_normalise() {
        assert_eq!(key("login", "u", " Alice "), key("login", "u", "alice"));
    }
}
