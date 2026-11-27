//! Cache en mémoire des données **communes à tous les étudiants** d'un service : signatures GWT
//! d'ADE, noms de l'arbre des ressources, liens des portlets. Jamais de donnée personnelle ni
//! de cookie : une entrée doit être la même quel que soit le compte qui l'a lue. Quand ce qu'on
//! lit dépend d'un profil (l'arbre du portail ADE), le profil fait partie de la clé.
//!
//! Le cache vit dans le processus (rien sur disque) ; il est vidé au redémarrage et borné en taille.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Au-delà, les entrées expirées sont retirées, puis tout est vidé si cela ne suffit pas.
const MAX_ENTRIES: usize = 20_000;

pub(crate) struct Shared<V> {
    entries: LazyLock<Mutex<HashMap<String, (Instant, V)>>>,
    ttl: Duration,
}

impl<V: Clone> Shared<V> {
    pub(crate) const fn new(ttl: Duration) -> Self {
        Shared {
            entries: LazyLock::new(|| Mutex::new(HashMap::new())),
            ttl,
        }
    }

    pub(crate) fn get(&self, key: &str) -> Option<V> {
        let map = self.entries.lock().ok()?;
        let (at, value) = map.get(key)?;
        (at.elapsed() < self.ttl).then(|| value.clone())
    }

    pub(crate) fn put(&self, key: String, value: V) {
        let Ok(mut map) = self.entries.lock() else {
            return;
        };
        if map.len() >= MAX_ENTRIES {
            let ttl = self.ttl;
            map.retain(|_, (at, _)| at.elapsed() < ttl);
            if map.len() >= MAX_ENTRIES {
                map.clear();
            }
        }
        map.insert(key, (Instant::now(), value));
    }

    pub(crate) fn remove(&self, key: &str) {
        if let Ok(mut map) = self.entries.lock() {
            map.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entree_lue_puis_expiree() {
        static CACHE: Shared<u32> = Shared::new(Duration::from_millis(30));
        CACHE.put("a".into(), 1);
        assert_eq!(CACHE.get("a"), Some(1));
        assert_eq!(CACHE.get("b"), None);
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(CACHE.get("a"), None);
        CACHE.put("a".into(), 2);
        CACHE.remove("a");
        assert_eq!(CACHE.get("a"), None);
    }
}
