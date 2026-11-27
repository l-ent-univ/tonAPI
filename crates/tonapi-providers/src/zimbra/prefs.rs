//! Préférences du webmail que la page suit (`/mail/prefs`) et données du compte lues dans
//! `GetInfo` : délai avant de marquer un mail lu, limites des pièces jointes, adresse du compte.
//! Repris de `server/mailFolders.js` (`parseZimbraMarkReadPref`), `server/mailContext.js`
//! (`readContextAttachmentLimits`) et `server/entAuthApp.js` (`fetchMailAddress`, la route
//! `/mail/prefs`).

use serde_json::{Value, json};

use crate::mail::attachments::{AttachmentLimits, derive_attachment_limits};
use crate::mail::text::{is_js_space, trim_js};

use super::identities::Identity;
use super::listing::{as_array, js_number, js_parse_int, js_string};

/// Le délai avant de marquer un mail lu ne dépasse pas une heure.
pub const MARK_READ_MAX_DELAY: i64 = 3600;
const MARK_READ_PREF: &str = "zimbraPrefMarkMsgRead";

/// Le corps de `GetPrefsRequest` (urn:zimbraAccount) qui lit `zimbraPrefMarkMsgRead`.
#[must_use]
pub fn mark_read_pref_request() -> Value {
    json!({ "pref": [{ "name": MARK_READ_PREF }] })
}

/// `parseZimbraMarkReadPref` : `zimbraPrefMarkMsgRead` d'une `GetPrefsResponse`, en secondes
/// avant qu'un mail ouvert soit marqué lu : -1 jamais, 0 tout de suite, une heure au plus. Lu dans
/// `_attrs` (`{nom: valeur}`) ou dans la liste `pref` (`[{name, _content}]`) ; illisible : 0, le
/// défaut du webmail. Chiffres ASCII seulement (`parseInt`).
#[must_use]
pub fn parse_mark_read_pref(response: &Value) -> i64 {
    let mut raw = response
        .get("_attrs")
        .and_then(|attrs| attrs.get(MARK_READ_PREF));
    if raw.is_none() {
        let pref = as_array(response.get("pref"))
            .into_iter()
            .find(|entry| entry.get("name").and_then(Value::as_str) == Some(MARK_READ_PREF));
        raw = pref.and_then(|pref| {
            pref.get("_content")
                .filter(|value| !value.is_null())
                .or_else(|| pref.get("value"))
        });
    }
    if let Some(Value::Array(list)) = raw {
        raw = list.first();
    }
    let text = raw.and_then(js_string).unwrap_or_default();
    match js_parse_int(&text) {
        None => 0,
        Some(value) if value < 0.0 => -1,
        #[allow(
            clippy::cast_possible_truncation,
            reason = "valeur positive bornée à une heure"
        )]
        Some(value) => value.min(3600.0 /* MARK_READ_MAX_DELAY */) as i64,
    }
}

/// Le corps de `GetInfoRequest` (urn:zimbraAccount) : les sections du compte et de ses attributs
/// (adresse, alias, limite des pièces jointes).
#[must_use]
pub fn info_request() -> Value {
    json!({ "sections": "mbox,attrs" })
}

/// Les limites des pièces jointes d'après `attSizeLimit` d'une `GetInfoResponse`
/// (`deriveAttachmentLimits`) ; sans réponse, celles de repli.
#[must_use]
pub fn attachment_limits_from_info(info: Option<&Value>) -> AttachmentLimits {
    derive_attachment_limits(
        info.and_then(|info| info.get("attSizeLimit"))
            .and_then(js_number),
    )
}

/// Ce que `GetInfo` a donné pour les limites d'un contexte du webmail.
#[derive(Debug, Clone, Copy)]
pub enum InfoRead<'a> {
    /// La `GetInfoResponse`.
    Read(&'a Value),
    /// Échec pour une autre raison que la session (une session refusée se propage avant).
    Failed,
}

/// Les limites retenues pour une requête, et si elles se gardent sur le contexte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextLimits {
    /// Limites à appliquer.
    pub limits: AttachmentLimits,
    /// À garder sur le contexte (lues) ; sinon valables pour cette requête seulement.
    pub keep: bool,
}

/// `readContextAttachmentLimits`, sans le réseau : celles déjà gardées sur le contexte ; sinon
/// celles de `GetInfo`, gardées ; un `GetInfo` en échec donne celles de repli pour cette requête
/// seulement (gardées, elles vaudraient pour toute la connexion). Quand `kept` est connu,
/// l'appelant n'a pas à lire `GetInfo` : `info` n'est alors pas consulté.
#[must_use]
pub fn context_attachment_limits(
    kept: Option<AttachmentLimits>,
    info: InfoRead<'_>,
) -> ContextLimits {
    if let Some(limits) = kept {
        return ContextLimits { limits, keep: true };
    }
    match info {
        InfoRead::Read(info) => ContextLimits {
            limits: attachment_limits_from_info(Some(info)),
            keep: true,
        },
        InfoRead::Failed => ContextLimits {
            limits: attachment_limits_from_info(None),
            keep: false,
        },
    }
}

/// L'adresse du compte telle que le webmail la connaît (`name` de `GetInfo`, sans espaces de
/// bord) si elle a la forme `^[^\s@]+@[^\s@]+\.[^\s@]+$` ; sinon `None`.
#[must_use]
pub fn account_address(info: &Value) -> Option<String> {
    let name = trim_js(info.get("name").and_then(Value::as_str)?);
    let (local, domain) = name.split_once('@')?;
    let plain = |part: &str| !part.is_empty() && !part.contains('@') && !part.contains(is_js_space);
    let dotted = domain
        .char_indices()
        .any(|(index, c)| c == '.' && index > 0 && index + 1 < domain.len());
    (plain(local) && plain(domain) && dotted).then(|| name.to_owned())
}

/// La réponse de `/mail/prefs` : `{markRead, attachments: {maxFileBytes, maxTotalBytes},
/// identities: [{id, name, email}]}`, l'identité par défaut d'abord.
#[must_use]
pub fn prefs_answer(mark_read: i64, limits: &AttachmentLimits, identities: &[Identity]) -> Value {
    json!({
        "markRead": mark_read,
        "attachments": {
            "maxFileBytes": limits.max_file_bytes,
            "maxTotalBytes": limits.max_total_bytes,
        },
        "identities": identities
            .iter()
            .map(|identity| json!({ "id": identity.id, "name": identity.name, "email": identity.email }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::attachments::FALLBACK_MESSAGE_LIMIT_BYTES;

    #[test]
    fn delai_avant_lu_des_deux_formes() {
        let attrs = |value: Value| json!({ "_attrs": { "zimbraPrefMarkMsgRead": value } });
        assert_eq!(parse_mark_read_pref(&attrs(json!("0"))), 0);
        assert_eq!(parse_mark_read_pref(&attrs(json!("-1"))), -1);
        assert_eq!(parse_mark_read_pref(&attrs(json!("5"))), 5);
        assert_eq!(parse_mark_read_pref(&attrs(json!(["3"]))), 3);
        assert_eq!(parse_mark_read_pref(&attrs(json!(7))), 7);
        assert_eq!(
            parse_mark_read_pref(
                &json!({ "pref": [{ "name": "zimbraPrefMarkMsgRead", "_content": "10" }] })
            ),
            10
        );
        assert_eq!(
            parse_mark_read_pref(
                &json!({ "pref": { "name": "zimbraPrefMarkMsgRead", "_content": "-1" } })
            ),
            -1
        );
        assert_eq!(
            parse_mark_read_pref(
                &json!({ "pref": [{ "name": "zimbraPrefMarkMsgRead", "value": "12" }] })
            ),
            12
        );
        assert_eq!(parse_mark_read_pref(&attrs(json!("999999"))), 3600);
        assert_eq!(
            parse_mark_read_pref(&attrs(json!("-999999999999999999999"))),
            -1
        );
        assert_eq!(parse_mark_read_pref(&json!({})), 0);
        assert_eq!(parse_mark_read_pref(&Value::Null), 0);
        assert_eq!(parse_mark_read_pref(&attrs(json!("soon"))), 0);
        assert_eq!(parse_mark_read_pref(&attrs(json!(null))), 0);
        // Chiffres Unicode : illisible.
        assert_eq!(parse_mark_read_pref(&attrs(json!("٥"))), 0);
        // `__proto__` n'est pas le nom de la préférence.
        assert_eq!(
            parse_mark_read_pref(
                &json!({ "_attrs": { "__proto__": "5" }, "pref": [{ "name": "__proto__", "_content": "5" }] })
            ),
            0
        );
        assert_eq!(
            mark_read_pref_request(),
            json!({ "pref": [{ "name": "zimbraPrefMarkMsgRead" }] })
        );
    }

    #[test]
    fn limites_de_get_info() {
        let fallback = attachment_limits_from_info(None);
        assert!(!fallback.known);
        assert_eq!(fallback.message_limit_bytes, FALLBACK_MESSAGE_LIMIT_BYTES);
        let known = attachment_limits_from_info(Some(&json!({ "attSizeLimit": 10_485_760 })));
        assert!(known.known);
        assert_eq!(known.message_limit_bytes, 10_485_760);
        let text = attachment_limits_from_info(Some(&json!({ "attSizeLimit": "10485760" })));
        assert_eq!(text, known);
        assert!(!attachment_limits_from_info(Some(&json!({ "attSizeLimit": "-1" }))).known);
        assert!(!attachment_limits_from_info(Some(&json!({ "attSizeLimit": "١٠٠٠٠٠٠٠" }))).known);
        assert_eq!(info_request(), json!({ "sections": "mbox,attrs" }));
    }

    #[test]
    fn limites_gardees_ou_non() {
        let info = json!({ "attSizeLimit": 10_485_760 });
        let read = context_attachment_limits(None, InfoRead::Read(&info));
        assert!(read.keep && read.limits.known);
        let failed = context_attachment_limits(None, InfoRead::Failed);
        assert!(!failed.keep && !failed.limits.known);
        let kept = context_attachment_limits(Some(read.limits), InfoRead::Failed);
        assert!(kept.keep);
        assert_eq!(kept.limits, read.limits);
    }

    #[test]
    fn adresse_du_compte() {
        assert_eq!(
            account_address(&json!({ "name": " camille.demo@example.org " })).as_deref(),
            Some("camille.demo@example.org")
        );
        for bad in [
            json!({ "name": "camille" }),
            json!({ "name": "a@b" }),
            json!({ "name": "a@.b" }),
            json!({ "name": "a@b." }),
            json!({ "name": "a b@c.fr" }),
            json!({ "name": "a@b@c.fr" }),
            json!({ "name": 5 }),
            json!({}),
        ] {
            assert_eq!(account_address(&bad), None, "{bad}");
        }
        assert_eq!(
            account_address(&json!({ "name": "a@b.c.d" })).as_deref(),
            Some("a@b.c.d")
        );
    }

    #[test]
    fn reponse_des_preferences() {
        let identities = vec![Identity {
            id: "acct-1".into(),
            name: "Camille Demo".into(),
            email: "camille.demo@example.org".into(),
            is_default: true,
        }];
        let limits = attachment_limits_from_info(None);
        assert_eq!(
            prefs_answer(5, &limits, &identities),
            json!({
                "markRead": 5,
                "attachments": { "maxFileBytes": limits.max_file_bytes, "maxTotalBytes": limits.max_total_bytes },
                "identities": [{ "id": "acct-1", "name": "Camille Demo", "email": "camille.demo@example.org" }],
            })
        );
    }
}
