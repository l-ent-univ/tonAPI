//! Limite des échecs de connexion par compte : protège l'étudiant d'un verrouillage par son
//! université quand un client réessaie en boucle un mauvais mot de passe.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tonapi_core::{ErrorCode, Problem};

const WINDOW: Duration = Duration::from_secs(15 * 60);
const MAX_FAILURES: usize = 5;

#[derive(Default)]
pub struct LoginLimiter {
    failures: Mutex<HashMap<String, Vec<Instant>>>,
}

impl LoginLimiter {
    fn recent(&self, key: &str, now: Instant) -> usize {
        let mut map = self
            .failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.retain(|_, times| {
            times.retain(|t| now.duration_since(*t) < WINDOW);
            !times.is_empty()
        });
        map.get(key).map_or(0, Vec::len)
    }

    /// Refuse si le compte a déjà trop d'échecs récents.
    pub fn check(&self, key: &str) -> Result<(), Problem> {
        if self.recent(key, Instant::now()) >= MAX_FAILURES {
            return Err(Problem::new(ErrorCode::RateLimited).detail(format!(
                "{MAX_FAILURES} échecs de connexion en {} minutes : réessayez plus tard",
                WINDOW.as_secs() / 60
            )));
        }
        Ok(())
    }

    pub fn record_failure(&self, key: &str) {
        let mut map = self
            .failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.entry(key.to_owned()).or_default().push(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cinq_echecs_bloquent_le_compte_seulement() {
        let limiter = LoginLimiter::default();
        for _ in 0..MAX_FAILURES {
            assert!(limiter.check("u:a").is_ok());
            limiter.record_failure("u:a");
        }
        assert_eq!(
            limiter.check("u:a").unwrap_err().code,
            ErrorCode::RateLimited
        );
        assert!(limiter.check("u:b").is_ok());
    }
}
