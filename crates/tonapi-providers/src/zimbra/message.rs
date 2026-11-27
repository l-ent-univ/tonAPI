//! Un message pour le lecteur de la page (`server/mailMessage.js` et la route `/mail/message` de
//! Tafac), d'après le `GetMsgResponse` de Zimbra : corps nettoyé (`mail::html`), images en ligne
//! par Content-ID, pièces jointes, en-têtes, et ce que sert la route des parties
//! (`/__ent_auth/mail/part` : nom de fichier, disposition).
//!
//! Les valeurs reçues sont lues comme JavaScript les lisait (`String(x)`, `Number(x)`, vérité),
//! et en plus (W10) l'objet, les noms et les types sont nettoyés et bornés, la profondeur MIME
//! limitée.

use std::fmt::Write as _;

use chrono::{DateTime, NaiveDate, Utc};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Map, Value, json};

use crate::mail::attachments::normalize_attachment_type;
use crate::mail::html::{mail_text_to_html, normalize_cid, sanitize_mail_html};
use crate::mail::text::{
    cap_chars, clean_received_name, cut_utf16, nfkd, sanitize_subject, trim_js,
};

/// Le chemin de la route des parties de Tafac.
pub const MAIL_PART_PATH: &str = "/__ent_auth/mail/part";
/// Les types d'image montrés dans le message (jamais SVG) ; le reste se télécharge.
pub const INLINE_IMAGE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/bmp",
    "image/avif",
];
/// Profondeur MIME lue au plus (W10).
pub const MAX_MIME_DEPTH: usize = 32;
/// Longueur maximale d'un nom reçu (personne, fichier), en points de code (W10).
pub const MAX_RECEIVED_NAME_LENGTH: usize = 256;

// ── Lectures à la JavaScript ─────────────────────────────────────────────

/// `String(x)` pour une valeur JSON (`null` : vide, comme `String(x ?? '')`).
pub(crate) fn js_string(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Number(number)) => {
            if let Some(integer) = number.as_i64() {
                integer.to_string()
            } else if let Some(integer) = number.as_u64() {
                integer.to_string()
            } else {
                let float = number.as_f64().unwrap_or(f64::NAN);
                if float.fract() == 0.0 && float.abs() < 1e21 {
                    format!("{float:.0}")
                } else {
                    float.to_string()
                }
            }
        }
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| js_string(Some(item)))
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_owned(),
    }
}

/// `Number(x)` (`NaN` pour ce qui n'en est pas un).
pub(crate) fn js_number(value: Option<&Value>) -> f64 {
    match value {
        Some(Value::Null) => 0.0,
        Some(Value::Bool(flag)) => f64::from(u8::from(*flag)),
        Some(Value::Number(number)) => number.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(text)) => {
            let text = trim_js(text);
            if text.is_empty() {
                0.0
            } else if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
                u32::from_str_radix(hex, 16).map_or(f64::NAN, f64::from)
            } else if text
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
            {
                text.parse::<f64>().unwrap_or(f64::NAN)
            } else {
                f64::NAN
            }
        }
        None | Some(Value::Array(_) | Value::Object(_)) => f64::NAN,
    }
}

/// La vérité de JavaScript (`if (x)`).
pub(crate) fn js_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// Un nombre de JavaScript en JSON : entier quand il l'est.
pub(crate) fn number_json(number: f64) -> Value {
    if number.fract() == 0.0 && number.abs() <= 9_007_199_254_740_991.0 {
        #[allow(clippy::cast_possible_truncation)]
        let integer = number as i64;
        json!(integer)
    } else {
        json!(number)
    }
}

/// `Array.isArray(x) ? x : []`.
pub(crate) fn as_array(value: Option<&Value>) -> &[Value] {
    match value {
        Some(Value::Array(items)) => items,
        _ => &[],
    }
}

/// Le message d'une réponse `GetMsgResponse` (`m`, liste ou objet), s'il y en a un.
pub(crate) fn first_message(response: &Value) -> Option<&Value> {
    let message = match response.get("m") {
        Some(Value::Array(items)) => items.first(),
        other => other,
    }?;
    js_truthy(Some(message)).then_some(message)
}

// ── Identifiants et liens ────────────────────────────────────────────────

/// `isValidMailPartId` : `^[0-9]+(\.[0-9]+)*$`, 40 caractères au plus.
#[must_use]
pub fn is_valid_part_id(value: &str) -> bool {
    value.len() <= 40
        && !value.is_empty()
        && value
            .split('.')
            .all(|piece| !piece.is_empty() && piece.bytes().all(|b| b.is_ascii_digit()))
}

/// `application/x-www-form-urlencoded`, comme `URLSearchParams`.
fn form_encode(value: &str, out: &mut String) {
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
}

/// `buildMailPartHref` : l'adresse d'une partie par la route de Tafac (même origine),
/// `download=1` pour la télécharger.
#[must_use]
pub fn build_mail_part_href(message_id: &str, part: &str, download: bool) -> String {
    let mut href = format!("{MAIL_PART_PATH}?id=");
    form_encode(message_id, &mut href);
    href.push_str("&part=");
    form_encode(part, &mut href);
    if download {
        href.push_str("&download=1");
    }
    href
}

// ── Disposition d'une partie ─────────────────────────────────────────────

/// Le type et la disposition que la route des parties renvoie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartDisposition {
    /// `Content-Type`.
    pub content_type: String,
    /// `Content-Disposition`.
    pub disposition: String,
}

/// `encodeURIComponent` puis `' ( ) *` encodés aussi.
const FILENAME_STAR: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~');

fn is_type_token(part: &str) -> bool {
    !part.is_empty()
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'+' | b'-'))
}

/// `getMailPartDisposition` : en ligne pour les images que le lecteur montre, en
/// téléchargement pour le reste, avec un nom de fichier sûr. Le nom ASCII de repli ne garde que
/// `[A-Za-z0-9 ._()-]`, **après** NFKD (W4b : NFKD changeait `＂` en `"` après le tri de Node).
#[must_use]
pub fn part_disposition(
    content_type: Option<&str>,
    filename: Option<&str>,
    download: bool,
) -> PartDisposition {
    let written = content_type.unwrap_or_default();
    let kind = trim_js(written.split(';').next().unwrap_or_default()).to_lowercase();
    let inline = !download && INLINE_IMAGE_TYPES.contains(&kind.as_str());
    let replaced: String = filename
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if matches!(c, '\r' | '\n' | '"' | '\\' | '/') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = clean_received_name(&replaced);
    let name = match cut_utf16(&cleaned, 200) {
        "" => "piece-jointe",
        name => name,
    };
    let ascii: String = nfkd(name)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '(' | ')' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let content_type = if inline {
        kind
    } else {
        match kind.split_once('/') {
            Some((main, sub)) if is_type_token(main) && is_type_token(sub) => kind.clone(),
            _ => "application/octet-stream".to_owned(),
        }
    };
    PartDisposition {
        content_type,
        disposition: format!(
            "{}; filename=\"{ascii}\"; filename*=UTF-8''{}",
            if inline { "inline" } else { "attachment" },
            utf8_percent_encode(name, FILENAME_STAR)
        ),
    }
}

/// `unescape()` : `%XX` (Latin-1) et `%uXXXX`, le reste tel quel.
fn js_unescape(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    let hex = |slice: &[char]| -> Option<u32> {
        if slice.iter().all(char::is_ascii_hexdigit) {
            u32::from_str_radix(&slice.iter().collect::<String>(), 16).ok()
        } else {
            None
        }
    };
    while index < chars.len() {
        if chars[index] == '%' {
            if chars.get(index + 1) == Some(&'u')
                && let Some(code) = chars.get(index + 2..index + 6).and_then(hex)
            {
                out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                index += 6;
                continue;
            }
            if let Some(code) = chars.get(index + 1..index + 3).and_then(hex) {
                out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                index += 3;
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

fn skip_js_spaces(chars: &[char], mut index: usize) -> usize {
    while chars
        .get(index)
        .is_some_and(|c| crate::mail::text::is_js_space(*c))
    {
        index += 1;
    }
    index
}

fn starts_with_ignore_case(chars: &[char], at: usize, word: &str) -> bool {
    word.chars().enumerate().all(|(offset, expected)| {
        chars
            .get(at + offset)
            .is_some_and(|c| c.eq_ignore_ascii_case(&expected))
    })
}

/// `readContentDispositionFilename` : le nom de fichier d'un `Content-Disposition` reçu
/// (`filename*` d'abord), ou `None`.
#[must_use]
pub fn read_content_disposition_filename(header: &str) -> Option<String> {
    let chars: Vec<char> = header.chars().collect();
    // filename*\s*=\s*([\w-]+)'[^']*'([^;]+)
    for at in 0..chars.len() {
        if !starts_with_ignore_case(&chars, at, "filename*") {
            continue;
        }
        let mut index = skip_js_spaces(&chars, at + 9);
        if chars.get(index) != Some(&'=') {
            continue;
        }
        index = skip_js_spaces(&chars, index + 1);
        let charset_start = index;
        while chars
            .get(index)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        {
            index += 1;
        }
        if index == charset_start || chars.get(index) != Some(&'\'') {
            continue;
        }
        let charset: String = chars[charset_start..index].iter().collect();
        let Some(language) = chars[index + 1..].iter().position(|c| *c == '\'') else {
            continue;
        };
        let value_start = index + 1 + language + 1;
        let value_end = chars[value_start..]
            .iter()
            .position(|c| *c == ';')
            .map_or(chars.len(), |offset| value_start + offset);
        if value_end == value_start {
            continue;
        }
        let encoded: String = chars[value_start..value_end].iter().collect();
        let encoded = trim_js(&encoded);
        let decoded = if charset.eq_ignore_ascii_case("utf-8") {
            crate::mail::html::decode_uri_component(encoded)
        } else {
            Some(js_unescape(encoded))
        };
        if let Some(decoded) = decoded {
            return Some(decoded);
        }
        break;
    }
    // filename\s*=\s*"([^"]*)" puis filename\s*=\s*([^;\s]+)
    let mut unquoted: Option<String> = None;
    for at in 0..chars.len() {
        if !starts_with_ignore_case(&chars, at, "filename") {
            continue;
        }
        let mut index = skip_js_spaces(&chars, at + 8);
        if chars.get(index) != Some(&'=') {
            continue;
        }
        index = skip_js_spaces(&chars, index + 1);
        if chars.get(index) == Some(&'"')
            && let Some(close) = chars[index + 1..].iter().position(|c| *c == '"')
        {
            return Some(chars[index + 1..index + 1 + close].iter().collect());
        }
        if unquoted.is_none() {
            let end = chars[index..]
                .iter()
                .position(|c| *c == ';' || crate::mail::text::is_js_space(*c))
                .map_or(chars.len(), |offset| index + offset);
            if end > index {
                unquoted = Some(chars[index..end].iter().collect());
            }
        }
    }
    unquoted
}

// ── Le message ────────────────────────────────────────────────────────────

struct Body {
    kind: String,
    content: String,
    truncated: bool,
}

struct Leaf {
    part: String,
    name: String,
    kind: String,
    size: Option<f64>,
    cid: Option<String>,
}

fn received_name(name: &str) -> String {
    cap_chars(&clean_received_name(name), MAX_RECEIVED_NAME_LENGTH).to_owned()
}

fn attachment_name(part: &Value) -> String {
    let filename = received_name(&js_string(part.get("filename")));
    if !filename.is_empty() {
        return filename;
    }
    match part.get("ct").and_then(Value::as_str) {
        Some("message/rfc822") => "Message joint.eml".to_owned(),
        Some("text/calendar") => "Invitation.ics".to_owned(),
        _ => "Pièce jointe".to_owned(),
    }
}

/// `collectParts` : les corps que Zimbra a marqués (`body`), les autres feuilles en pièces jointes
/// (un message joint en est une). Profondeur limitée à `MAX_MIME_DEPTH`.
fn collect_parts(
    parts: Option<&Value>,
    depth: usize,
    bodies: &mut Vec<Body>,
    leaves: &mut Vec<Leaf>,
) {
    if depth > MAX_MIME_DEPTH {
        return;
    }
    for part in as_array(parts) {
        if !matches!(part, Value::Object(_) | Value::Array(_)) {
            continue;
        }
        let part_id = js_string(part.get("part"));
        let kind = js_string(part.get("ct")).to_lowercase();
        if js_truthy(part.get("body"))
            && let Some(content) = part.get("content").and_then(Value::as_str)
        {
            bodies.push(Body {
                kind,
                content: content.to_owned(),
                truncated: js_truthy(part.get("truncated")),
            });
            continue;
        }
        if kind.starts_with("multipart/") {
            collect_parts(part.get("mp"), depth + 1, bodies, leaves);
            continue;
        }
        if !is_valid_part_id(&part_id) {
            continue;
        }
        if !js_truthy(part.get("filename"))
            && part.get("cd").and_then(Value::as_str) != Some("attachment")
            && (kind == "text/plain" || kind == "text/html")
        {
            continue;
        }
        let size = js_number(part.get("s"));
        leaves.push(Leaf {
            part: part_id,
            name: attachment_name(part),
            size: (size != 0.0 && !size.is_nan()).then_some(size),
            cid: js_truthy(part.get("ci")).then(|| normalize_cid(&js_string(part.get("ci")))),
            kind,
        });
    }
}

fn iso_date(milliseconds: f64) -> Option<String> {
    if !milliseconds.is_finite() || milliseconds <= 0.0 || milliseconds > 253_402_300_799_999.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    let date = DateTime::<Utc>::from_timestamp_millis(milliseconds.trunc() as i64)?;
    Some(date.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}

fn addresses(message: &Value, kind: &str) -> Vec<Value> {
    as_array(message.get("e"))
        .iter()
        .filter(|address| address.get("t").and_then(Value::as_str) == Some(kind))
        .map(|address| {
            let name = ["p", "d", "a"]
                .iter()
                .map(|key| address.get(*key))
                .find(|value| js_truthy(*value))
                .flatten();
            json!({
                "name": received_name(&js_string(name)),
                "email": js_truthy(address.get("a")).then(|| js_string(address.get("a"))),
            })
        })
        .collect()
}

/// `buildMailMessage` : le message d'un `GetMsgResponse` pour le lecteur. Clés de Node : `id`,
/// `subject`, `from`, `replyTo`, `to`, `cc`, `receivedAt`, `unread`, `flagged`, `format`
/// (`html` dès qu'un corps l'est), `html`, `remoteImages`, `truncated`, `attachments` (toujours
/// une liste). `own_hosts` : les hôtes de Tafac (W6).
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn build_mail_message(message: &Value, message_id: &str, own_hosts: &[&str]) -> Value {
    let mut bodies = Vec::new();
    let mut leaves = Vec::new();
    collect_parts(message.get("mp"), 0, &mut bodies, &mut leaves);
    let id = match message.get("id") {
        None | Some(Value::Null) => message_id.to_owned(),
        value => js_string(value),
    };
    let images: Vec<(&str, &str)> = leaves
        .iter()
        .filter(|leaf| INLINE_IMAGE_TYPES.contains(&leaf.kind.as_str()))
        .filter_map(|leaf| leaf.cid.as_deref().map(|cid| (cid, leaf.part.as_str())))
        .collect();
    let mut used: Vec<String> = Vec::new();
    let mut remote_images = 0;
    let mut parts = Vec::with_capacity(bodies.len());
    for body in &bodies {
        if body.kind == "text/html" {
            let mut resolve = |cid: &str| {
                let (_, part) = images.iter().rev().find(|(known, _)| *known == cid)?;
                if !used.iter().any(|seen| seen == cid) {
                    used.push(cid.to_owned());
                }
                Some(build_mail_part_href(&id, part, false))
            };
            let clean = sanitize_mail_html(&body.content, own_hosts, &mut resolve);
            remote_images += clean.remote_images;
            parts.push(clean.html);
        } else {
            parts.push(mail_text_to_html(&body.content, own_hosts));
        }
    }
    let attachments: Vec<Value> = leaves
        .iter()
        .filter(|leaf| !leaf.cid.as_ref().is_some_and(|cid| used.contains(cid)))
        .map(|leaf| {
            json!({
                "part": leaf.part,
                "name": leaf.name,
                "type": (!leaf.kind.is_empty()).then(|| normalize_attachment_type(&leaf.kind)),
                "size": leaf.size.map(number_json),
                "href": build_mail_part_href(&id, &leaf.part, true),
            })
        })
        .collect();
    let flags = message.get("f").and_then(Value::as_str);
    let mut out = Map::new();
    out.insert("id".into(), json!(id));
    out.insert(
        "subject".into(),
        json!(
            message
                .get("su")
                .and_then(Value::as_str)
                .map(sanitize_subject)
                .unwrap_or_default()
        ),
    );
    out.insert(
        "from".into(),
        addresses(message, "f")
            .into_iter()
            .next()
            .unwrap_or(Value::Null),
    );
    out.insert(
        "replyTo".into(),
        addresses(message, "r")
            .into_iter()
            .next()
            .unwrap_or(Value::Null),
    );
    out.insert("to".into(), Value::Array(addresses(message, "t")));
    out.insert("cc".into(), Value::Array(addresses(message, "c")));
    out.insert(
        "receivedAt".into(),
        json!(iso_date(js_number(message.get("d")))),
    );
    out.insert(
        "unread".into(),
        json!(flags.is_some_and(|f| f.contains('u'))),
    );
    out.insert(
        "flagged".into(),
        json!(flags.is_some_and(|f| f.contains('f'))),
    );
    out.insert(
        "format".into(),
        json!(if bodies.iter().any(|body| body.kind == "text/html") {
            "html"
        } else {
            "text"
        }),
    );
    out.insert(
        "html".into(),
        json!(parts.join("<hr class=\"tafac-mail-part-break\">")),
    );
    out.insert("remoteImages".into(), json!(remote_images));
    out.insert(
        "truncated".into(),
        json!(bodies.iter().any(|body| body.truncated)),
    );
    out.insert("attachments".into(), Value::Array(attachments));
    Value::Object(out)
}

/// La réponse de la route `/mail/message` : `{message: …}` d'après le `GetMsgResponse`, ou
/// `None` quand il n'a pas de message (404).
#[must_use]
pub fn mail_message_response(
    response: &Value,
    message_id: &str,
    own_hosts: &[&str],
) -> Option<Value> {
    let message = first_message(response)?;
    Some(json!({ "message": build_mail_message(message, message_id, own_hosts) }))
}

// ── Un fil ────────────────────────────────────────────────────────────────

/// `Date.parse` des dates de Tafac (ISO 8601), en millisecondes.
fn parse_date(value: &str) -> Option<i64> {
    if let Ok(date) = DateTime::parse_from_rfc3339(value) {
        return Some(date.timestamp_millis());
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis())
}

/// `dropOwnMailCopies` : un mail envoyé à soi-même est deux fois dans son fil (la copie des
/// Envoyés, la reçue) ; une seule reste, celle du dossier lu (`folder`), sinon la reçue. Copies
/// reconnues à leur `messageIdHeader` ; sans lui, une copie envoyée à côté d'une reçue, même
/// expéditeur, même objet, même seconde. `sent_folder` : l'id des Envoyés (`5`).
#[must_use]
pub fn drop_own_mail_copies(messages: Vec<Value>, folder: &str, sent_folder: &str) -> Vec<Value> {
    let key_of = |message: &Value| -> Option<String> {
        if js_truthy(message.get("messageIdHeader")) {
            return Some(format!("id:{}", js_string(message.get("messageIdHeader"))));
        }
        let email = message.get("from").and_then(|from| from.get("email"));
        if !js_truthy(email) {
            return None;
        }
        let time = parse_date(&js_string(message.get("receivedAt")))?;
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let seconds = ((time as f64) / 1000.0 + 0.5).floor() as i64;
        Some(format!(
            "same:{}|{}|{seconds}",
            js_string(email).to_lowercase(),
            js_string(message.get("subject"))
        ))
    };
    let in_folder = |message: &Value, wanted: &str| {
        message.get("folder").and_then(Value::as_str) == Some(wanted)
    };
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let Some(key) = key_of(message) else { continue };
        match groups.iter_mut().find(|(known, _)| *known == key) {
            Some((_, members)) => members.push(index),
            None => groups.push((key, vec![index])),
        }
    }
    let mut dropped = vec![false; messages.len()];
    for (key, copies) in &groups {
        if copies.len() < 2 {
            continue;
        }
        if key.starts_with("same:")
            && !(copies.iter().any(|i| in_folder(&messages[*i], sent_folder))
                && copies
                    .iter()
                    .any(|i| !in_folder(&messages[*i], sent_folder)))
        {
            continue;
        }
        let kept = copies
            .iter()
            .find(|i| in_folder(&messages[**i], folder))
            .or_else(|| {
                copies
                    .iter()
                    .find(|i| !in_folder(&messages[**i], sent_folder))
            })
            .or_else(|| copies.first())
            .copied();
        for index in copies {
            if Some(*index) != kept {
                dropped[*index] = true;
            }
        }
    }
    messages
        .into_iter()
        .zip(dropped)
        .filter_map(|(message, dropped)| (!dropped).then_some(message))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/mail/zimbra.json"))
            .expect("fixture")
    }

    #[test]
    fn messages_comme_node() {
        for case in fixture()["messages"].as_array().expect("liste") {
            let id = case["id"].as_str().expect("id");
            let got = build_mail_message(&case["input"], id, &[]);
            assert_eq!(got, case["output"], "{}", case["input"]);
            let response = json!({ "m": [case["input"].clone()] });
            assert_eq!(
                mail_message_response(&response, id, &[]),
                Some(json!({ "message": got }))
            );
        }
        assert_eq!(mail_message_response(&json!({}), "1", &[]), None);
        assert_eq!(mail_message_response(&json!({ "m": [] }), "1", &[]), None);
        // `attachments` est toujours une liste.
        let empty = build_mail_message(&json!({}), "7", &[]);
        assert_eq!(empty["attachments"], json!([]));
        assert_eq!(empty["id"], json!("7"));
    }

    #[test]
    fn objet_noms_types_et_profondeur() {
        // W10 : l'objet, les noms et les types nettoyés et bornés ; la profondeur MIME limitée.
        let mut deep =
            json!({ "part": "9", "ct": "text/html", "body": true, "content": "<p>fond</p>" });
        for _ in 0..200 {
            deep = json!({ "ct": "multipart/mixed", "mp": [deep] });
        }
        let message = json!({
            "su": "Re:\u{202e} objet\r\nBcc: x",
            "e": [{ "t": "f", "a": "a@b.fr", "p": "n".repeat(1000) }],
            "mp": [deep, { "part": "2", "ct": "Text/X\r\nY: z", "cd": "attachment", "filename": "f".repeat(1000) }],
        });
        let built = build_mail_message(&message, "1", &[]);
        assert_eq!(built["subject"], json!("Re: objet Bcc: x"));
        assert_eq!(
            built["from"]["name"].as_str().map(|n| n.chars().count()),
            Some(MAX_RECEIVED_NAME_LENGTH)
        );
        assert_eq!(built["html"], json!(""));
        assert_eq!(
            built["attachments"][0]["type"],
            json!("application/octet-stream")
        );
        assert_eq!(
            built["attachments"][0]["name"]
                .as_str()
                .map(|n| n.chars().count()),
            Some(MAX_RECEIVED_NAME_LENGTH)
        );
    }

    #[test]
    fn liens_de_tafac_dans_un_message() {
        let message = json!({ "mp": [{ "part": "1", "ct": "text/html", "body": true, "content": "<a href=\"https://tafac.fr/x\">a</a>" }] });
        let built = build_mail_message(&message, "1", &["tafac.fr"]);
        assert_eq!(
            built["html"],
            json!("<a target=\"_blank\" rel=\"noopener noreferrer\">a</a>")
        );
    }

    #[test]
    fn identifiants_et_liens() {
        assert!(is_valid_part_id("1.2"));
        assert!(is_valid_part_id("10"));
        for id in [
            "TEXT",
            "1&x=2",
            "",
            "1.",
            ".1",
            "1..2",
            "١",
            &"1".repeat(41),
        ] {
            assert!(!is_valid_part_id(id), "{id:?}");
        }
        assert_eq!(
            build_mail_part_href("257", "1.2", false),
            "/__ent_auth/mail/part?id=257&part=1.2"
        );
        assert_eq!(
            build_mail_part_href("-1", "2", true),
            "/__ent_auth/mail/part?id=-1&part=2&download=1"
        );
        assert_eq!(
            build_mail_part_href("a b&c", "é", false),
            "/__ent_auth/mail/part?id=a+b%26c&part=%C3%A9"
        );
    }

    #[test]
    fn dispositions() {
        for case in fixture()["dispositions"].as_array().expect("liste") {
            let got = part_disposition(
                case["contentType"].as_str(),
                case["filename"].as_str(),
                case["download"].as_bool().unwrap_or(false),
            );
            let expected = &case["output"];
            assert_eq!(
                Some(got.content_type.as_str()),
                expected["contentType"].as_str(),
                "{case}"
            );
            // Tout sauf le nom ASCII de repli, que W4b restreint.
            let tail = |d: &str| d.split_once("; filename*=").map(|(_, t)| t.to_owned());
            let kind = |d: &str| d.split(';').next().map(str::to_owned);
            let disposition = expected["disposition"].as_str().expect("disposition");
            assert_eq!(tail(&got.disposition), tail(disposition), "{case}");
            assert_eq!(kind(&got.disposition), kind(disposition), "{case}");
            let ascii = got
                .disposition
                .split_once("filename=\"")
                .and_then(|(_, rest)| rest.split_once('"'))
                .map(|(name, _)| name.to_owned())
                .expect("nom");
            assert!(
                ascii
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || " ._()-".contains(c)),
                "{ascii}"
            );
        }
        // Le test de Node (« a\r\nX: y ») : `:` devient `_` (W4b).
        let odd = part_disposition(Some("application/pdf"), Some("a\r\nX: y"), true);
        assert!(
            odd.disposition
                .starts_with("attachment; filename=\"a__X_ y\""),
            "{}",
            odd.disposition
        );
        // W4b : NFKD ne fait pas apparaître de guillemet ni de barre inverse.
        let nfkd = part_disposition(
            Some("text/html"),
            Some("a\u{ff02}; filename=evil.html\u{ff3c}"),
            false,
        );
        let quoted = nfkd.disposition.split("; filename*=").next().expect("nom");
        assert_eq!(quoted.matches('"').count(), 2, "{quoted}");
        assert!(!nfkd.disposition.contains('\\'));
        assert_eq!(
            nfkd.disposition.matches("filename=").count(),
            1,
            "{}",
            nfkd.disposition
        );
    }

    #[test]
    fn noms_de_fichier_recus() {
        for case in fixture()["filenames"].as_array().expect("liste") {
            let got = read_content_disposition_filename(case["header"].as_str().expect("en-tête"));
            assert_eq!(got.as_deref(), case["output"].as_str(), "{case}");
        }
    }

    #[test]
    fn copies_de_ses_propres_mails() {
        for case in fixture()["copies"].as_array().expect("liste") {
            let list = case["input"].as_array().expect("liste").clone();
            let kept: Vec<Value> =
                drop_own_mail_copies(list, case["folder"].as_str().expect("dossier"), "5")
                    .into_iter()
                    .map(|message| message["id"].clone())
                    .collect();
            assert_eq!(Value::Array(kept), case["output"], "{case}");
        }
    }
}
