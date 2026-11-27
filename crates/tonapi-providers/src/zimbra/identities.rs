//! Identités du compte (« De : ») et signatures que chacune nomme, lues dans `GetIdentities` et
//! `GetInfo`. Repris de `server/mailSend.js` de Tafac (`readZimbraAttrs`,
//! `normalizeZimbraIdentities`, `readIdentitySignatureIds`, `pickIdentity`).
//!
//! Une identité n'est gardée que si son adresse est l'une de celles que Zimbra laisse le compte
//! utiliser (son nom, ses alias, `zimbraAllowFromAddress`), vérifiée ici comme Zimbra le ferait à
//! l'envoi.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::Value;

use crate::mail::address::{is_valid_address, sanitize_display_name};
use crate::mail::text::trim_js;

use super::listing::js_string;

/// Identités gardées au plus.
pub const MAX_IDENTITIES: usize = 20;
/// Identités lues au plus dans une réponse (le reste est ignoré).
const MAX_IDENTITIES_READ: usize = 400;
/// Valeurs gardées au plus pour un même attribut (alias…).
const MAX_ATTR_VALUES: usize = 200;
/// Identifiant de l'identité de repli, l'adresse du compte.
pub const DEFAULT_IDENTITY_ID: &str = "default";

/// Une identité d'envoi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Identifiant (`^[A-Za-z0-9:._-]{1,64}$`, ou `default`).
    pub id: String,
    /// Nom affiché, nettoyé (peut être vide).
    pub name: String,
    /// Adresse, valide et permise au compte.
    pub email: String,
    /// L'identité par défaut du compte.
    pub is_default: bool,
}

/// Les signatures du webmail qu'une identité nomme.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentitySignatureIds {
    /// `zimbraPrefDefaultSignatureId`.
    pub signature_id: Option<String>,
    /// `zimbraPrefForwardReplySignatureId`.
    pub reply_signature_id: Option<String>,
}

/// Identifiant d'identité : `^[A-Za-z0-9:._-]{1,64}$`.
#[must_use]
pub fn is_valid_identity_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'.' | b'_' | b'-'))
}

/// Identifiant de signature : `^[A-Za-z0-9-]{1,64}$`.
#[must_use]
pub fn is_valid_signature_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Attributs d'un élément Zimbra, écrits `{_attrs: {nom: valeur}}` ou `{a: [{n, _content}]}` :
/// une table nom → valeurs (une liste pour un attribut répété). Seuls les noms demandés ensuite
/// sont lus : un nom comme `__proto__` n'a aucun effet.
type Attrs = BTreeMap<String, Vec<String>>;

/// `String(v)` d'une valeur d'attribut, comme JavaScript (`null` dans une liste donne `"null"`).
fn attr_text(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Array(_) | Value::Object(_) => "[object Object]".to_owned(),
        other => js_string(other).unwrap_or_default(),
    }
}

/// `readZimbraAttrs`.
fn read_attrs(element: Option<&Value>) -> Attrs {
    let mut attrs = Attrs::new();
    let mut add = |name: &str, value: &Value| {
        if value.is_null() {
            return;
        }
        let entry = attrs.entry(name.to_owned()).or_default();
        let values: Vec<&Value> = match value {
            Value::Array(list) => list.iter().collect(),
            other => vec![other],
        };
        for value in values {
            if entry.len() < MAX_ATTR_VALUES {
                entry.push(attr_text(value));
            }
        }
    };
    let Some(element) = element else {
        return attrs;
    };
    if let Some(map) = element.get("_attrs").and_then(Value::as_object) {
        for (name, value) in map {
            add(name, value);
        }
    }
    if let Some(list) = element.get("a").and_then(Value::as_array) {
        for entry in list {
            if let (Some(name), Some(value)) = (
                entry.get("n").and_then(Value::as_str),
                entry.get("_content"),
            ) {
                add(name, value);
            }
        }
    }
    attrs
}

/// `firstAttr` : la première valeur non blanche de l'attribut, sans espaces de bord ; vide sinon.
fn first_attr(attrs: &Attrs, name: &str) -> String {
    attrs
        .get(name)
        .and_then(|values| values.iter().find(|value| !trim_js(value).is_empty()))
        .map(|value| trim_js(value).to_owned())
        .unwrap_or_default()
}

/// L'identifiant d'une identité : `id`, sinon `zimbraPrefIdentityId`.
fn identity_id(identity: &Value, attrs: &Attrs) -> String {
    match identity.get("id") {
        None | Some(Value::Null) => first_attr(attrs, "zimbraPrefIdentityId"),
        Some(value) => js_string(value).unwrap_or_default(),
    }
}

/// La liste `identity` d'une `GetIdentitiesResponse` (une liste seulement).
fn identity_list(response: Option<&Value>) -> &[Value] {
    response
        .and_then(|response| response.get("identity"))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// `normalizeZimbraIdentities` : les identités d'envoi du compte d'après `GetIdentitiesResponse`
/// (absente si la requête a échoué) et `GetInfoResponse` (sections mbox et attrs). Une adresse
/// valide et permise au compte, un identifiant valide et unique, le nom nettoyé, l'identité par
/// défaut d'abord, 20 au plus. Sans identité utilisable, l'adresse du compte
/// (`{id: "default"}`) ; sans elle non plus, rien.
#[must_use]
pub fn normalize_identities(identities: Option<&Value>, info: Option<&Value>) -> Vec<Identity> {
    let info_attrs = read_attrs(info.and_then(|info| info.get("attrs")));
    let account_name = info
        .and_then(|info| info.get("name"))
        .and_then(js_string)
        .map(|name| trim_js(&name).to_owned())
        .unwrap_or_default();
    let display = {
        let display_name = first_attr(&info_attrs, "displayName");
        let written = if display_name.is_empty() {
            [
                first_attr(&info_attrs, "givenName"),
                first_attr(&info_attrs, "sn"),
            ]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
        } else {
            display_name
        };
        sanitize_display_name(&written, &account_name)
    };
    let allowed: HashSet<String> = std::iter::once(account_name.clone())
        .chain(
            info_attrs
                .get("zimbraMailAlias")
                .into_iter()
                .flatten()
                .cloned(),
        )
        .chain(
            info_attrs
                .get("zimbraAllowFromAddress")
                .into_iter()
                .flatten()
                .cloned(),
        )
        .map(|address| trim_js(&address).to_lowercase())
        .filter(|address| !address.is_empty())
        .collect();

    let mut result: Vec<Identity> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for identity in identity_list(identities).iter().take(MAX_IDENTITIES_READ) {
        let attrs = read_attrs(Some(identity));
        let id = identity_id(identity, &attrs);
        let is_default = identity.get("name").and_then(Value::as_str) == Some("DEFAULT")
            || first_attr(&attrs, "zimbraPrefIdentityName") == "DEFAULT";
        let from_address = first_attr(&attrs, "zimbraPrefFromAddress");
        let email = if !from_address.is_empty() {
            from_address
        } else if is_default {
            account_name.clone()
        } else {
            String::new()
        };
        if !is_valid_identity_id(&id)
            || seen.contains(&id)
            || !is_valid_address(&email)
            || !allowed.contains(&email.to_lowercase())
        {
            continue;
        }
        seen.insert(id.clone());
        let own_name = sanitize_display_name(&first_attr(&attrs, "zimbraPrefFromDisplay"), &email);
        result.push(Identity {
            id,
            name: if own_name.is_empty() {
                display.clone()
            } else {
                own_name
            },
            email,
            is_default,
        });
    }
    // Tri stable : l'identité par défaut d'abord, les autres dans l'ordre reçu.
    result.sort_by_key(|identity| !identity.is_default);
    if result.is_empty() && is_valid_address(&account_name) {
        result.push(Identity {
            id: DEFAULT_IDENTITY_ID.to_owned(),
            name: display,
            email: account_name,
            is_default: true,
        });
    }
    result.truncate(MAX_IDENTITIES);
    result
}

/// `readIdentitySignatureIds` : les signatures que nomme chaque identité de la
/// `GetIdentitiesResponse`, par identifiant d'identité (lu comme [`normalize_identities`]). Un
/// identifiant de signature d'une autre forme que `^[A-Za-z0-9-]{1,64}$` devient `None` ; 40
/// identités lues au plus.
#[must_use]
pub fn read_identity_signature_ids(
    identities: Option<&Value>,
) -> HashMap<String, IdentitySignatureIds> {
    let mut ids = HashMap::new();
    for identity in identity_list(identities).iter().take(MAX_IDENTITIES * 2) {
        let attrs = read_attrs(Some(identity));
        let id = identity_id(identity, &attrs);
        if !is_valid_identity_id(&id) || ids.contains_key(&id) {
            continue;
        }
        let read = |name: &str| {
            let value = first_attr(&attrs, name);
            is_valid_signature_id(&value).then_some(value)
        };
        ids.insert(
            id,
            IdentitySignatureIds {
                signature_id: read("zimbraPrefDefaultSignatureId"),
                reply_signature_id: read("zimbraPrefForwardReplySignatureId"),
            },
        );
    }
    ids
}

/// `pickIdentity` : l'identité qu'un envoi utilise. Celle demandée, de la liste ; la première
/// (par défaut) quand aucune n'est demandée (`None` ou vide) ; `None` pour un identifiant que le
/// compte n'a pas.
#[must_use]
pub fn pick_identity<'a>(
    identities: &'a [Identity],
    identity_id: Option<&str>,
) -> Option<&'a Identity> {
    match identity_id {
        None | Some("") => identities.first(),
        Some(id) => identities.iter().find(|identity| identity.id == id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn identities() -> Vec<Identity> {
        normalize_identities(
            Some(&json!({
                "identity": [
                    { "name": "Perso", "id": "persona-1", "_attrs": { "zimbraPrefFromAddress": "camille@gmail.com", "zimbraPrefFromDisplay": "Camille" } },
                    { "name": "Alias", "id": "persona-2", "_attrs": { "zimbraPrefFromAddress": "c.martin@etudiant.univ-rennes.fr", "zimbraPrefFromDisplay": "Camille M." } },
                    { "name": "DEFAULT", "id": "acct-1", "_attrs": { "zimbraPrefFromDisplay": "Camille Martin" } },
                    { "name": "Bad", "id": "bad id", "_attrs": { "zimbraPrefFromAddress": "camille.martin@univ-rennes.fr" } },
                ],
            })),
            Some(&json!({
                "name": "camille.martin@univ-rennes.fr",
                "attrs": { "_attrs": { "displayName": "Camille Martin", "zimbraMailAlias": ["c.martin@etudiant.univ-rennes.fr"] } },
            })),
        )
    }

    fn identity(id: &str, name: &str, email: &str, is_default: bool) -> Identity {
        Identity {
            id: id.into(),
            name: name.into(),
            email: email.into(),
            is_default,
        }
    }

    #[test]
    fn seulement_les_adresses_permises_par_defaut_d_abord() {
        let list = identities();
        assert_eq!(
            list,
            [
                identity(
                    "acct-1",
                    "Camille Martin",
                    "camille.martin@univ-rennes.fr",
                    true
                ),
                identity(
                    "persona-2",
                    "Camille M.",
                    "c.martin@etudiant.univ-rennes.fr",
                    false
                ),
            ]
        );
        // La page ne peut pas en choisir une à elle.
        assert_eq!(pick_identity(&list, Some("persona-1")), None);
        assert_eq!(
            pick_identity(&list, Some("acct-1")).map(|i| i.email.as_str()),
            Some("camille.martin@univ-rennes.fr")
        );
        assert_eq!(
            pick_identity(&list, None).map(|i| i.id.as_str()),
            Some("acct-1")
        );
        assert_eq!(
            pick_identity(&list, Some("")).map(|i| i.id.as_str()),
            Some("acct-1")
        );
        assert_eq!(pick_identity(&[], None), None);
    }

    #[test]
    fn attributs_en_liste_et_repli() {
        let list = normalize_identities(
            Some(
                &json!({ "identity": [{ "name": "DEFAULT", "id": "a", "a": [{ "n": "zimbraPrefFromAddress", "_content": "shared@univ.fr" }] }] }),
            ),
            Some(
                &json!({ "name": "me@univ.fr", "attrs": { "a": [{ "n": "zimbraAllowFromAddress", "_content": "shared@univ.fr" }] } }),
            ),
        );
        assert_eq!(
            list.iter().map(|i| i.email.as_str()).collect::<Vec<_>>(),
            ["shared@univ.fr"]
        );
        assert_eq!(
            normalize_identities(None, Some(&json!({ "name": "me@univ.fr", "attrs": {} }))),
            [identity("default", "", "me@univ.fr", true)]
        );
        assert!(normalize_identities(None, Some(&json!({}))).is_empty());
        assert!(normalize_identities(None, None).is_empty());
        // Une identité seule (pas une liste) n'est pas lue, comme dans Tafac.
        assert_eq!(
            normalize_identities(
                Some(&json!({ "identity": { "name": "DEFAULT", "id": "x" } })),
                Some(&json!({ "name": "me@univ.fr" }))
            )[0]
            .id,
            "default"
        );
    }

    #[test]
    fn nom_du_compte_et_identifiants() {
        // Prénom et nom quand `displayName` manque ; identifiant numérique ; doublon ignoré.
        let list = normalize_identities(
            Some(&json!({ "identity": [
                { "name": "DEFAULT", "id": 7 },
                { "name": "DEFAULT", "id": "7" },
                { "name": "x", "_attrs": { "zimbraPrefIdentityId": "from-attr", "zimbraPrefFromAddress": "ME@univ.fr" } },
                { "name": "y", "id": "٣", "_attrs": { "zimbraPrefFromAddress": "me@univ.fr" } },
                { "name": "z", "id": "__proto__", "_attrs": { "zimbraPrefFromAddress": "evil@x.fr" } },
            ] })),
            Some(
                &json!({ "name": "me@univ.fr", "attrs": { "_attrs": { "givenName": "Camille", "sn": "Demo", "__proto__": ["evil@x.fr"] } } }),
            ),
        );
        assert_eq!(
            list,
            [
                identity("7", "Camille Demo", "me@univ.fr", true),
                identity("from-attr", "Camille Demo", "ME@univ.fr", false),
            ]
        );
    }

    #[test]
    fn nom_affiche_qui_ressemble_a_une_adresse() {
        let list = normalize_identities(
            Some(
                &json!({ "identity": [{ "name": "DEFAULT", "id": "a", "_attrs": { "zimbraPrefFromDisplay": "boss＠univ.fr" } }] }),
            ),
            Some(&json!({ "name": "me@univ.fr", "attrs": { "_attrs": { "displayName": "Moi" } } })),
        );
        assert_eq!(list[0].name, "Moi");
    }

    #[test]
    fn vingt_identites_au_plus() {
        let many: Vec<Value> = (0..100)
            .map(|i| json!({ "id": format!("id-{i}"), "_attrs": { "zimbraPrefFromAddress": "me@univ.fr" } }))
            .collect();
        let list = normalize_identities(
            Some(&json!({ "identity": many })),
            Some(&json!({ "name": "me@univ.fr" })),
        );
        assert_eq!(list.len(), MAX_IDENTITIES);
    }

    #[test]
    fn signatures_des_identites() {
        let map = read_identity_signature_ids(Some(&json!({
            "identity": [
                { "id": "acc-1", "name": "DEFAULT", "_attrs": { "zimbraPrefDefaultSignatureId": "sig-1", "zimbraPrefForwardReplySignatureId": "sig-2" } },
                { "id": "per-1", "name": "Perso", "a": [{ "n": "zimbraPrefDefaultSignatureId", "_content": "11111111-1111-1111-1111-111111111111" }, { "n": "zimbraPrefForwardReplySignatureId", "_content": "<bad>" }] },
                { "id": "acc-1", "_attrs": { "zimbraPrefDefaultSignatureId": "other" } },
                { "id": "uni", "_attrs": { "zimbraPrefDefaultSignatureId": "٣" } },
            ],
        })));
        assert_eq!(
            map.get("acc-1"),
            Some(&IdentitySignatureIds {
                signature_id: Some("sig-1".into()),
                reply_signature_id: Some("sig-2".into()),
            })
        );
        assert_eq!(
            map.get("per-1"),
            Some(&IdentitySignatureIds {
                signature_id: Some("11111111-1111-1111-1111-111111111111".into()),
                reply_signature_id: None,
            })
        );
        assert_eq!(
            map.get("uni").and_then(|ids| ids.signature_id.clone()),
            None
        );
        assert!(read_identity_signature_ids(None).is_empty());
        let many: Vec<Value> = (0..100).map(|i| json!({ "id": format!("i{i}") })).collect();
        assert_eq!(
            read_identity_signature_ids(Some(&json!({ "identity": many }))).len(),
            MAX_IDENTITIES * 2
        );
    }

    #[test]
    fn identifiants_valides() {
        assert!(is_valid_identity_id("a1:b._-"));
        for bad in ["", "a b", "é", "٣", &"x".repeat(65)] {
            assert!(!is_valid_identity_id(bad), "{bad}");
        }
        assert!(is_valid_signature_id("a1b2-c3"));
        for bad in ["", "a:b", "<x>", "٣", &"x".repeat(65)] {
            assert!(!is_valid_signature_id(bad), "{bad}");
        }
    }
}
