//! Requêtes simultanées par compte sur les routes de données.
//!
//! Chaque requête de données peut occuper plusieurs des places réservées à une université
//! (`tonapi-http`, 32 par hôte et par processus) pendant des dizaines de secondes. Sans plafond,
//! un seul compte qui lance des dizaines d'appels en parallèle prendrait toutes les places et
//! bloquerait les autres étudiants de son université.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tonapi_core::{ErrorCode, Problem};
use tonapi_session::SessionData;

/// Requêtes de données en cours au plus pour un même compte (l'appel groupé en compte une).
const PER_ACCOUNT: usize = 4;
/// Au-delà, les comptes sans requête en cours sont retirés.
const MAX_ACCOUNTS: usize = 100_000;

#[derive(Default)]
pub struct AccountSlots {
    accounts: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl AccountSlots {
    /// Prend une place pour le compte de `session`, ou refuse tout de suite (`rate_limited`).
    pub fn acquire(&self, session: &SessionData) -> Result<OwnedSemaphorePermit, Problem> {
        let key = format!(
            "{}:{}",
            session.university,
            session.username.trim().to_lowercase()
        );
        let semaphore = {
            let mut map = self
                .accounts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if map.len() >= MAX_ACCOUNTS {
                map.retain(|_, s| Arc::strong_count(s) > 1 || s.available_permits() < PER_ACCOUNT);
            }
            map.entry(key)
                .or_insert_with(|| Arc::new(Semaphore::new(PER_ACCOUNT)))
                .clone()
        };
        semaphore.try_acquire_owned().map_err(|_| {
            Problem::new(ErrorCode::RateLimited).detail(format!(
                "{PER_ACCOUNT} requêtes en cours au plus pour un même compte : attendre leurs réponses"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn session(username: &str) -> SessionData {
        SessionData {
            university: "u".to_owned(),
            username: username.to_owned(),
            establishment: None,
            cookies: Vec::new(),
            tokens: BTreeMap::new(),
            pending: false,
            issued_at: 0,
            expires_at: 1,
        }
    }

    #[test]
    fn plafond_par_compte() {
        let slots = AccountSlots::default();
        let held: Vec<_> = (0..PER_ACCOUNT)
            .map(|_| slots.acquire(&session("Alice")).unwrap())
            .collect();
        assert_eq!(
            slots.acquire(&session(" alice ")).err().map(|p| p.code),
            Some(ErrorCode::RateLimited),
            "même compte, autre casse"
        );
        assert!(slots.acquire(&session("bob")).is_ok());
        drop(held);
        assert!(slots.acquire(&session("alice")).is_ok());
    }
}
