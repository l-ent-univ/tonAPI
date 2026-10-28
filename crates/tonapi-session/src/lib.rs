//! Jeton de session chiffré, gardé par le client. Le serveur ne stocke rien.
//!
//! Format : `v1.<kid>.<base64url(nonce ‖ texte chiffré)>`, AES-256-GCM, données associées
//! `tonapi:v1:<kid>`. Le `kid` désigne la clé : on chiffre avec la première clé configurée et on
//! déchiffre avec n'importe laquelle, ce qui permet de changer de clé sans déconnecter personne.
//! Le mot de passe de l'étudiant n'entre jamais dans le jeton.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use tonapi_core::EstablishmentSource;

const VERSION: &str = "v1";
const NONCE_LEN: usize = 12;

/// Contenu d'une session.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionData {
    pub university: String,
    /// Identifiant de connexion (jamais le mot de passe).
    pub username: String,
    pub establishment: Option<EstablishmentPick>,
    /// Cookies utiles des services de l'université.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cookies: Vec<StoredCookie>,
    /// Jetons propres à un service (jeton Moodle, jeton de rafraîchissement ADE…), par nom.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tokens: BTreeMap<String, String>,
    /// Connexion en attente d'un second facteur : le jeton ne donne accès à aucune donnée.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
    /// Secondes Unix.
    pub issued_at: i64,
    /// Secondes Unix.
    pub expires_at: i64,
}

impl fmt::Debug for SessionData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionData")
            .field("university", &self.university)
            .field("establishment", &self.establishment)
            .field("cookies", &self.cookies.len())
            .field("tokens", &self.tokens.keys().collect::<Vec<_>>())
            .field("issued_at", &self.issued_at)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Composante retenue pour la session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EstablishmentPick {
    pub id: String,
    pub source: EstablishmentSource,
    pub confidence: f32,
}

/// Cookie conservé entre deux requêtes.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCookie {
    pub domain: String,
    pub path: String,
    pub name: String,
    pub value: String,
    /// Cookie réservé à l'hôte exact (pas d'attribut `Domain`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub host_only: bool,
    /// Secondes Unix ; `None` pour un cookie de session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<i64>,
}

impl fmt::Debug for StoredCookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}{}", self.name, self.domain, self.path)
    }
}

impl SessionData {
    /// Session après plusieurs fournisseurs lancés en parallèle, chacun sur une copie de `self`.
    /// Pour les cookies (domaine, chemin, nom) et les jetons, chaque ajout ou changement est
    /// repris ; une suppression l'est aussi, sauf si un autre fournisseur a écrit la même entrée.
    /// La composante choisie par un fournisseur (détection) remplace celle de départ.
    #[must_use]
    pub fn merged(&self, results: &[SessionData]) -> SessionData {
        type Key = (String, String, String);
        let key = |c: &StoredCookie| (c.domain.clone(), c.path.clone(), c.name.clone());
        let base: BTreeMap<Key, &StoredCookie> = self.cookies.iter().map(|c| (key(c), c)).collect();
        let mut cookies: BTreeMap<Key, StoredCookie> =
            self.cookies.iter().map(|c| (key(c), c.clone())).collect();
        // Ordre de départ, puis cookies nouveaux dans l'ordre où ils apparaissent.
        let mut order: Vec<Key> = self.cookies.iter().map(key).collect();
        let mut written: BTreeSet<Key> = BTreeSet::new();
        let mut tokens = self.tokens.clone();
        let mut written_tokens: BTreeSet<String> = BTreeSet::new();
        let mut merged = self.clone();

        // Les écritures d'abord, puis les suppressions : l'ordre des résultats ne compte pas.
        for result in results {
            for cookie in &result.cookies {
                let k = key(cookie);
                if base.get(&k).is_none_or(|old| *old != cookie) {
                    if !cookies.contains_key(&k) {
                        order.push(k.clone());
                    }
                    cookies.insert(k.clone(), cookie.clone());
                    written.insert(k);
                }
            }
            for (name, value) in &result.tokens {
                if self.tokens.get(name) != Some(value) {
                    tokens.insert(name.clone(), value.clone());
                    written_tokens.insert(name.clone());
                }
            }
            if result.establishment != self.establishment {
                merged.establishment.clone_from(&result.establishment);
            }
        }
        for result in results {
            let kept: BTreeSet<Key> = result.cookies.iter().map(key).collect();
            for k in base.keys() {
                if !kept.contains(k) && !written.contains(k) {
                    cookies.remove(k);
                }
            }
            for name in self.tokens.keys() {
                if !result.tokens.contains_key(name) && !written_tokens.contains(name) {
                    tokens.remove(name);
                }
            }
        }
        merged.cookies = order.iter().filter_map(|k| cookies.remove(k)).collect();
        merged.tokens = tokens;
        merged
    }
}

/// Refus d'un jeton ou d'une clé.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SessionError {
    #[error("jeton invalide")]
    Invalid,
    #[error("session expirée")]
    Expired,
    #[error("clé de session invalide : {0}")]
    BadKey(String),
}

/// Clés de session. La première chiffre ; toutes déchiffrent.
pub struct SessionKeys {
    keys: Vec<(String, Aes256Gcm)>,
}

impl fmt::Debug for SessionKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kids: Vec<&str> = self.keys.iter().map(|(kid, _)| kid.as_str()).collect();
        f.debug_struct("SessionKeys").field("kids", &kids).finish()
    }
}

impl SessionKeys {
    /// Lit `kid:base64,kid:base64…` (clés de 32 octets en base64 standard).
    pub fn parse(value: &str) -> Result<Self, SessionError> {
        let mut keys = Vec::new();
        for entry in value.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let (kid, b64) = entry
                .split_once(':')
                .ok_or_else(|| SessionError::BadKey(format!("`{entry}` : attendu kid:base64")))?;
            if !valid_kid(kid) {
                return Err(SessionError::BadKey(format!(
                    "identifiant `{kid}` : 1 à 16 caractères a-z, A-Z, 0-9, `_`"
                )));
            }
            let raw = STANDARD
                .decode(b64.trim())
                .map_err(|e| SessionError::BadKey(format!("{kid} : {e}")))?;
            let cipher = Aes256Gcm::new_from_slice(&raw).map_err(|_| {
                SessionError::BadKey(format!("{kid} : la clé doit faire 32 octets"))
            })?;
            if keys.iter().any(|(k, _)| k == kid) {
                return Err(SessionError::BadKey(format!("{kid} : en double")));
            }
            keys.push((kid.to_owned(), cipher));
        }
        if keys.is_empty() {
            return Err(SessionError::BadKey("aucune clé".to_owned()));
        }
        Ok(SessionKeys { keys })
    }

    /// Clé aléatoire pour le développement : les jetons ne survivent pas au redémarrage.
    pub fn ephemeral() -> Result<Self, SessionError> {
        Self::parse(&generate_key("dev")?)
    }

    /// Identifiant de la clé qui chiffre.
    #[must_use]
    pub fn current_kid(&self) -> &str {
        &self.keys[0].0
    }

    /// Chiffre une session.
    pub fn seal(&self, data: &SessionData) -> Result<String, SessionError> {
        let (kid, cipher) = &self.keys[0];
        let plaintext = serde_json::to_vec(data).map_err(|_| SessionError::Invalid)?;
        let mut nonce = [0_u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|e| SessionError::BadKey(e.to_string()))?;
        let aad = aad(kid);
        let ciphertext = cipher
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &plaintext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| SessionError::Invalid)?;
        let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        Ok(format!("{VERSION}.{kid}.{}", URL_SAFE_NO_PAD.encode(blob)))
    }

    /// Déchiffre un jeton et vérifie son expiration (`now` en secondes Unix).
    pub fn open(&self, token: &str, now: i64) -> Result<SessionData, SessionError> {
        self.open_with_grace(token, now, 0)
    }

    /// Comme [`SessionKeys::open`], mais accepte un jeton expiré depuis moins de `grace_secs`.
    /// Sert à reprendre les cookies d'une ancienne session (appareil mémorisé par le CAS).
    pub fn open_with_grace(
        &self,
        token: &str,
        now: i64,
        grace_secs: i64,
    ) -> Result<SessionData, SessionError> {
        let mut parts = token.splitn(3, '.');
        let (Some(VERSION), Some(kid), Some(body)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(SessionError::Invalid);
        };
        let (_, cipher) = self
            .keys
            .iter()
            .find(|(k, _)| k == kid)
            .ok_or(SessionError::Invalid)?;
        let blob = URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| SessionError::Invalid)?;
        if blob.len() <= NONCE_LEN {
            return Err(SessionError::Invalid);
        }
        let (nonce, ciphertext) = blob.split_at(NONCE_LEN);
        let nonce = Nonce::try_from(nonce).map_err(|_| SessionError::Invalid)?;
        let aad = aad(kid);
        let plaintext = cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: ciphertext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| SessionError::Invalid)?;
        let data: SessionData =
            serde_json::from_slice(&plaintext).map_err(|_| SessionError::Invalid)?;
        if data.expires_at + grace_secs <= now {
            return Err(SessionError::Expired);
        }
        Ok(data)
    }
}

/// Nouvelle clé au format `kid:base64`, pour `TONAPI_SESSION_KEYS`.
pub fn generate_key(kid: &str) -> Result<String, SessionError> {
    if !valid_kid(kid) {
        return Err(SessionError::BadKey(format!(
            "identifiant `{kid}` invalide"
        )));
    }
    let mut key = [0_u8; 32];
    getrandom::fill(&mut key).map_err(|e| SessionError::BadKey(e.to_string()))?;
    Ok(format!("{kid}:{}", STANDARD.encode(key)))
}

fn valid_kid(kid: &str) -> bool {
    (1..=16).contains(&kid.len()) && kid.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn aad(kid: &str) -> String {
    format!("tonapi:{VERSION}:{kid}")
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    fn cookie(domain: &str, name: &str, value: &str) -> StoredCookie {
        StoredCookie {
            domain: domain.to_owned(),
            path: "/".to_owned(),
            name: name.to_owned(),
            value: value.to_owned(),
            host_only: true,
            expires: None,
        }
    }

    fn session(cookies: Vec<StoredCookie>, tokens: &[(&str, &str)]) -> SessionData {
        SessionData {
            university: "u".to_owned(),
            username: "e".to_owned(),
            establishment: None,
            cookies,
            tokens: tokens
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            pending: false,
            issued_at: 0,
            expires_at: 1,
        }
    }

    fn value(s: &SessionData, name: &str) -> Option<String> {
        s.cookies
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.value.clone())
    }

    #[test]
    fn chaque_fournisseur_garde_ses_cookies() {
        let start = session(vec![cookie("cas", "TGC", "t0")], &[]);
        let mut a = start.clone();
        a.cookies.push(cookie("scodoc", "PHPSESSID", "s1"));
        let mut b = start.clone();
        b.cookies.push(cookie("moodle", "MoodleSession", "m1"));
        b.tokens.insert("moodle".to_owned(), "jeton".to_owned());
        let merged = start.merged(&[a, b]);
        assert_eq!(value(&merged, "TGC").as_deref(), Some("t0"));
        assert_eq!(value(&merged, "PHPSESSID").as_deref(), Some("s1"));
        assert_eq!(value(&merged, "MoodleSession").as_deref(), Some("m1"));
        assert_eq!(
            merged.tokens.get("moodle").map(String::as_str),
            Some("jeton")
        );
    }

    #[test]
    fn une_ecriture_l_emporte_sur_une_suppression() {
        let start = session(
            vec![
                cookie("cas", "TGC", "t0"),
                cookie("ade", "JSESSIONID", "j0"),
            ],
            &[("ade", "1,2")],
        );
        // A renouvelle le TGC ; B a vidé ses cookies et retiré un jeton refusé.
        let mut a = start.clone();
        a.cookies[0].value = "t1".to_owned();
        let mut b = start.clone();
        b.cookies.clear();
        b.tokens.clear();
        let merged = start.merged(&[b, a]);
        assert_eq!(value(&merged, "TGC").as_deref(), Some("t1"));
        assert_eq!(
            value(&merged, "JSESSIONID"),
            None,
            "supprimé par B, écrit par personne"
        );
        assert!(merged.tokens.is_empty());
        assert_eq!(start.merged(std::slice::from_ref(&start)), start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(expires_at: i64) -> SessionData {
        SessionData {
            university: "demo".into(),
            username: "demo".into(),
            establishment: None,
            cookies: vec![StoredCookie {
                domain: "cas.exemple.fr".into(),
                path: "/".into(),
                name: "TGC".into(),
                value: "secret".into(),
                host_only: true,
                expires: None,
            }],
            tokens: BTreeMap::new(),
            pending: false,
            issued_at: 0,
            expires_at,
        }
    }

    #[test]
    fn aller_retour() {
        let keys = SessionKeys::ephemeral().unwrap();
        let token = keys.seal(&data(100)).unwrap();
        assert!(token.starts_with("v1.dev."));
        assert!(!token.contains("secret"));
        assert_eq!(keys.open(&token, 50).unwrap(), data(100));
    }

    #[test]
    fn jeton_modifie_refuse() {
        let keys = SessionKeys::ephemeral().unwrap();
        let token = keys.seal(&data(100)).unwrap();
        let mut bytes = token.into_bytes();
        let last = bytes.len() - 2;
        bytes[last] = if bytes[last] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap();
        assert_eq!(keys.open(&tampered, 50), Err(SessionError::Invalid));
        assert_eq!(keys.open("n'importe quoi", 50), Err(SessionError::Invalid));
    }

    #[test]
    fn expiration() {
        let keys = SessionKeys::ephemeral().unwrap();
        let token = keys.seal(&data(100)).unwrap();
        assert_eq!(keys.open(&token, 100), Err(SessionError::Expired));
    }

    #[test]
    fn delai_de_grace() {
        let keys = SessionKeys::ephemeral().unwrap();
        let token = keys.seal(&data(100)).unwrap();
        assert!(keys.open_with_grace(&token, 150, 100).is_ok());
        assert_eq!(
            keys.open_with_grace(&token, 250, 100),
            Err(SessionError::Expired)
        );
    }

    #[test]
    fn rotation_de_cle() {
        let old = generate_key("k1").unwrap();
        let new = generate_key("k2").unwrap();
        let before = SessionKeys::parse(&old).unwrap();
        let token = before.seal(&data(100)).unwrap();

        let after = SessionKeys::parse(&format!("{new},{old}")).unwrap();
        assert_eq!(after.current_kid(), "k2");
        assert_eq!(after.open(&token, 50).unwrap(), data(100));
        assert!(after.seal(&data(100)).unwrap().starts_with("v1.k2."));

        let only_new = SessionKeys::parse(&new).unwrap();
        assert_eq!(only_new.open(&token, 50), Err(SessionError::Invalid));
    }

    #[test]
    fn cles_mal_formees() {
        assert!(SessionKeys::parse("").is_err());
        assert!(SessionKeys::parse("k1:AAAA").is_err());
        assert!(SessionKeys::parse("sans-deux-points").is_err());
        let k = generate_key("k1").unwrap();
        assert!(SessionKeys::parse(&format!("{k},{k}")).is_err());
    }

    #[test]
    fn debug_ne_montre_pas_les_secrets() {
        let text = format!("{:?}", data(1));
        assert!(!text.contains("secret"));
        let keys = SessionKeys::ephemeral().unwrap();
        assert_eq!(format!("{keys:?}"), r#"SessionKeys { kids: ["dev"] }"#);
    }
}
