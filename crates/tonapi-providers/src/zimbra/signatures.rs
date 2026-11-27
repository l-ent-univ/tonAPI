//! Signatures de l'étudiant dans le webmail (`GetSignatures`) et celle que Tafac y écrit. Repris
//! de `server/mailSignature.js` (`readZimbraSignatures`, `findCustomSignature`,
//! `pickPartageSignature`, `buildCustomSignatureRequest`) et de `src/mailSignatureChoice.js`
//! (`normalizeCustomSignatureText`, `signatureLogin`, `CUSTOM_SIGNATURE_NAME`).
//!
//! Le HTML et le texte d'une signature reçue ne sont jamais montrés ni envoyés tels quels : la
//! lecture ici les rend bruts ([`RawSignature`]), le nettoyage (`readSignatureContent`) est fait
//! par `mail::html`, et l'appelant assemble la [`Signature`] nettoyée.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::mail::text::{
    cap_chars, char_count, clean_text_lines, collapse_spaces, escape_html, replace_controls,
    trim_js, utf16_len,
};

use super::identities::{Identity, IdentitySignatureIds, is_valid_signature_id};
use super::listing::js_string;

/// Le nom de la signature que Tafac écrit dans le webmail, la seule qu'il change jamais.
pub const CUSTOM_SIGNATURE_NAME: &str = "Tafac";
/// Longueur maximale de la signature personnelle, en points de code (refusée au-delà, pas coupée).
pub const CUSTOM_SIGNATURE_MAX_LENGTH: usize = 2000;
/// Lignes de la signature personnelle au plus.
pub const CUSTOM_SIGNATURE_MAX_LINES: usize = 30;
/// Signatures lues au plus (Zimbra en permet 20).
pub const MAX_SIGNATURES: usize = 30;
/// Longueur maximale du nom d'une signature, en points de code.
pub const MAX_SIGNATURE_NAME_LENGTH: usize = 80;
/// La « sans signature » du webmail (`ZmIdentity.SIG_ID_NONE`).
pub const NO_SIGNATURE_ID: &str = "11111111-1111-1111-1111-111111111111";

/// Une signature du webmail telle que lue : identifiant et nom vérifiés, contenus bruts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSignature {
    /// Identifiant (`^[A-Za-z0-9-]{1,64}$`).
    pub id: String,
    /// Nom, sans caractère de contrôle, une espace entre les mots, 80 points de code au plus.
    pub name: String,
    /// Contenu `text/html` brut, à nettoyer (`readSignatureContent`).
    pub html: Option<String>,
    /// Contenu `text/plain` brut, à nettoyer.
    pub text: Option<String>,
}

/// Une signature nettoyée, telle que la page la reçoit et qu'un envoi la prend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// Identifiant.
    pub id: String,
    /// Nom.
    pub name: String,
    /// HTML nettoyé.
    pub html: String,
    /// Texte nettoyé.
    pub text: String,
    /// Elle avait des images, laissées de côté.
    pub has_images: bool,
}

/// Ce qui a un identifiant et un nom de signature.
pub trait NamedSignature {
    /// Identifiant.
    fn signature_id(&self) -> &str;
    /// Nom.
    fn signature_name(&self) -> &str;
}

impl NamedSignature for RawSignature {
    fn signature_id(&self) -> &str {
        &self.id
    }
    fn signature_name(&self) -> &str {
        &self.name
    }
}

impl NamedSignature for Signature {
    fn signature_id(&self) -> &str {
        &self.id
    }
    fn signature_name(&self) -> &str {
        &self.name
    }
}

/// `asList` : une liste, un objet seul comme liste d'un élément, sinon rien.
fn as_list(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(list)) => list.iter().collect(),
        Some(object @ Value::Object(_)) => vec![object],
        _ => Vec::new(),
    }
}

/// Nom d'une signature : contrôles en espaces, une espace entre les mots, 80 points de code.
fn signature_name(value: Option<&Value>) -> String {
    let name = value.and_then(js_string).unwrap_or_default();
    let name = collapse_spaces(&replace_controls(&name, Some(' ')));
    cap_chars(trim_js(&name), MAX_SIGNATURE_NAME_LENGTH).to_owned()
}

/// `readZimbraSignatures`, sans le nettoyage des contenus : les signatures d'une
/// `GetSignaturesResponse` dans l'ordre du webmail, 30 lues au plus. Une signature sans
/// identifiant de la bonne forme, ou déjà vue, est laissée de côté. `content` peut être une liste
/// ou un objet seul ; le premier contenu de chaque type (`text/html`, `text/plain`) qui est une
/// chaîne est pris.
#[must_use]
pub fn read_signatures(response: &Value) -> Vec<RawSignature> {
    let mut signatures = Vec::new();
    let mut seen = HashSet::new();
    for entry in as_list(response.get("signature"))
        .into_iter()
        .take(MAX_SIGNATURES)
    {
        let id = match entry.get("id") {
            None | Some(Value::Null) => String::new(),
            Some(value) => js_string(value).unwrap_or_default(),
        };
        if !is_valid_signature_id(&id) || seen.contains(&id) {
            continue;
        }
        seen.insert(id.clone());
        let contents = as_list(entry.get("content"));
        let pick = |wanted: &str| {
            contents
                .iter()
                .find(|content| {
                    let kind = content
                        .get("type")
                        .and_then(js_string)
                        .unwrap_or_default()
                        .to_lowercase();
                    kind == wanted && content.get("_content").is_some_and(Value::is_string)
                })
                .and_then(|content| content.get("_content"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        signatures.push(RawSignature {
            name: signature_name(entry.get("name")),
            html: pick("text/html"),
            text: pick("text/plain"),
            id,
        });
    }
    signatures
}

/// `findCustomSignature` : celle que Tafac écrit ([`CUSTOM_SIGNATURE_NAME`]), ou `None`.
#[must_use]
pub fn find_custom_signature<S: NamedSignature>(signatures: &[S]) -> Option<&S> {
    signatures
        .iter()
        .find(|signature| signature.signature_name() == CUSTOM_SIGNATURE_NAME)
}

/// `pickPartageSignature` : la signature du webmail qu'un mail prend quand l'étudiant a choisi
/// « Partage ». Celle de l'identité (pour une réponse ou un transfert, celle de réponse si elle
/// en nomme une), sinon, aucune ou une supprimée depuis, la première des siennes hors celle de
/// Tafac qui a du texte. `None` quand il n'en a aucune.
#[must_use]
pub fn pick_partage_signature<'a>(
    signatures: &'a [Signature],
    ids: Option<&IdentitySignatureIds>,
    reply: bool,
) -> Option<&'a Signature> {
    let by_id = |id: Option<&String>| {
        id.filter(|id| id.as_str() != NO_SIGNATURE_ID)
            .and_then(|id| signatures.iter().find(|signature| &signature.id == id))
    };
    let reply_one = if reply {
        by_id(ids.and_then(|ids| ids.reply_signature_id.as_ref()))
    } else {
        None
    };
    reply_one
        .or_else(|| by_id(ids.and_then(|ids| ids.signature_id.as_ref())))
        .or_else(|| {
            signatures.iter().find(|signature| {
                signature.name != CUSTOM_SIGNATURE_NAME && !signature.text.is_empty()
            })
        })
}

/// La réponse de `/mail/signatures` (`describeMailSignatures`) : `{signatures: [{id, name, html,
/// text, hasImages}], identities: [{id, signatureId, replySignatureId}], customId, maxLength}`.
/// `signature_ids` : ceux de [`super::identities::read_identity_signature_ids`].
#[must_use]
pub fn signatures_answer<H: std::hash::BuildHasher>(
    signatures: &[Signature],
    identities: &[Identity],
    signature_ids: &HashMap<String, IdentitySignatureIds, H>,
) -> Value {
    json!({
        "signatures": signatures
            .iter()
            .map(|signature| json!({
                "id": signature.id,
                "name": signature.name,
                "html": signature.html,
                "text": signature.text,
                "hasImages": signature.has_images,
            }))
            .collect::<Vec<_>>(),
        "identities": identities
            .iter()
            .map(|identity| {
                let ids = signature_ids.get(&identity.id);
                json!({
                    "id": identity.id,
                    "signatureId": pick_partage_signature(signatures, ids, false).map(|s| &s.id),
                    "replySignatureId": pick_partage_signature(signatures, ids, true).map(|s| &s.id),
                })
            })
            .collect::<Vec<_>>(),
        "customId": find_custom_signature(signatures).map(|s| &s.id),
        "maxLength": CUSTOM_SIGNATURE_MAX_LENGTH,
    })
}

/// `plainSignatureToHtml` : une signature en texte comme HTML, une ligne par `<div>`, une ligne
/// vide gardée.
fn plain_signature_to_html(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            if line.is_empty() {
                "<div><br /></div>".to_owned()
            } else {
                format!("<div>{}</div>", escape_html(line))
            }
        })
        .collect()
}

/// Une requête SOAP (urn:zimbraAccount) : son nom et son corps.
#[derive(Debug, Clone, PartialEq)]
pub struct SignatureRequest {
    /// `CreateSignatureRequest`, `ModifySignatureRequest` ou `DeleteSignatureRequest`.
    pub name: &'static str,
    /// Le corps.
    pub body: Value,
}

/// `buildCustomSignatureRequest` : la requête qui fait de la signature Tafac du webmail le texte
/// `text` (déjà passé par [`normalize_custom_signature_text`]), `existing` celle qu'il a
/// ([`find_custom_signature`]) : modifiée (par son identifiant), créée, supprimée pour un texte
/// vide, ou rien à faire (`None`). Ses deux contenus, texte et HTML, pour que le webmail montre la
/// même dans ses deux éditeurs.
#[must_use]
pub fn custom_signature_request<S: NamedSignature>(
    text: &str,
    existing: Option<&S>,
) -> Option<SignatureRequest> {
    if text.is_empty() {
        return existing.map(|existing| SignatureRequest {
            name: "DeleteSignatureRequest",
            body: json!({ "signature": { "id": existing.signature_id() } }),
        });
    }
    let content = json!([
        { "type": "text/plain", "_content": text },
        { "type": "text/html", "_content": plain_signature_to_html(text) },
    ]);
    Some(match existing {
        Some(existing) => SignatureRequest {
            name: "ModifySignatureRequest",
            body: json!({ "signature": {
                "id": existing.signature_id(),
                "name": CUSTOM_SIGNATURE_NAME,
                "content": content,
            } }),
        },
        None => SignatureRequest {
            name: "CreateSignatureRequest",
            body: json!({ "signature": { "name": CUSTOM_SIGNATURE_NAME, "content": content } }),
        },
    })
}

/// Pourquoi un texte de signature personnelle est refusé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomSignatureError {
    /// Pas une chaîne : `mail_signature_invalid` (400).
    Invalid,
    /// Trop long ou trop de lignes : `mail_signature_too_long` (413).
    TooLong,
}

impl CustomSignatureError {
    /// Le code d'erreur que la page attend.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            CustomSignatureError::Invalid => "mail_signature_invalid",
            CustomSignatureError::TooLong => "mail_signature_too_long",
        }
    }
}

/// `normalizeCustomSignatureText` : le texte de la signature personnelle tel qu'il est gardé
/// (`cleanTextLines` : NFC, fins de ligne Unix, sans contrôle, sans espace en fin de ligne, sans
/// ligne vide aux bords ni plus d'une de suite). `""` la retire. Refusé (pas coupé) au-delà de
/// 2000 points de code ou de 30 lignes, et d'emblée au-delà de 8000 unités UTF-16. `value` : le
/// champ `text` du corps JSON, absent ou d'un autre type que chaîne refusé.
pub fn normalize_custom_signature_text(
    value: Option<&Value>,
) -> Result<String, CustomSignatureError> {
    let Some(Value::String(value)) = value else {
        return Err(CustomSignatureError::Invalid);
    };
    if utf16_len(value) > CUSTOM_SIGNATURE_MAX_LENGTH * 4 {
        return Err(CustomSignatureError::TooLong);
    }
    let text = clean_text_lines(value);
    if char_count(&text) > CUSTOM_SIGNATURE_MAX_LENGTH
        || text.split('\n').count() > CUSTOM_SIGNATURE_MAX_LINES
    {
        return Err(CustomSignatureError::TooLong);
    }
    Ok(text)
}

/// `signatureLogin` : l'identifiant CAS que la signature automatique nomme là où l'université le
/// demande : `^[A-Za-z0-9._-]{1,32}$` une fois les espaces de bord retirées, sinon `None` (une
/// adresse tapée pour se connecter n'est pas un identifiant).
#[must_use]
pub fn signature_login(user: &str) -> Option<&str> {
    let login = trim_js(user);
    let valid = (1..=32).contains(&login.len())
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    valid.then_some(login)
}

/// `signatureLoginLine` : la ligne telle que la signature la montre.
#[must_use]
pub fn signature_login_line(login: &str) -> String {
    format!("Identifiant : {login}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature(id: &str, name: &str, text: &str) -> Signature {
        Signature {
            id: id.into(),
            name: name.into(),
            html: String::new(),
            text: text.into(),
            has_images: false,
        }
    }

    #[test]
    fn get_signatures_les_deux_formes() {
        let list = read_signatures(&json!({
            "signature": [
                { "id": "a1b2c3d4-0000-4000-8000-000000000001", "name": " Pro\n ", "content": [{ "type": "text/html", "_content": "<div><b>Camille</b></div>" }, { "type": "text/plain", "_content": "Camille" }] },
                { "id": "a1b2c3d4-0000-4000-8000-000000000002", "name": "Tafac", "content": { "type": "TEXT/PLAIN", "_content": "Camille\r\nMMI 2" } },
                { "id": "<script>", "name": "x", "content": [] },
                { "id": "a1b2c3d4-0000-4000-8000-000000000001", "name": "twice", "content": [] },
                { "name": "no id" },
                { "id": "٣", "name": "chiffre arabe" },
                { "id": "__proto__", "name": "proto" },
            ],
        }));
        assert_eq!(
            list.iter()
                .map(|s| (s.id.chars().last().unwrap(), s.name.as_str()))
                .collect::<Vec<_>>(),
            [('1', "Pro"), ('2', "Tafac")]
        );
        assert_eq!(
            list[0],
            RawSignature {
                id: "a1b2c3d4-0000-4000-8000-000000000001".into(),
                name: "Pro".into(),
                html: Some("<div><b>Camille</b></div>".into()),
                text: Some("Camille".into()),
            }
        );
        assert_eq!(list[1].html, None);
        assert_eq!(list[1].text.as_deref(), Some("Camille\r\nMMI 2"));
        assert_eq!(
            find_custom_signature(&list).map(|s| s.id.as_str()),
            Some("a1b2c3d4-0000-4000-8000-000000000002")
        );
        // Une seule, pas en liste ; rien du tout.
        assert_eq!(
            read_signatures(&json!({ "signature": { "id": "abc", "name": "Seule", "content": { "type": "text/plain", "_content": "x" } } })).len(),
            1
        );
        assert!(read_signatures(&Value::Null).is_empty());
        assert!(read_signatures(&json!({ "signature": "nope" })).is_empty());
        // Contenu d'un autre type que chaîne : ignoré.
        let odd = read_signatures(
            &json!({ "signature": [{ "id": "x", "content": [{ "type": "text/plain", "_content": 5 }, { "type": "text/plain", "_content": "ok" }] }] }),
        );
        assert_eq!(odd[0].text.as_deref(), Some("ok"));
    }

    #[test]
    fn noms_nettoyes_et_trente_au_plus() {
        let list = read_signatures(
            &json!({ "signature": [{ "id": "x", "name": format!("a\u{202e}b\tc {}", "é".repeat(100)) }] }),
        );
        assert!(list[0].name.starts_with("a b c "));
        assert_eq!(list[0].name.chars().count(), MAX_SIGNATURE_NAME_LENGTH);
        let many: Vec<Value> = (0..50).map(|i| json!({ "id": format!("s{i}") })).collect();
        assert_eq!(
            read_signatures(&json!({ "signature": many })).len(),
            MAX_SIGNATURES
        );
    }

    #[test]
    fn signature_partage_choisie() {
        let ids = IdentitySignatureIds {
            signature_id: Some("sig-1".into()),
            reply_signature_id: Some("sig-2".into()),
        };
        let none = IdentitySignatureIds {
            signature_id: Some(NO_SIGNATURE_ID.into()),
            reply_signature_id: None,
        };
        let gone = IdentitySignatureIds {
            signature_id: Some("gone".into()),
            reply_signature_id: None,
        };
        let signatures = [
            signature("sig-tafac", "Tafac", "mine"),
            signature("sig-1", "Nouveau", "new"),
            signature("sig-2", "Réponse", "reply"),
        ];
        let pick = |ids: Option<&IdentitySignatureIds>, reply: bool| {
            pick_partage_signature(&signatures, ids, reply).map(|s| s.id.as_str())
        };
        assert_eq!(pick(Some(&ids), false), Some("sig-1"));
        assert_eq!(pick(Some(&ids), true), Some("sig-2"));
        // « Aucune » ou rien de choisi : la première hors celle de Tafac.
        assert_eq!(pick(Some(&none), false), Some("sig-1"));
        assert_eq!(pick(Some(&gone), false), Some("sig-1"));
        assert_eq!(pick_partage_signature(&signatures[..1], None, false), None);
        assert_eq!(pick_partage_signature(&[], None, false), None);
    }

    #[test]
    fn reponse_des_signatures() {
        let signatures = [
            signature("sig-tafac", "Tafac", "mine"),
            signature("sig-1", "Nouveau", "new"),
        ];
        let identities = [Identity {
            id: "acc-1".into(),
            name: String::new(),
            email: "me@univ.fr".into(),
            is_default: true,
        }];
        let mut ids = HashMap::new();
        ids.insert(
            "acc-1".to_owned(),
            IdentitySignatureIds {
                signature_id: Some("sig-1".into()),
                reply_signature_id: None,
            },
        );
        let answer = signatures_answer(&signatures, &identities, &ids);
        assert_eq!(answer["customId"], "sig-tafac");
        assert_eq!(answer["maxLength"], 2000);
        assert_eq!(
            answer["identities"],
            json!([{ "id": "acc-1", "signatureId": "sig-1", "replySignatureId": "sig-1" }])
        );
        assert_eq!(
            answer["signatures"][1],
            json!({ "id": "sig-1", "name": "Nouveau", "html": "", "text": "new", "hasImages": false })
        );
        let empty = signatures_answer(&[], &[], &HashMap::new());
        assert_eq!(empty["customId"], Value::Null);
    }

    #[test]
    fn texte_de_la_signature_personnelle() {
        let text = |value: Value| normalize_custom_signature_text(Some(&value));
        assert_eq!(
            text(json!("\n\n  Camille  \r\nMMI\t2\u{7}\n\n\n\nfin  \n")),
            Ok("  Camille\nMMI 2\n\nfin".to_owned())
        );
        assert_eq!(text(json!("")), Ok(String::new()));
        assert_eq!(text(json!(42)), Err(CustomSignatureError::Invalid));
        assert_eq!(text(Value::Null), Err(CustomSignatureError::Invalid));
        assert_eq!(
            normalize_custom_signature_text(None),
            Err(CustomSignatureError::Invalid)
        );
        assert!(text(json!("x".repeat(CUSTOM_SIGNATURE_MAX_LENGTH))).is_ok());
        assert_eq!(
            text(json!("x".repeat(CUSTOM_SIGNATURE_MAX_LENGTH + 1))),
            Err(CustomSignatureError::TooLong)
        );
        assert_eq!(
            text(json!(vec!["l"; 31].join("\n"))),
            Err(CustomSignatureError::TooLong)
        );
        assert_eq!(
            text(json!(" ".repeat(8001))),
            Err(CustomSignatureError::TooLong)
        );
        assert_eq!(
            CustomSignatureError::TooLong.code(),
            "mail_signature_too_long"
        );
        assert_eq!(
            CustomSignatureError::Invalid.code(),
            "mail_signature_invalid"
        );
    }

    #[test]
    fn ecriture_de_la_signature_personnelle() {
        let existing = signature("sig-tafac", "Tafac", "");
        assert_eq!(
            custom_signature_request::<Signature>("Camille <3", None),
            Some(SignatureRequest {
                name: "CreateSignatureRequest",
                body: json!({ "signature": { "name": "Tafac", "content": [
                    { "type": "text/plain", "_content": "Camille <3" },
                    { "type": "text/html", "_content": "<div>Camille &lt;3</div>" },
                ] } }),
            })
        );
        let modify = custom_signature_request("x\n\ny", Some(&existing)).unwrap();
        assert_eq!(modify.name, "ModifySignatureRequest");
        assert_eq!(modify.body["signature"]["id"], "sig-tafac");
        assert_eq!(
            modify.body["signature"]["content"][1]["_content"],
            "<div>x</div><div><br /></div><div>y</div>"
        );
        assert_eq!(
            custom_signature_request("", Some(&existing)),
            Some(SignatureRequest {
                name: "DeleteSignatureRequest",
                body: json!({ "signature": { "id": "sig-tafac" } }),
            })
        );
        assert_eq!(custom_signature_request::<Signature>("", None), None);
    }

    #[test]
    fn identifiant_de_la_signature_automatique() {
        assert_eq!(signature_login(" e2101234 "), Some("e2101234"));
        assert_eq!(signature_login("prenom.nom-2_x"), Some("prenom.nom-2_x"));
        for bad in [
            "a@b",
            "camille@univ-rennes2.fr",
            "<b>x</b>",
            &"a".repeat(33),
            "",
            "é",
            "٣",
        ] {
            assert_eq!(signature_login(bad), None, "{bad}");
        }
        assert_eq!(signature_login_line("e2101234"), "Identifiant : e2101234");
    }
}
