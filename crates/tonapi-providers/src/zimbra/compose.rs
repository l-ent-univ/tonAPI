//! L'envoi et les brouillons, côté Zimbra (`server/mailSend.js` et la route GET `/mail/draft`
//! de Tafac) : le `<m>` d'un `SendMsgRequest` ou d'un `SaveDraftRequest` en JSON, les parties
//! d'un brouillon enregistré rapportées aux références de la page, et un brouillon rendu à
//! l'éditeur. Le HTML passe par `mail::html` (passe stricte d'abord).

use serde_json::{Map, Value, json};

use super::message::{
    MAX_MIME_DEPTH, as_array, build_mail_part_href, first_message, js_number, js_string, js_truthy,
    number_json,
};
use crate::mail::address::{is_valid_address, normalize_email, sanitize_display_name};
use crate::mail::attachments::{normalize_attachment_type, sanitize_attachment_name};
use crate::mail::html::{
    is_inline_cid, plain_text_to_composer_html, read_draft_images, sanitize_composer_html,
};
use crate::mail::text::{sanitize_subject, trim_js};

/// Les types d'une image du texte.
pub const INLINE_IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/gif"];

/// Un destinataire (déjà vérifié).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient {
    /// Nom affiché, vide s'il n'y en a pas.
    pub name: String,
    /// Adresse.
    pub email: String,
}

/// L'identité d'envoi, prise dans la liste du compte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenderIdentity {
    /// Son id Zimbra (`default` : l'adresse du compte, sans `idnt`).
    pub id: String,
    /// Nom affiché.
    pub name: String,
    /// Adresse.
    pub email: String,
}

/// Un fichier nommé par un envoi : un dépôt de la session, ou la partie d'un mail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartReference {
    /// Un fichier déposé (`aid` de Zimbra).
    Upload {
        /// L'id du dépôt.
        aid: String,
    },
    /// La partie d'un mail de la boîte.
    Message {
        /// L'id du mail.
        message_id: String,
        /// Le numéro de la partie.
        part: String,
    },
}

impl PartReference {
    fn is_upload(&self) -> bool {
        matches!(self, Self::Upload { .. })
    }
}

/// Une partie en ligne du mail (logo de la signature, image du texte), par son Content-ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlinePart {
    /// Content-ID (chevrons facultatifs).
    pub cid: String,
    /// D'où vient le fichier.
    pub reference: PartReference,
}

/// Le mail auquel on répond, ou qu'on transfère.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyOrigin {
    /// Son id.
    pub message_id: String,
    /// `r` (réponse) ou `w` (transfert).
    pub reply_type: String,
    /// Son en-tête Message-ID, lu par le serveur (pour In-Reply-To).
    pub message_id_header: Option<String>,
}

/// Ce que `build_zimbra_message` assemble.
#[derive(Debug, Clone, Copy)]
pub struct ZimbraMessage<'a> {
    /// L'identité d'envoi (`pickIdentity`).
    pub identity: Option<&'a SenderIdentity>,
    /// Destinataires.
    pub to: &'a [Recipient],
    /// Copies.
    pub cc: &'a [Recipient],
    /// Copies cachées.
    pub bcc: &'a [Recipient],
    /// Objet (déjà nettoyé).
    pub subject: &'a str,
    /// La partie HTML (`build_mail_html`).
    pub html: &'a str,
    /// La partie texte (`composer_html_to_text`).
    pub text: &'a str,
    /// Les parties en ligne.
    pub inline_parts: &'a [InlinePart],
    /// Les pièces jointes.
    pub attachments: &'a [PartReference],
    /// La réponse ou le transfert.
    pub origin: Option<&'a ReplyOrigin>,
    /// Envoi : le brouillon qu'il remplace (`did`) ; brouillon : celui qu'il écrase (`id`).
    pub draft_id: Option<&'a str>,
    /// Un `SaveDraftRequest`.
    pub for_draft: bool,
}

fn strip_angles(cid: &str) -> &str {
    let cid = cid.strip_prefix('<').unwrap_or(cid);
    cid.strip_suffix('>').unwrap_or(cid)
}

fn address(kind: &str, name: &str, email: &str) -> Value {
    if name.is_empty() {
        json!({ "t": kind, "a": email })
    } else {
        json!({ "t": kind, "a": email, "p": name })
    }
}

/// `buildZimbraMessage` : le `<m>` d'un `SendMsgRequest` ou d'un `SaveDraftRequest`. `ci` part
/// sans chevrons (Zimbra les ajoute) ; From vient de l'identité, jamais de la page.
#[must_use]
pub fn build_zimbra_message(input: &ZimbraMessage<'_>) -> Value {
    let mut message = Map::new();
    if let Some(identity) = input.identity
        && identity.id != "default"
    {
        message.insert("idnt".into(), json!(identity.id));
    }
    if let Some(draft_id) = input.draft_id.filter(|id| !id.is_empty()) {
        message.insert(
            (if input.for_draft { "id" } else { "did" }).into(),
            json!(draft_id),
        );
    }
    if let Some(origin) = input.origin {
        message.insert("origid".into(), json!(origin.message_id));
        message.insert("rt".into(), json!(origin.reply_type));
        if origin.reply_type == "r"
            && let Some(header) = origin
                .message_id_header
                .as_deref()
                .filter(|h| !h.is_empty())
        {
            message.insert("irt".into(), json!({ "_content": header }));
        }
    }
    let mut addresses = Vec::new();
    if let Some(identity) = input.identity {
        addresses.push(address("f", &identity.name, &identity.email));
    }
    for (kind, list) in [("t", input.to), ("c", input.cc), ("b", input.bcc)] {
        addresses.extend(
            list.iter()
                .map(|entry| address(kind, &entry.name, &entry.email)),
        );
    }
    message.insert("e".into(), Value::Array(addresses));
    message.insert("su".into(), json!({ "_content": input.subject }));
    let html_part = json!({ "ct": "text/html", "content": { "_content": input.html } });
    let second = if input.inline_parts.is_empty() {
        html_part
    } else {
        let mut related = vec![html_part];
        related.extend(input.inline_parts.iter().map(|part| {
            let attach = match &part.reference {
                PartReference::Upload { aid } => json!({ "aid": aid }),
                PartReference::Message { message_id, part } => {
                    json!({ "mp": [{ "mid": message_id, "part": part }] })
                }
            };
            json!({ "ci": strip_angles(&part.cid), "attach": attach })
        }));
        json!({ "ct": "multipart/related", "mp": related })
    };
    message.insert(
        "mp".into(),
        json!([{
            "ct": "multipart/alternative",
            "mp": [{ "ct": "text/plain", "content": { "_content": input.text } }, second],
        }]),
    );
    let uploads: Vec<&str> = input
        .attachments
        .iter()
        .filter_map(|entry| match entry {
            PartReference::Upload { aid } => Some(aid.as_str()),
            PartReference::Message { .. } => None,
        })
        .collect();
    let parts: Vec<Value> = input
        .attachments
        .iter()
        .filter_map(|entry| match entry {
            PartReference::Message { message_id, part } => {
                Some(json!({ "mid": message_id, "part": part }))
            }
            PartReference::Upload { .. } => None,
        })
        .collect();
    if !uploads.is_empty() || !parts.is_empty() {
        let mut attach = Map::new();
        if !uploads.is_empty() {
            attach.insert("aid".into(), json!(uploads.join(",")));
        }
        if !parts.is_empty() {
            attach.insert("mp".into(), Value::Array(parts));
        }
        message.insert("attach".into(), Value::Object(attach));
    }
    Value::Object(message)
}

/// `zimbraAttachmentOrder` : l'ordre où Zimbra met les pièces jointes (les dépôts d'abord, puis
/// les parties d'autres mails), en indices de `attachments`.
#[must_use]
pub fn zimbra_attachment_order(attachments: &[PartReference]) -> Vec<usize> {
    let uploads = (0..attachments.len()).filter(|index| attachments[*index].is_upload());
    let others = (0..attachments.len()).filter(|index| !attachments[*index].is_upload());
    uploads.chain(others).collect()
}

// ── Les parties d'un brouillon ───────────────────────────────────────────

/// Une pièce jointe d'un brouillon enregistré.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftPart {
    /// Numéro de la partie.
    pub part: String,
    /// Nom (`sanitize_attachment_name`).
    pub name: String,
    /// Taille, si Zimbra la donne.
    pub size: Option<u64>,
    /// Type (W10 : `normalize_attachment_type`).
    pub content_type: String,
}

impl DraftPart {
    /// Comme la page la reçoit (`{part, name, size, contentType}`).
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({ "part": self.part, "name": self.name, "size": self.size, "contentType": self.content_type })
    }
}

/// Une image du texte d'un brouillon enregistré.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftInlinePart {
    /// Son Content-ID (forme de l'éditeur).
    pub cid: String,
    /// Numéro de la partie.
    pub part: String,
    /// Nom.
    pub name: String,
    /// Taille.
    pub size: Option<u64>,
    /// Type.
    pub content_type: String,
}

impl DraftInlinePart {
    /// `{cid, part, name, size, contentType}`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({ "cid": self.cid, "part": self.part, "name": self.name, "size": self.size, "contentType": self.content_type })
    }
}

fn is_part_number(value: &str) -> bool {
    !value.is_empty()
        && value
            .split('.')
            .all(|piece| !piece.is_empty() && piece.bytes().all(|b| b.is_ascii_digit()))
}

fn safe_size(value: Option<&Value>) -> Option<u64> {
    let size = js_number(value);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (size.fract() == 0.0 && size > 0.0 && size <= 9_007_199_254_740_991.0).then_some(size as u64)
}

fn part_cid(part: &Value) -> String {
    strip_angles(trim_js(&js_string(part.get("ci")))).to_lowercase()
}

/// `readDraftAttachmentParts` : les pièces jointes d'un brouillon, les parties qui suivent son
/// corps dans son `multipart/mixed`, dans l'ordre.
#[must_use]
pub fn read_draft_attachment_parts(message: &Value) -> Vec<DraftPart> {
    let Some(root) = as_array(message.get("mp")).first() else {
        return Vec::new();
    };
    let Some(parts) = root.get("mp").and_then(Value::as_array) else {
        return Vec::new();
    };
    if js_string(root.get("ct")).to_lowercase() != "multipart/mixed" {
        return Vec::new();
    }
    parts
        .iter()
        .skip(1)
        .filter_map(|part| {
            let number = part.get("part")?.as_str()?;
            if !is_part_number(number) || is_inline_cid(&part_cid(part)) {
                return None;
            }
            let content_type = match part.get("ct") {
                None | Some(Value::Null) => "application/octet-stream".to_owned(),
                value => js_string(value),
            };
            Some(DraftPart {
                part: number.to_owned(),
                name: sanitize_attachment_name(&js_string(part.get("filename"))),
                size: safe_size(part.get("s")),
                content_type: normalize_attachment_type(&content_type),
            })
        })
        .collect()
}

fn walk_inline_parts(parts: Option<&Value>, depth: usize, found: &mut Vec<DraftInlinePart>) {
    if depth > MAX_MIME_DEPTH {
        return;
    }
    for part in as_array(parts) {
        if !part.is_object() {
            continue;
        }
        if part.get("mp").is_some_and(Value::is_array) {
            walk_inline_parts(part.get("mp"), depth + 1, found);
        }
        let cid = part_cid(part);
        let content_type = js_string(part.get("ct")).to_lowercase();
        let Some(number) = part.get("part").and_then(Value::as_str) else {
            continue;
        };
        if !is_inline_cid(&cid)
            || !INLINE_IMAGE_TYPES.contains(&content_type.as_str())
            || !is_part_number(number)
            || found.iter().any(|entry| entry.cid == cid)
        {
            continue;
        }
        found.push(DraftInlinePart {
            cid,
            part: number.to_owned(),
            name: sanitize_attachment_name(&js_string(part.get("filename"))),
            size: safe_size(part.get("s")),
            content_type,
        });
    }
}

/// `readDraftInlineParts` : les images du texte d'un brouillon, toutes les parties portant un
/// Content-ID de l'éditeur (une fois chacun).
#[must_use]
pub fn read_draft_inline_parts(message: &Value) -> Vec<DraftInlinePart> {
    let mut found = Vec::new();
    walk_inline_parts(message.get("mp"), 0, &mut found);
    found
}

/// Une référence devenue partie d'un brouillon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftPartRef {
    /// Le brouillon.
    pub message_id: String,
    /// Sa partie.
    pub part: String,
}

impl DraftPartRef {
    /// `{messageId, part}`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({ "messageId": self.message_id, "part": self.part })
    }
}

/// Après un enregistrement : chaque image du texte comme partie du brouillon (`None` si elle n'y
/// est pas), et les dépôts devenus parties (à oublier).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettledInlineImages {
    /// `(cid, partie)` dans l'ordre de la page.
    pub inline_images: Vec<Option<(String, DraftPartRef)>>,
    /// Les dépôts utilisés.
    pub used: Vec<String>,
}

/// `settleDraftInlineImages`.
#[must_use]
pub fn settle_draft_inline_images(
    images: &[InlinePart],
    draft_id: &str,
    parts: &[DraftInlinePart],
) -> SettledInlineImages {
    let inline_images: Vec<Option<(String, DraftPartRef)>> = images
        .iter()
        .map(|image| {
            parts
                .iter()
                .rev()
                .find(|part| part.cid == image.cid)
                .map(|part| {
                    (
                        image.cid.clone(),
                        DraftPartRef {
                            message_id: draft_id.to_owned(),
                            part: part.part.clone(),
                        },
                    )
                })
        })
        .collect();
    let used = images
        .iter()
        .zip(&inline_images)
        .filter_map(|(image, mapped)| match (&image.reference, mapped) {
            (PartReference::Upload { aid }, Some(_)) => Some(aid.clone()),
            _ => None,
        })
        .collect();
    SettledInlineImages {
        inline_images,
        used,
    }
}

/// `mapDraftAttachments` : chaque référence de la page comme partie du brouillon. Par position
/// (`zimbra_attachment_order`) quand il en a autant ; sinon par nom (`names` : ceux des
/// références, `None` quand inconnu).
#[must_use]
pub fn map_draft_attachments(
    attachments: &[PartReference],
    draft_id: &str,
    parts: &[DraftPart],
    names: &[Option<String>],
) -> Vec<Option<DraftPartRef>> {
    let mut result: Vec<Option<DraftPartRef>> = vec![None; attachments.len()];
    if parts.len() == attachments.len() {
        for (position, index) in zimbra_attachment_order(attachments).into_iter().enumerate() {
            if let (Some(slot), Some(part)) = (result.get_mut(index), parts.get(position)) {
                *slot = Some(DraftPartRef {
                    message_id: draft_id.to_owned(),
                    part: part.part.clone(),
                });
            }
        }
        return result;
    }
    let mut free: Vec<&DraftPart> = parts.iter().collect();
    for (index, slot) in result.iter_mut().enumerate() {
        let Some(name) = names
            .get(index)
            .and_then(Option::as_deref)
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        let name = sanitize_attachment_name(name);
        if let Some(at) = free.iter().position(|part| part.name == name) {
            let part = free.remove(at);
            *slot = Some(DraftPartRef {
                message_id: draft_id.to_owned(),
                part: part.part.clone(),
            });
        }
    }
    result
}

/// Après un enregistrement : les références et les dépôts devenus parties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettledAttachments {
    /// Chaque référence comme partie du brouillon.
    pub attachments: Vec<Option<DraftPartRef>>,
    /// Les dépôts à oublier.
    pub used: Vec<String>,
}

/// `settleDraftAttachments` : un dépôt resté sans partie n'est pas oublié (la page le renomme
/// au prochain enregistrement).
#[must_use]
pub fn settle_draft_attachments(
    attachments: &[PartReference],
    draft_id: &str,
    parts: &[DraftPart],
    names: &[Option<String>],
) -> SettledAttachments {
    let mapped = map_draft_attachments(attachments, draft_id, parts, names);
    let used = attachments
        .iter()
        .zip(&mapped)
        .filter_map(|(entry, mapped)| match (entry, mapped) {
            (PartReference::Upload { aid }, Some(_)) => Some(aid.clone()),
            _ => None,
        })
        .collect();
    SettledAttachments {
        attachments: mapped,
        used,
    }
}

// ── Un brouillon, de retour dans l'éditeur ───────────────────────────────

fn collect_bodies(parts: Option<&Value>, depth: usize, found: &mut Vec<(String, String)>) {
    if depth > MAX_MIME_DEPTH {
        return;
    }
    for part in as_array(parts) {
        if !part.is_object() {
            continue;
        }
        let kind = js_string(part.get("ct")).to_lowercase();
        if js_truthy(part.get("body"))
            && let Some(content) = part.get("content").and_then(Value::as_str)
        {
            found.push((kind, content.to_owned()));
        } else if kind.starts_with("multipart/") {
            collect_bodies(part.get("mp"), depth + 1, found);
        }
    }
}

/// `findBodyParts` : les corps (`body`) d'un message, `(type, contenu)`, dans l'ordre.
#[must_use]
pub fn find_body_parts(parts: Option<&Value>) -> Vec<(String, String)> {
    let mut found = Vec::new();
    collect_bodies(parts, 0, &mut found);
    found
}

fn draft_recipients(message: &Value, kind: &str) -> Vec<Value> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for entry in as_array(message.get("e")) {
        if entry.get("t").and_then(Value::as_str) != Some(kind) {
            continue;
        }
        let Some(email) = entry.get("a").and_then(Value::as_str) else {
            continue;
        };
        if !is_valid_address(email) {
            continue;
        }
        let key = normalize_email(email);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let name = ["p", "d"]
            .iter()
            .filter_map(|key| entry.get(*key).and_then(Value::as_str))
            .find(|name| !name.is_empty())
            .unwrap_or_default();
        out.push(json!({ "name": sanitize_display_name(name, email), "email": trim_js(email) }));
    }
    out
}

/// `/\s${name}="([^"]*)"/` dans une balise.
fn tag_attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let bytes = tag.as_bytes();
    let pattern = format!("{name}=\"");
    let mut from = 0;
    while let Some(offset) = tag[from..].find(&pattern) {
        let at = from + offset;
        if at > 0 && crate::mail::text::is_js_space(char::from(bytes[at - 1])) {
            let start = at + pattern.len();
            let end = tag[start..].find('"')?;
            return Some(&tag[start..start + end]);
        }
        from = at + 1;
    }
    None
}

/// La taille de chaque image du HTML nettoyé, par son `data-inline-cid`.
fn image_sizes(html: &str) -> Vec<(Option<String>, Value, Value)> {
    let mut sizes = Vec::new();
    let mut from = 0;
    while let Some(offset) = html[from..].find("<img") {
        let at = from + offset;
        let next = html.as_bytes().get(at + 4);
        if next.is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_') {
            from = at + 4;
            continue;
        }
        let Some(end) = html[at..].find('>') else {
            break;
        };
        let tag = &html[at..=at + end];
        let read = |name: &str| {
            let number = js_number(
                tag_attribute(tag, name)
                    .map(|v| Value::String(v.to_owned()))
                    .as_ref(),
            );
            if number == 0.0 || number.is_nan() {
                Value::Null
            } else {
                number_json(number)
            }
        };
        sizes.push((
            tag_attribute(tag, "data-inline-cid").map(str::to_owned),
            read("width"),
            read("height"),
        ));
        from = at + end + 1;
    }
    sizes
}

/// `readDraftForComposer` : un brouillon (`GetMsgRequest` avec `html: 1, neuter: 0`) tel que
/// l'éditeur l'ouvre. Clés de Node : `id`, `to`, `cc`, `bcc`, `subject`, `html` (passé par
/// `sanitize_composer_html`), `attachments`, `inlineImages` (`{cid, part, name, size,
/// contentType, width, height}`), `droppedImages`, `identityId` (l'un de `identity_ids`, sinon
/// `null`), `origin` (`{kind, messageId}` ou `null`).
#[must_use]
pub fn read_draft_for_composer(message: &Value, identity_ids: &[&str]) -> Value {
    let bodies = find_body_parts(message.get("mp"));
    let html_body = bodies.iter().find(|(kind, _)| kind == "text/html");
    let text_body = bodies.iter().find(|(kind, _)| kind == "text/plain");
    let inline_parts = read_draft_inline_parts(message);
    let cids: Vec<String> = inline_parts.iter().map(|part| part.cid.clone()).collect();
    let (html, used, dropped) = if let Some((_, content)) = html_body {
        let images = read_draft_images(content, &cids);
        (
            sanitize_composer_html(&images.html),
            images.used,
            images.dropped,
        )
    } else {
        let text = text_body
            .map(|(_, content)| content.as_str())
            .unwrap_or_default();
        (
            sanitize_composer_html(&plain_text_to_composer_html(text)),
            Vec::new(),
            0,
        )
    };
    let sizes = image_sizes(&html);
    let inline_images: Vec<Value> = used
        .iter()
        .map(|cid| {
            let mut entry = inline_parts
                .iter()
                .find(|part| part.cid == *cid)
                .map_or_else(|| json!({}), DraftInlinePart::to_json);
            if let (Value::Object(map), Some((_, width, height))) = (
                &mut entry,
                sizes
                    .iter()
                    .rev()
                    .find(|(key, _, _)| key.as_deref() == Some(cid.as_str())),
            ) {
                map.insert("width".into(), width.clone());
                map.insert("height".into(), height.clone());
            }
            entry
        })
        .collect();
    let identity = message.get("idnt").and_then(Value::as_str);
    let identity_id = identity.filter(|id| identity_ids.contains(id));
    let origin_id = js_string(message.get("origid"));
    let reply_type = message.get("rt").and_then(Value::as_str);
    let origin = (matches!(reply_type, Some("r" | "w"))
        && (1..=10).contains(&origin_id.len())
        && origin_id.bytes().all(|b| b.is_ascii_digit()))
    .then(|| {
        json!({
            "kind": if reply_type == Some("w") { "forward" } else { "reply" },
            "messageId": origin_id,
        })
    });
    json!({
        "id": js_string(message.get("id")),
        "to": draft_recipients(message, "t"),
        "cc": draft_recipients(message, "c"),
        "bcc": draft_recipients(message, "b"),
        "subject": message.get("su").and_then(Value::as_str).map(sanitize_subject).unwrap_or_default(),
        "html": html,
        "attachments": read_draft_attachment_parts(message).iter().map(DraftPart::to_json).collect::<Vec<_>>(),
        "inlineImages": inline_images,
        "droppedImages": dropped,
        "identityId": identity_id,
        "origin": origin,
    })
}

/// La réponse de la route GET `/mail/draft` : `{draft: …}` d'après le `GetMsgResponse`, chaque
/// image avec son `href` (la route des parties), ou `None` (404) quand il n'y a pas de message ou
/// que ce n'est pas un brouillon (dossier 6 ou drapeau `d`).
#[must_use]
pub fn draft_response(response: &Value, identity_ids: &[&str]) -> Option<Value> {
    let message = first_message(response)?;
    let in_drafts = js_string(message.get("l")) == "6" || js_string(message.get("f")).contains('d');
    if !in_drafts {
        return None;
    }
    let mut draft = read_draft_for_composer(message, identity_ids);
    let id = draft["id"].as_str().unwrap_or_default().to_owned();
    if let Some(Value::Array(images)) = draft.get_mut("inlineImages") {
        for image in images.iter_mut() {
            let part = image
                .get("part")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if let Value::Object(map) = image {
                map.insert(
                    "href".into(),
                    json!(build_mail_part_href(&id, &part, false)),
                );
            }
        }
    }
    if let Some(html) = draft.get("html").and_then(Value::as_str) {
        let html = sanitize_composer_html(html);
        draft["html"] = json!(html);
    }
    Some(json!({ "draft": draft }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE_CID: &str = "tafac-img-0123abcd-4567-89ab@tafac.fr";
    const OTHER_CID: &str = "tafac-img-fedcba98-7654-3210@tafac.fr";

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/mail/zimbra.json"))
            .expect("fixture")
    }

    fn upload(aid: &str) -> PartReference {
        PartReference::Upload {
            aid: aid.to_owned(),
        }
    }

    fn part(message_id: &str, part: &str) -> PartReference {
        PartReference::Message {
            message_id: message_id.to_owned(),
            part: part.to_owned(),
        }
    }

    fn recipient(name: &str, email: &str) -> Recipient {
        Recipient {
            name: name.to_owned(),
            email: email.to_owned(),
        }
    }

    #[test]
    fn brouillons_comme_node() {
        for case in fixture()["drafts"].as_array().expect("liste") {
            let ids: Vec<&str> = case["identities"]
                .as_array()
                .expect("liste")
                .iter()
                .filter_map(Value::as_str)
                .collect();
            assert_eq!(
                read_draft_for_composer(&case["input"], &ids),
                case["output"],
                "{}",
                case["input"]
            );
        }
    }

    #[test]
    fn route_du_brouillon() {
        let case = &fixture()["drafts"][2];
        let response = json!({ "m": case["input"].clone() });
        let draft = draft_response(&response, &[]).expect("brouillon");
        assert_eq!(
            draft["draft"]["inlineImages"][0]["href"],
            json!("/__ent_auth/mail/part?id=400&part=1.2.2")
        );
        assert_eq!(draft["draft"]["droppedImages"], json!(1));
        // Pas un brouillon : 404.
        let mut other = case["input"].clone();
        other["l"] = json!("2");
        assert_eq!(draft_response(&json!({ "m": [other.clone()] }), &[]), None);
        other["f"] = json!("ud");
        assert!(draft_response(&json!({ "m": [other] }), &[]).is_some());
        assert_eq!(draft_response(&json!({}), &[]), None);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn message_zimbra() {
        let identity = SenderIdentity {
            id: "acct-1".into(),
            name: "Camille Martin".into(),
            email: "camille@univ.fr".into(),
        };
        let origin = ReplyOrigin {
            message_id: "257".into(),
            reply_type: "r".into(),
            message_id_header: Some("<abc@univ.fr>".into()),
        };
        let inline = [InlinePart {
            cid: "<tafac-logo-1.x@tafac.fr>".into(),
            reference: upload("logo-aid-1"),
        }];
        let attachments = [part("300", "2"), upload("aid-1"), upload("aid-2")];
        let message = build_zimbra_message(&ZimbraMessage {
            identity: Some(&identity),
            to: &[recipient("Léo", "leo@univ.fr")],
            cc: &[recipient("", "cc@univ.fr")],
            bcc: &[recipient("", "bcc@univ.fr")],
            subject: "Re: TP",
            html: "<html>…</html>",
            text: "texte",
            inline_parts: &inline,
            attachments: &attachments,
            origin: Some(&origin),
            draft_id: Some("400"),
            for_draft: false,
        });
        assert_eq!(
            message,
            json!({
                "idnt": "acct-1",
                "did": "400",
                "origid": "257",
                "rt": "r",
                "irt": { "_content": "<abc@univ.fr>" },
                "e": [
                    { "t": "f", "a": "camille@univ.fr", "p": "Camille Martin" },
                    { "t": "t", "a": "leo@univ.fr", "p": "Léo" },
                    { "t": "c", "a": "cc@univ.fr" },
                    { "t": "b", "a": "bcc@univ.fr" },
                ],
                "su": { "_content": "Re: TP" },
                "mp": [{
                    "ct": "multipart/alternative",
                    "mp": [
                        { "ct": "text/plain", "content": { "_content": "texte" } },
                        { "ct": "multipart/related", "mp": [
                            { "ct": "text/html", "content": { "_content": "<html>…</html>" } },
                            { "ci": "tafac-logo-1.x@tafac.fr", "attach": { "aid": "logo-aid-1" } },
                        ] },
                    ],
                }],
                "attach": { "aid": "aid-1,aid-2", "mp": [{ "mid": "300", "part": "2" }] },
            })
        );
        let me = SenderIdentity {
            id: "default".into(),
            name: String::new(),
            email: "me@univ.fr".into(),
        };
        let forward = ReplyOrigin {
            message_id: "9".into(),
            reply_type: "w".into(),
            message_id_header: Some("<x@y>".into()),
        };
        let inline = [
            InlinePart {
                cid: IMAGE_CID.into(),
                reference: upload("aid-1"),
            },
            InlinePart {
                cid: OTHER_CID.into(),
                reference: part("400", "2.2"),
            },
        ];
        let draft = build_zimbra_message(&ZimbraMessage {
            identity: Some(&me),
            to: &[],
            cc: &[],
            bcc: &[],
            subject: "",
            html: "h",
            text: "t",
            inline_parts: &[],
            attachments: &[],
            origin: Some(&forward),
            draft_id: Some("5"),
            for_draft: true,
        });
        assert_eq!(draft.get("idnt"), None);
        assert_eq!(draft["id"], json!("5"));
        assert_eq!(draft.get("did"), None);
        assert_eq!(draft["rt"], json!("w"));
        assert_eq!(draft.get("irt"), None);
        assert_eq!(draft["e"], json!([{ "t": "f", "a": "me@univ.fr" }]));
        assert_eq!(
            draft["mp"][0]["mp"][1],
            json!({ "ct": "text/html", "content": { "_content": "h" } })
        );
        assert_eq!(draft.get("attach"), None);
        let images = build_zimbra_message(&ZimbraMessage {
            identity: Some(&me),
            to: &[],
            cc: &[],
            bcc: &[],
            subject: "",
            html: "h",
            text: "t",
            inline_parts: &inline,
            attachments: &[],
            origin: None,
            draft_id: None,
            for_draft: false,
        });
        assert_eq!(
            images["mp"][0]["mp"][1]["mp"][2],
            json!({ "ci": OTHER_CID, "attach": { "mp": [{ "mid": "400", "part": "2.2" }] } })
        );
    }

    fn named(parts: &[(&str, &str)]) -> Vec<DraftPart> {
        parts
            .iter()
            .map(|(part, name)| DraftPart {
                part: (*part).into(),
                name: (*name).into(),
                size: None,
                content_type: "application/pdf".into(),
            })
            .collect()
    }

    #[test]
    fn parties_d_un_brouillon() {
        let draft = json!({ "id": "400", "mp": [{ "ct": "multipart/mixed", "part": "TEXT", "mp": [
            { "ct": "multipart/alternative", "part": "1", "mp": [{ "ct": "text/plain", "part": "1.1" }, { "ct": "text/html", "part": "1.2" }] },
            { "ct": "application/pdf", "part": "2", "filename": "a.pdf", "s": 10 },
            { "ct": "image/png", "part": "3", "filename": "b.png", "s": 20 },
            { "ct": "text/plain", "part": "4", "filename": "c.txt", "s": 30 },
        ] }] });
        let parts = read_draft_attachment_parts(&draft);
        assert_eq!(
            parts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["a.pdf", "b.png", "c.txt"]
        );
        assert_eq!(parts[0].size, Some(10));
        let references = [part("257", "2"), upload("a"), upload("b")];
        let mapped = map_draft_attachments(&references, "400", &parts, &[]);
        let numbers: Vec<Option<&str>> = mapped
            .iter()
            .map(|m| m.as_ref().map(|m| m.part.as_str()))
            .collect();
        assert_eq!(numbers, [Some("4"), Some("2"), Some("3")]);
        let names = [
            Some("x".to_owned()),
            Some("b.png".to_owned()),
            Some("nope".to_owned()),
        ];
        let mapped = map_draft_attachments(&references, "400", &parts[1..], &names);
        let numbers: Vec<Option<&str>> = mapped
            .iter()
            .map(|m| m.as_ref().map(|m| m.part.as_str()))
            .collect();
        assert_eq!(numbers, [None, Some("3"), None]);
        assert!(
            read_draft_attachment_parts(&json!({ "mp": [{ "ct": "multipart/alternative" }] }))
                .is_empty()
        );

        let references = [upload("a"), upload("b"), part("257", "2")];
        let parts = named(&[("2", "a.pdf"), ("3", "b.png"), ("4", "c.txt")]);
        assert_eq!(
            settle_draft_attachments(&references, "400", &parts, &[]).used,
            ["a", "b"]
        );
        let names = [Some("a.pdf".to_owned()), Some("b.png".to_owned()), None];
        let settled = settle_draft_attachments(
            &references,
            "400",
            &named(&[("3", "b.png"), ("2", "c.txt")]),
            &names,
        );
        assert_eq!(
            settled.attachments,
            [
                None,
                Some(DraftPartRef {
                    message_id: "400".into(),
                    part: "3".into()
                }),
                None
            ]
        );
        assert_eq!(settled.used, ["b"]);
        assert!(
            settle_draft_attachments(&references, "400", &named(&[("3", "b.png")]), &[])
                .used
                .is_empty()
        );
        let none = settle_draft_attachments(&references, "400", &[], &names);
        assert_eq!(none.attachments, [None, None, None]);
        assert!(none.used.is_empty());
        assert_eq!(zimbra_attachment_order(&references), [0, 1, 2]);
        assert_eq!(
            zimbra_attachment_order(&[part("1", "2"), upload("a")]),
            [1, 0]
        );
    }

    #[test]
    fn images_d_un_brouillon() {
        let draft = &fixture()["drafts"][2]["input"];
        let inline = read_draft_inline_parts(draft);
        assert_eq!(inline.len(), 1);
        assert_eq!(
            inline[0].to_json(),
            json!({ "cid": IMAGE_CID, "part": "1.2.2", "name": "note.png", "size": 80000, "contentType": "image/png" })
        );
        let images = [
            InlinePart {
                cid: IMAGE_CID.into(),
                reference: upload("aid-1"),
            },
            InlinePart {
                cid: OTHER_CID.into(),
                reference: upload("aid-2"),
            },
        ];
        let settled = settle_draft_inline_images(&images, "400", &inline);
        assert_eq!(
            settled.inline_images,
            [
                Some((
                    IMAGE_CID.to_owned(),
                    DraftPartRef {
                        message_id: "400".into(),
                        part: "1.2.2".into()
                    }
                )),
                None
            ]
        );
        assert_eq!(settled.used, ["aid-1"]);
        assert_eq!(find_body_parts(draft.get("mp")).len(), 2);
    }
}
