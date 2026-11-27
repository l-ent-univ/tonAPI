//! Liste de la boîte et fil de discussion, tels que la page de Tafac les attend (`/mail/recent`,
//! `/mail/thread`). Repris de `server/entAuthApp.js` (`normalizeZimbraMessage`,
//! `normalizeZimbraConversation`, `queryZimbraInbox`, `queryZimbraThread`) et de
//! `server/mailMessage.js` (`dropOwnMailCopies`), sans le réseau : l'appelant envoie les requêtes
//! construites ici et passe les réponses SOAP (ou REST) telles quelles.
//!
//! Écarts voulus avec Tafac (W10 et §4.5 du dossier de sécurité) :
//! - l'objet passe par [`sanitize_subject`] (contrôles et renversements bidi en espaces, 255
//!   points de code) ; l'aperçu perd ses caractères de contrôle ; noms coupés à 256 points de code ;
//! - une adresse reçue qui contient un caractère de contrôle ou de direction, ou de plus de 512
//!   unités, devient `null` : la page fonde son repère de domaine sur `from.email` ;
//! - une ligne dont l'identifiant n'a pas la forme `^-?[0-9]{1,10}$` est laissée de côté (la page
//!   ne pourrait rien en faire), comme les identifiants de `messageIds` qui n'ont pas cette forme ;
//! - les nombres reçus (`u`, `n`) deviennent des entiers positifs bornés.
//!
//! On trouve aussi ici de petites lectures de valeurs JSON « à la JavaScript » (`Number(x)`,
//! `parseInt`, vérité), partagées par les autres modules de Zimbra.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Datelike, SecondsFormat};
use serde_json::{Map, Value, json};

use crate::mail::text::{
    cap_chars, clean_received_name, collapse_spaces, cut_utf16, is_control, is_direction,
    replace_controls, sanitize_subject, trim_js, utf16_len,
};

use super::folders::{INBOX_FOLDER, JUNK_FOLDER, SENT_FOLDER, TRASH_FOLDER, is_valid_folder_id};
use super::search::build_search_query;

// Lectures « à la JavaScript »

/// Vérité d'une valeur au sens de JavaScript (`null`, `false`, `0`, `""` sont faux).
pub(crate) fn js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `asArray` : une liste telle quelle, une autre valeur vraie comme liste d'un élément, sinon rien.
pub(crate) fn as_array(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(list)) => list.iter().collect(),
        Some(other) if js_truthy(other) => vec![other],
        _ => Vec::new(),
    }
}

/// `Number(text)` : `None` quand JavaScript donnerait `NaN`. Chiffres ASCII seulement.
pub(crate) fn js_number_text(text: &str) -> Option<f64> {
    let text = trim_js(text);
    if text.is_empty() {
        return Some(0.0);
    }
    let (negative, unsigned) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    if unsigned == "Infinity" {
        return Some(if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    // `0x…`, `0o…`, `0b…` : sans signe en JavaScript.
    let radix = match text.get(..2) {
        Some("0x" | "0X") => Some(16),
        Some("0o" | "0O") => Some(8),
        Some("0b" | "0B") => Some(2),
        _ => None,
    };
    if let Some(radix) = radix {
        let digits = text.get(2..).unwrap_or_default();
        if digits.is_empty() {
            return None;
        }
        let mut value = 0.0_f64;
        for c in digits.chars() {
            value = value * f64::from(radix) + f64::from(c.to_digit(radix)?);
        }
        return Some(value);
    }
    // Décimal : chiffres, une partie décimale, un exposant.
    let bytes = unsigned.as_bytes();
    let mut index = 0;
    let integer = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    index += integer;
    let mut fraction = 0;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        fraction = bytes[index..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        index += fraction;
    }
    if integer + fraction == 0 {
        return None;
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent = bytes[index..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if exponent == 0 {
            return None;
        }
        index += exponent;
    }
    if index != bytes.len() {
        return None;
    }
    let value: f64 = unsigned.parse().ok()?;
    Some(if negative { -value } else { value })
}

/// `Number(value)` pour une valeur JSON : `None` quand JavaScript donnerait `NaN` (listes et
/// objets compris).
pub(crate) fn js_number(value: &Value) -> Option<f64> {
    match value {
        Value::Null => Some(0.0),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Number(n) => n.as_f64(),
        Value::String(s) => js_number_text(s),
        Value::Array(_) | Value::Object(_) => None,
    }
}

/// `Number.parseInt(text, 10)` : espaces de tête, un signe, puis les chiffres ASCII qui suivent.
/// `None` sans chiffre (`NaN`).
pub(crate) fn js_parse_int(text: &str) -> Option<f64> {
    let text = text.trim_start_matches(crate::mail::text::is_js_space);
    let (negative, rest) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let digits: Vec<u8> = rest.bytes().take_while(u8::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let value = digits
        .iter()
        .fold(0.0_f64, |sum, digit| sum * 10.0 + f64::from(digit - b'0'));
    Some(if negative { -value } else { value })
}

/// `String(value)` pour une chaîne, un nombre ou un booléen ; `None` pour `null` et pour une liste
/// ou un objet (JavaScript en ferait `"[object Object]"`, qu'aucune règle d'ici n'accepte).
pub(crate) fn js_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(js_number_string(n)),
        _ => None,
    }
}

fn js_number_string(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    match n.as_f64() {
        Some(f) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{f:.0}"),
        Some(f) => f.to_string(),
        None => String::new(),
    }
}

/// Un identifiant tel que Zimbra l'écrit : une chaîne, ou un entier ; vide sinon.
pub(crate) fn id_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => n.to_string(),
        _ => String::new(),
    }
}

/// Un entier positif tiré d'un nombre reçu : fini, positif, arrondi vers le bas, borné ; 0 sinon.
pub(crate) fn positive_count(value: Option<&Value>) -> u64 {
    match value.and_then(js_number) {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "valeur finie, positive et bornée"
        )]
        Some(n) if n.is_finite() && n > 0.0 => n.floor().min(MAX_SAFE_INTEGER) as u64,
        _ => 0,
    }
}

/// `Number.MAX_SAFE_INTEGER`.
pub(crate) const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

// Identifiants

/// Identifiant d'un message ou d'un fil : `^-?[0-9]{1,10}$` (chiffres ASCII ; un fil d'un seul
/// mail a l'identifiant virtuel `-<id du mail>`).
#[must_use]
pub fn is_valid_message_id(id: &str) -> bool {
    let digits = id.strip_prefix('-').unwrap_or(id);
    (1..=10).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
}

// Taille d'une page

/// Messages d'une page quand l'université ne dit rien.
pub const DEFAULT_PAGE_SIZE: usize = 5;
/// Jamais plus de messages par page.
pub const MAX_PAGE_SIZE: usize = 20;
/// La liste sans fin de la page ne va pas au-delà de ce décalage.
pub const MAX_OFFSET: usize = 5000;
/// Longueur maximale de l'aperçu, en unités UTF-16.
pub const MAX_SNIPPET_LENGTH: usize = 280;
/// Longueur maximale d'un nom de la liste, en points de code.
pub const MAX_ROW_NAME_LENGTH: usize = 256;
/// Une adresse reçue plus longue (en unités UTF-16) n'est pas montrée.
pub const MAX_ROW_EMAIL_LENGTH: usize = 512;

/// `MAIL_MAX_MESSAGES` : la taille d'une page d'après `mail.maxMessages` de l'université (5 par
/// défaut), entre 1 et 20.
#[must_use]
pub fn page_size(configured: Option<f64>) -> usize {
    match configured {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "valeur bornée entre 1 et 20"
        )]
        Some(n) if n.is_finite() && n != 0.0 => n.clamp(1.0, 20.0).floor() as usize,
        _ => DEFAULT_PAGE_SIZE,
    }
}

/// Le paramètre `offset` de `/mail/recent` (`parseInt`, chiffres ASCII) : entre 0 et
/// [`MAX_OFFSET`], 0 quand il est absent ou illisible.
#[must_use]
pub fn parse_offset(raw: Option<&str>) -> usize {
    match raw.and_then(js_parse_int) {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "valeur bornée entre 0 et MAX_OFFSET, exacte en f64"
        )]
        Some(n) if n > 0.0 => n.min(MAX_OFFSET as f64) as usize,
        _ => 0,
    }
}

// Une ligne de la liste

/// `receivedAt` : `toISOString()` d'une date reçue en millisecondes, ou `None`.
fn received_at(value: Option<&Value>) -> Option<(String, i64)> {
    let millis = value.and_then(js_number)?;
    if !millis.is_finite() || millis <= 0.0 || millis > 8.64e15 {
        return None;
    }
    #[allow(
        clippy::cast_possible_truncation,
        reason = "valeur finie bornée par la plus grande date de JavaScript"
    )]
    let millis = millis.trunc() as i64;
    let date = DateTime::from_timestamp_millis(millis)?;
    // Au-delà de l'an 9999, JavaScript écrit l'année sur six chiffres : hors de ce qu'un mail porte.
    if date.year() > 9999 {
        return None;
    }
    Some((date.to_rfc3339_opts(SecondsFormat::Millis, true), millis))
}

/// Une adresse reçue telle que la liste la montre : une chaîne non vide, sans caractère de
/// contrôle ni de direction, 512 unités au plus ; sinon `None`.
fn row_email(value: Option<&Value>) -> Option<String> {
    let email = value.and_then(Value::as_str)?;
    let clean = !email.is_empty()
        && utf16_len(email) <= MAX_ROW_EMAIL_LENGTH
        && !email.chars().any(|c| is_control(c) || is_direction(c));
    clean.then(|| email.to_owned())
}

/// Le nom montré pour une adresse : `p` (nom complet), sinon `d`, sinon l'adresse, nettoyé
/// (`cleanReceivedName`) et coupé.
fn row_name(address: &Value, email: Option<&str>) -> String {
    let pick = ["p", "d"]
        .iter()
        .find_map(|key| {
            address
                .get(*key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        })
        .or(email)
        .unwrap_or_default();
    let clean = clean_received_name(pick);
    trim_js(cap_chars(&clean, MAX_ROW_NAME_LENGTH)).to_owned()
}

fn snippet(value: Option<&Value>) -> String {
    let Some(text) = value.and_then(Value::as_str) else {
        return String::new();
    };
    // Un aperçu de Zimbra est court ; un très long est coupé avant le travail.
    let text = replace_controls(cut_utf16(text, MAX_SNIPPET_LENGTH * 16), Some(' '));
    cut_utf16(trim_js(&collapse_spaces(&text)), MAX_SNIPPET_LENGTH).to_owned()
}

fn flags(value: &Value) -> &str {
    value.get("f").and_then(Value::as_str).unwrap_or_default()
}

/// Une ligne et ce qui sert à la trier, à la reconnaître dans un fil.
struct Row {
    json: Map<String, Value>,
    id: String,
    received_at: Option<String>,
    received_ms: Option<i64>,
    from_email: Option<String>,
    subject: String,
}

fn message_row(message: &Value, id: String, webmail_href: Option<&str>) -> Row {
    let addresses = as_list(message.get("e"));
    let sender = addresses
        .iter()
        .find(|address| address.get("t").and_then(Value::as_str) == Some("f"));
    let email = sender.and_then(|sender| row_email(sender.get("a")));
    let recipient = addresses
        .iter()
        .find(|address| address.get("t").and_then(Value::as_str) == Some("t"));
    let to = recipient.map_or(Value::Null, |recipient| {
        let recipient_email = row_email(recipient.get("a"));
        json!({
            "name": row_name(recipient, recipient_email.as_deref()),
            "email": recipient_email,
        })
    });
    let from_name = sender.map_or_else(
        || clean_received_name(email.as_deref().unwrap_or_default()),
        |sender| row_name(sender, email.as_deref()),
    );
    let subject = sanitize_subject(
        message
            .get("su")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let received = received_at(message.get("d"));
    let f = flags(message);
    let mut json = Map::new();
    json.insert("id".into(), Value::String(id.clone()));
    json.insert(
        "from".into(),
        json!({ "name": from_name, "email": email.clone() }),
    );
    json.insert("to".into(), to);
    json.insert("subject".into(), Value::String(subject.clone()));
    json.insert("snippet".into(), Value::String(snippet(message.get("fr"))));
    json.insert(
        "receivedAt".into(),
        received
            .as_ref()
            .map_or(Value::Null, |(iso, _)| Value::String(iso.clone())),
    );
    json.insert("unread".into(), Value::Bool(f.contains('u')));
    json.insert("flagged".into(), Value::Bool(f.contains('f')));
    json.insert("replied".into(), Value::Bool(f.contains('r')));
    json.insert("forwarded".into(), Value::Bool(f.contains('w')));
    json.insert("draft".into(), Value::Bool(f.contains('d')));
    json.insert(
        "messageIds".into(),
        if id.is_empty() {
            json!([])
        } else {
            json!([id.clone()])
        },
    );
    json.insert("count".into(), json!(1));
    json.insert(
        "href".into(),
        webmail_href.map_or(Value::Null, |href| Value::String(href.to_owned())),
    );
    let (received_at, received_ms) =
        received.map_or((None, None), |(iso, ms)| (Some(iso), Some(ms)));
    Row {
        json,
        id,
        received_at,
        received_ms,
        from_email: email,
        subject,
    }
}

/// `Array.isArray(x) ? x : []`.
fn as_list(value: Option<&Value>) -> &[Value] {
    value.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// `normalizeZimbraMessage` : un message de `SearchResponse.m[]` ou de la liste REST, en ligne de
/// la liste (`{id, from, to, subject, snippet, receivedAt, unread, flagged, replied, forwarded,
/// draft, messageIds, count, href}`). `webmail_href` : le lien vers le webmail (`/mail/open`), ou
/// `None` sans webmail configuré.
#[must_use]
pub fn normalize_message(message: &Value, webmail_href: Option<&str>) -> Value {
    Value::Object(message_row(message, id_text(message.get("id")), webmail_href).json)
}

/// Mails d'un fil gardés au plus (`messageIds`) : une sélection de la page n'en agit jamais sur
/// plus.
const MAX_THREAD_HITS: usize = 500;
/// Nombre de mails d'un fil montré au plus.
const MAX_THREAD_COUNT: f64 = 1_000_000.0;

fn conversation_row(conversation: &Value, webmail_href: Option<&str>) -> Row {
    let id = id_text(conversation.get("id"));
    let hits: Vec<String> = as_list(conversation.get("m"))
        .iter()
        .map(|message| id_text(message.get("id")))
        .filter(|hit| !hit.is_empty() && is_valid_message_id(hit) && !hit.starts_with('-'))
        .take(MAX_THREAD_HITS)
        .collect();
    let message_ids = if !hits.is_empty() {
        hits
    } else if let Some(lone) = id.strip_prefix('-') {
        vec![lone.to_owned()]
    } else {
        Vec::new()
    };
    let mut row = message_row(conversation, id, webmail_href);
    let f = flags(conversation);
    let unread_count = conversation.get("u").and_then(js_number).unwrap_or(0.0);
    let n = conversation
        .get("n")
        .and_then(js_number)
        .filter(|n| n.is_finite() && *n >= 1.0)
        .map_or(1.0, |n| n.floor().min(MAX_THREAD_COUNT));
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "valeur positive bornée"
    )]
    let count = (n as u64).max(message_ids.len() as u64).max(1);
    row.json.insert(
        "unread".into(),
        Value::Bool(f.contains('u') || unread_count > 0.0),
    );
    row.json
        .insert("flagged".into(), Value::Bool(f.contains('f')));
    row.json
        .insert("replied".into(), Value::Bool(f.contains('r')));
    row.json
        .insert("forwarded".into(), Value::Bool(f.contains('w')));
    row.json.insert("messageIds".into(), json!(message_ids));
    row.json.insert("count".into(), json!(count));
    row
}

/// `normalizeZimbraConversation` : un fil de `SearchResponse.c[]` (types=conversation), en ligne
/// de la liste : l'expéditeur, l'objet, l'aperçu et la date de son mail le plus récent ;
/// `messageIds` ses mails trouvés dans le dossier, `count` tous ses mails.
#[must_use]
pub fn normalize_conversation(conversation: &Value, webmail_href: Option<&str>) -> Value {
    Value::Object(conversation_row(conversation, webmail_href).json)
}

// La page de la liste

/// Le corps de `SearchRequest` (urn:zimbraMail) d'une page : les fils du dossier `folder`, ou
/// ceux que trouvent les mots `words` ([`super::search::parse_search_words`]). `None` si le
/// dossier n'est pas un identifiant.
#[must_use]
pub fn inbox_search_request(
    offset: usize,
    words: &[String],
    folder: &str,
    page_size: usize,
) -> Option<Value> {
    let query = if words.is_empty() {
        if !is_valid_folder_id(folder) {
            return None;
        }
        format!("inid:{folder}")
    } else {
        build_search_query(words, folder)?
    };
    Some(json!({
        "types": "conversation",
        "query": query,
        "sortBy": "dateDesc",
        "limit": page_size,
        "offset": offset,
    }))
}

/// Le nombre de non lus du dossier n'est demandé (`GetFolderRequest`) que pour la première page,
/// hors recherche.
#[must_use]
pub fn reads_folder_count(offset: usize, words: &[String]) -> bool {
    offset == 0 && words.is_empty()
}

/// Le corps de `GetFolderRequest` (urn:zimbraMail) qui donne le nombre de non lus d'un dossier.
#[must_use]
pub fn folder_count_request(folder: &str) -> Value {
    json!({ "folder": { "l": folder }, "depth": 0 })
}

/// Seule la boîte de réception, hors recherche, a le repli REST quand SOAP est refusé (son URL
/// nomme la boîte).
#[must_use]
pub fn uses_rest_fallback(words: &[String], folder: &str) -> bool {
    words.is_empty() && folder == INBOX_FOLDER
}

/// Chemin du repli REST, sous l'origine du webmail : tout jusqu'à la page voulue, et un de plus
/// pour savoir si d'autres suivent (`limit` = décalage + taille de page + 1).
#[must_use]
pub fn rest_inbox_path(limit: usize) -> String {
    format!("/service/home/~/inbox?fmt=json&limit={limit}")
}

/// Les messages d'une réponse REST (`{m: [...]}`) ; `None` si ce n'est pas un objet JSON.
#[must_use]
pub fn rest_inbox_messages(payload: &Value) -> Option<&[Value]> {
    payload.as_object()?;
    Some(as_list(payload.get("m")))
}

/// Ce que la page de la liste est faite : les fils d'une `SearchResponse`, ou les messages du
/// repli REST.
#[derive(Debug, Clone, Copy)]
pub enum InboxRows<'a> {
    /// `Body.SearchResponse` (types=conversation).
    Threads(&'a Value),
    /// Les messages de [`rest_inbox_messages`], et le décalage de la page (REST n'en a pas).
    Rest {
        /// Messages reçus.
        messages: &'a [Value],
        /// Où la page commence.
        offset: usize,
    },
}

/// Nombre de non lus d'une `GetFolderResponse` : `folder[0].u` (ou `folder.u`), 0 quand il est
/// illisible ; `None` sans dossier.
#[must_use]
pub fn folder_unread_count(response: &Value) -> Option<u64> {
    let folder = match response.get("folder")? {
        Value::Array(list) => list.first()?,
        other => other,
    };
    folder.as_object()?;
    Some(positive_count(folder.get("u")))
}

/// `queryZimbraInbox` après les requêtes : `{unreadCount, webmailHref, messages, hasMore}`, ce que
/// `/mail/recent` met dans `mail`. Les lignes sans identifiant valable sont laissées de côté, les
/// autres triées du plus récent au plus ancien, `page_size` au plus.
#[must_use]
pub fn inbox_page(
    rows: InboxRows<'_>,
    folder_response: Option<&Value>,
    page_size: usize,
    webmail_href: Option<&str>,
) -> Value {
    let (mut list, has_more, page_start): (Vec<Row>, bool, usize) = match rows {
        InboxRows::Threads(search) => (
            as_list(search.get("c"))
                .iter()
                .map(|conversation| conversation_row(conversation, webmail_href))
                .collect(),
            search.get("more") == Some(&Value::Bool(true)),
            0,
        ),
        InboxRows::Rest { messages, offset } => (
            messages
                .iter()
                .map(|message| message_row(message, id_text(message.get("id")), webmail_href))
                .collect(),
            messages.len() > offset.saturating_add(page_size),
            offset,
        ),
    };
    list.retain(|row| !row.id.is_empty() && is_valid_message_id(&row.id));
    list.sort_by(|left, right| {
        right
            .received_at
            .as_deref()
            .unwrap_or_default()
            .cmp(left.received_at.as_deref().unwrap_or_default())
    });
    let messages: Vec<Value> = list
        .into_iter()
        .skip(page_start)
        .take(page_size)
        .map(|row| Value::Object(row.json))
        .collect();
    json!({
        "unreadCount": folder_response.and_then(folder_unread_count),
        "webmailHref": webmail_href,
        "messages": messages,
        "hasMore": has_more,
    })
}

/// La réponse de `/mail/recent` sans session : `{authenticated: false, mail: null}`.
#[must_use]
pub fn recent_signed_out() -> Value {
    json!({ "authenticated": false, "mail": null })
}

/// La réponse de `/mail/recent` : `{authenticated: true, sessionMode, mail}`. `searching` : une
/// recherche, sans nombre de non lus (`unreadCount: null`).
#[must_use]
pub fn recent_answer(mut mail: Value, session_mode: Option<&str>, searching: bool) -> Value {
    if searching && let Some(object) = mail.as_object_mut() {
        object.insert("unreadCount".into(), Value::Null);
    }
    json!({
        "authenticated": true,
        "sessionMode": session_mode,
        "mail": mail,
    })
}

// Le fil

/// Le corps de `GetConvRequest` (urn:zimbraMail) d'un fil, avec l'en-tête `Message-ID` de chaque
/// mail (pour reconnaître un mail envoyé à soi-même).
#[must_use]
pub fn thread_request(id: &str) -> Value {
    json!({ "c": { "id": id, "header": [{ "n": "Message-ID" }] } })
}

/// `readZimbraMessageIdHeader` : l'en-tête `Message-ID` d'un mail (dans `_attrs`, ou la liste
/// `header`), sinon le `mid` de Zimbra ; sans espaces de bord, en minuscules.
#[must_use]
pub fn message_id_header(message: &Value) -> Option<String> {
    let from_attrs = message
        .get("_attrs")
        .and_then(|attrs| attrs.get("Message-ID"));
    let from_list = message
        .get("header")
        .and_then(Value::as_array)
        .and_then(|list| {
            list.iter()
                .find(|header| header.get("n").and_then(Value::as_str) == Some("Message-ID"))
        })
        .and_then(|header| header.get("_content"));
    [from_attrs, from_list, message.get("mid")]
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|candidate| !trim_js(candidate).is_empty())
        .map(|value| trim_js(value).to_lowercase())
}

/// Une ligne du fil, et ce qui sert à reconnaître une copie.
struct ThreadRow {
    row: Row,
    folder: String,
    message_id_header: Option<String>,
}

/// `dropOwnMailCopies` : un mail envoyé à soi-même est deux fois dans son fil (la copie d'Envoyés
/// et celle reçue). Une seule est montrée : celle du dossier lu, sinon la reçue. Les copies se
/// reconnaissent à leur `Message-ID` ; sans lui, à une copie envoyée de même expéditeur, objet et
/// seconde qu'une autre.
fn drop_own_copies(rows: Vec<ThreadRow>, folder: &str) -> Vec<ThreadRow> {
    let key_of = |row: &ThreadRow| -> Option<String> {
        if let Some(header) = &row.message_id_header {
            return Some(format!("id:{header}"));
        }
        let email = row.row.from_email.as_deref().filter(|e| !e.is_empty())?;
        let millis = row.row.received_ms?;
        // `Math.round(time / 1000)`, le temps étant positif.
        let seconds = (millis + 500) / 1000;
        Some(format!(
            "same:{}|{}|{seconds}",
            email.to_lowercase(),
            row.row.subject
        ))
    };
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        let Some(key) = key_of(row) else { continue };
        groups
            .entry(key.clone())
            .or_insert_with(|| {
                order.push(key);
                Vec::new()
            })
            .push(index);
    }
    let mut dropped: HashSet<usize> = HashSet::new();
    for key in &order {
        let Some(copies) = groups.get(key) else {
            continue;
        };
        if copies.len() < 2 {
            continue;
        }
        let in_folder = |index: &&usize, wanted: &str| {
            rows.get(**index).is_some_and(|row| row.folder == wanted)
        };
        let sent = copies.iter().any(|i| in_folder(&i, SENT_FOLDER));
        let received = copies.iter().any(|i| !in_folder(&i, SENT_FOLDER));
        // Sans l'en-tête, seule une copie envoyée à côté d'une reçue compte.
        if key.starts_with("same:") && !(sent && received) {
            continue;
        }
        let kept = copies
            .iter()
            .find(|i| in_folder(i, folder))
            .or_else(|| copies.iter().find(|i| !in_folder(i, SENT_FOLDER)))
            .or_else(|| copies.first())
            .copied();
        dropped.extend(copies.iter().copied().filter(|i| Some(*i) != kept));
    }
    rows.into_iter()
        .enumerate()
        .filter(|(index, _)| !dropped.contains(index))
        .map(|(_, row)| row)
        .collect()
}

/// `queryZimbraThread` après `GetConvRequest` : les mails du fil, du plus ancien au plus récent,
/// en lignes de la liste avec leur dossier (`folder`). Ceux de la corbeille et des indésirables
/// sont laissés de côté sauf si le fil est lu de là (`folder`). Un mail envoyé à soi-même n'est
/// montré qu'une fois.
///
/// `response` : `Body.GetConvResponse`, ou `None` quand Zimbra a refusé le fil. L'appelant ne
/// passe `None` que pour un fil virtuel (`id` commençant par `-`) : une autre erreur se propage.
/// Le fil virtuel d'un mail seul que Zimbra ne donne pas devient ce mail seul.
#[must_use]
pub fn thread_rows(
    response: Option<&Value>,
    id: &str,
    folder: &str,
    webmail_href: Option<&str>,
) -> Vec<Value> {
    let conversation = response.and_then(|response| match response.get("c") {
        Some(Value::Array(list)) => list.first(),
        other => other,
    });
    let messages = conversation.map_or(&[][..], |c| as_list(c.get("m")));
    let mut rows: Vec<ThreadRow> = messages
        .iter()
        .filter_map(|message| {
            let location = id_text(message.get("l"));
            if location != folder && (location == TRASH_FOLDER || location == JUNK_FOLDER) {
                return None;
            }
            let row = message_row(message, id_text(message.get("id")), webmail_href);
            if row.id.is_empty() || !is_valid_message_id(&row.id) {
                return None;
            }
            Some(ThreadRow {
                row,
                folder: if is_valid_folder_id(&location) {
                    location
                } else {
                    String::new()
                },
                message_id_header: message_id_header(message),
            })
        })
        .collect();
    rows.sort_by(|left, right| {
        left.row
            .received_at
            .as_deref()
            .unwrap_or_default()
            .cmp(right.row.received_at.as_deref().unwrap_or_default())
    });
    let shown: Vec<Value> = drop_own_copies(rows, folder)
        .into_iter()
        .map(|thread_row| {
            let mut json = thread_row.row.json;
            json.insert("folder".into(), Value::String(thread_row.folder));
            Value::Object(json)
        })
        .collect();
    if !shown.is_empty() {
        return shown;
    }
    match id.strip_prefix('-') {
        Some(lone) => vec![json!({
            "id": lone,
            "messageIds": [lone],
            "count": 1,
            "folder": folder,
        })],
        None => Vec::new(),
    }
}

/// La réponse de `/mail/thread` : `{messages: [...]}`.
#[must_use]
pub fn thread_answer(rows: &[Value]) -> Value {
    json!({ "messages": rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HREF: Option<&str> = Some("/__ent_auth/mail/open");

    #[test]
    fn nombres_a_la_javascript() {
        assert_eq!(js_number_text(" 12 "), Some(12.0));
        assert_eq!(js_number_text(""), Some(0.0));
        assert_eq!(js_number_text("1e3"), Some(1000.0));
        assert_eq!(js_number_text("0x10"), Some(16.0));
        assert_eq!(js_number_text("-0x10"), None);
        assert_eq!(js_number_text("inf"), None);
        assert_eq!(js_number_text("NaN"), None);
        assert_eq!(js_number_text("١٢"), None);
        assert_eq!(js_number_text("-Infinity"), Some(f64::NEG_INFINITY));
        assert_eq!(js_parse_int("  42abc"), Some(42.0));
        assert_eq!(js_parse_int("-5"), Some(-5.0));
        assert_eq!(js_parse_int("١٢"), None);
        assert_eq!(js_parse_int("abc"), None);
        assert_eq!(positive_count(Some(&json!("3.7"))), 3);
        assert_eq!(positive_count(Some(&json!(-2))), 0);
        assert_eq!(positive_count(Some(&json!("many"))), 0);
        assert_eq!(positive_count(Some(&json!(1e300))), 9_007_199_254_740_991);
    }

    #[test]
    fn identifiants_de_message() {
        for good in ["1", "257", "-4211", "1234567890"] {
            assert!(is_valid_message_id(good), "{good}");
        }
        for bad in [
            "",
            "-",
            "12345678901",
            "1a",
            "١٢٣",
            "zid:1",
            "--1",
            " 1",
            "1.0",
        ] {
            assert!(!is_valid_message_id(bad), "{bad}");
        }
    }

    #[test]
    fn taille_de_page_et_decalage() {
        assert_eq!(page_size(None), 5);
        assert_eq!(page_size(Some(0.0)), 5);
        assert_eq!(page_size(Some(50.0)), 20);
        assert_eq!(page_size(Some(-3.0)), 1);
        assert_eq!(page_size(Some(f64::NAN)), 5);
        assert_eq!(parse_offset(Some("40")), 40);
        assert_eq!(parse_offset(Some("-1")), 0);
        assert_eq!(parse_offset(Some("999999")), MAX_OFFSET);
        assert_eq!(parse_offset(Some("١٢")), 0);
        assert_eq!(parse_offset(None), 0);
    }

    fn message() -> Value {
        json!({
            "id": "4211",
            "d": 1_792_998_900_000_i64,
            "f": "ufr",
            "su": "Réunion\u{202e} de rentrée\r\n",
            "fr": "  Bonjour\n\tCamille\u{0}  ",
            "e": [
                { "t": "t", "a": "camille.demo@example.org", "p": "Camille Demo" },
                { "t": "f", "a": "secretariat.demo@example.org", "p": "Secrétariat\u{202e} Demo " },
            ],
        })
    }

    #[test]
    fn une_ligne_de_la_liste() {
        let row = normalize_message(&message(), HREF);
        assert_eq!(row["id"], "4211");
        assert_eq!(
            row["from"],
            json!({ "name": "Secrétariat Demo", "email": "secretariat.demo@example.org" })
        );
        assert_eq!(
            row["to"],
            json!({ "name": "Camille Demo", "email": "camille.demo@example.org" })
        );
        // W10 : objet nettoyé (renversement bidi, fin de ligne).
        assert_eq!(row["subject"], "Réunion de rentrée");
        assert_eq!(row["snippet"], "Bonjour Camille");
        assert_eq!(row["receivedAt"], "2026-10-26T07:15:00.000Z");
        assert_eq!(row["unread"], true);
        assert_eq!(row["flagged"], true);
        assert_eq!(row["replied"], true);
        assert_eq!(row["forwarded"], false);
        assert_eq!(row["draft"], false);
        assert_eq!(row["messageIds"], json!(["4211"]));
        assert_eq!(row["count"], 1);
        assert_eq!(row["href"], "/__ent_auth/mail/open");
        assert_eq!(normalize_message(&message(), None)["href"], Value::Null);
    }

    #[test]
    fn ligne_sans_champs() {
        let row = normalize_message(&json!({ "id": 12 }), HREF);
        assert_eq!(row["id"], "12");
        assert_eq!(row["from"], json!({ "name": "", "email": null }));
        assert_eq!(row["to"], Value::Null);
        assert_eq!(row["subject"], "");
        assert_eq!(row["snippet"], "");
        assert_eq!(row["receivedAt"], Value::Null);
        let row = normalize_message(&json!({ "id": {}, "d": "hier", "e": "x" }), HREF);
        assert_eq!(row["id"], "");
        assert_eq!(row["messageIds"], json!([]));
        assert_eq!(row["receivedAt"], Value::Null);
        // Le nom retombe sur `d`, puis sur l'adresse.
        let row = normalize_message(
            &json!({ "id": "1", "e": [{ "t": "f", "a": "a@b.fr", "d": "Alice" }] }),
            HREF,
        );
        assert_eq!(row["from"]["name"], "Alice");
        let row = normalize_message(
            &json!({ "id": "1", "e": [{ "t": "f", "a": "a@b.fr", "p": "" }] }),
            HREF,
        );
        assert_eq!(row["from"]["name"], "a@b.fr");
    }

    #[test]
    fn adresse_et_nom_bornes() {
        let long = "x".repeat(600);
        let row = normalize_message(
            &json!({ "id": "1", "e": [{ "t": "f", "a": format!("{long}@b.fr"), "p": long }] }),
            HREF,
        );
        assert_eq!(row["from"]["email"], Value::Null);
        assert_eq!(
            row["from"]["name"].as_str().unwrap().chars().count(),
            MAX_ROW_NAME_LENGTH
        );
        // Une adresse qui porte un renversement n'est pas montrée.
        let row = normalize_message(
            &json!({ "id": "1", "e": [{ "t": "f", "a": "evil\u{202e}rf.vinu@x.fr" }] }),
            HREF,
        );
        assert_eq!(row["from"]["email"], Value::Null);
        assert_eq!(row["from"]["name"], "");
        // Objet coupé à 255 points de code.
        let row = normalize_message(&json!({ "id": "1", "su": "é".repeat(400) }), HREF);
        assert_eq!(row["subject"].as_str().unwrap().chars().count(), 255);
    }

    #[test]
    fn un_fil_en_ligne() {
        let conversation = json!({
            "id": "-4211",
            "d": 1_792_998_900_000_i64,
            "n": 3,
            "u": 1,
            "f": "",
            "su": "Fil",
            "m": [{ "id": "4211" }, { "id": "4212" }, { "id": "zid:3" }, { "id": "١٢" }],
            "e": [{ "t": "f", "a": "a@b.fr" }],
        });
        let row = normalize_conversation(&conversation, HREF);
        assert_eq!(row["id"], "-4211");
        assert_eq!(row["messageIds"], json!(["4211", "4212"]));
        assert_eq!(row["count"], 3);
        assert_eq!(row["unread"], true);
        // Sans mail trouvé, le fil virtuel d'un mail seul : ce mail.
        let row = normalize_conversation(&json!({ "id": "-77", "n": "x" }), HREF);
        assert_eq!(row["messageIds"], json!(["77"]));
        assert_eq!(row["count"], 1);
        assert_eq!(row["unread"], false);
        let row = normalize_conversation(
            &json!({ "id": "77", "n": 0, "m": [{ "id": "5" }, { "id": "6" }] }),
            HREF,
        );
        assert_eq!(row["count"], 2);
        let row = normalize_conversation(&json!({ "id": "77", "n": 1e300 }), HREF);
        assert_eq!(row["count"], 1_000_000);
    }

    #[test]
    fn page_de_la_liste_en_fils() {
        let search = json!({
            "more": true,
            "c": [
                { "id": "1", "d": 1000, "su": "ancien" },
                { "id": "2", "d": 3000, "su": "récent" },
                { "id": "bad id", "d": 4000 },
                { "id": "3", "su": "sans date" },
                { "id": "4", "d": 2000 },
            ],
        });
        let folder = json!({ "folder": [{ "id": "2", "u": 7 }] });
        let page = inbox_page(InboxRows::Threads(&search), Some(&folder), 3, HREF);
        assert_eq!(page["unreadCount"], 7);
        assert_eq!(page["webmailHref"], "/__ent_auth/mail/open");
        assert_eq!(page["hasMore"], true);
        let ids: Vec<&str> = page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["2", "4", "1"]);
        // Sans réponse de dossier : `null` ; un dossier sans `u` : 0.
        let page = inbox_page(InboxRows::Threads(&search), None, 3, None);
        assert_eq!(page["unreadCount"], Value::Null);
        assert_eq!(page["webmailHref"], Value::Null);
        let page = inbox_page(
            InboxRows::Threads(&search),
            Some(&json!({ "folder": { "id": "2" } })),
            3,
            None,
        );
        assert_eq!(page["unreadCount"], 0);
        let page = inbox_page(InboxRows::Threads(&search), Some(&json!({})), 3, None);
        assert_eq!(page["unreadCount"], Value::Null);
        // Réponse vide ou illisible.
        let page = inbox_page(InboxRows::Threads(&json!({ "c": "x" })), None, 3, None);
        assert_eq!(page["messages"], json!([]));
        assert_eq!(page["hasMore"], false);
    }

    #[test]
    fn page_du_repli_rest() {
        let messages: Vec<Value> = (1..=8)
            .map(|i| json!({ "id": i.to_string(), "d": i * 1000 }))
            .collect();
        let payload = json!({ "m": messages });
        let list = rest_inbox_messages(&payload).unwrap();
        let page = inbox_page(
            InboxRows::Rest {
                messages: list,
                offset: 3,
            },
            None,
            3,
            HREF,
        );
        let ids: Vec<&str> = page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["5", "4", "3"]);
        assert_eq!(page["hasMore"], true);
        let page = inbox_page(
            InboxRows::Rest {
                messages: list,
                offset: 6,
            },
            None,
            3,
            HREF,
        );
        assert_eq!(page["hasMore"], false);
        assert_eq!(page["messages"].as_array().unwrap().len(), 2);
        assert!(rest_inbox_messages(&json!([1])).is_none());
        assert_eq!(rest_inbox_messages(&json!({ "m": {} })).unwrap().len(), 0);
        assert_eq!(
            rest_inbox_path(21),
            "/service/home/~/inbox?fmt=json&limit=21"
        );
    }

    #[test]
    fn requetes_de_la_liste() {
        let request = inbox_search_request(0, &[], "2", 5).unwrap();
        assert_eq!(
            request,
            json!({ "types": "conversation", "query": "inid:2", "sortBy": "dateDesc", "limit": 5, "offset": 0 })
        );
        let words = vec!["moodle".to_owned()];
        let request = inbox_search_request(10, &words, "5", 5).unwrap();
        assert_eq!(
            request["query"],
            "inid:5 (from:moodle* OR to:moodle* OR subject:moodle* OR content:moodle*)"
        );
        assert!(inbox_search_request(0, &[], "2 OR in:trash", 5).is_none());
        assert!(inbox_search_request(0, &words, "", 5).is_none());
        assert!(inbox_search_request(0, &[], "١", 5).is_none());
        assert!(reads_folder_count(0, &[]));
        assert!(!reads_folder_count(5, &[]));
        assert!(!reads_folder_count(0, &words));
        assert!(uses_rest_fallback(&[], "2"));
        assert!(!uses_rest_fallback(&[], "5"));
        assert!(!uses_rest_fallback(&words, "2"));
        assert_eq!(
            folder_count_request("2"),
            json!({ "folder": { "l": "2" }, "depth": 0 })
        );
    }

    #[test]
    fn reponses_de_recent() {
        assert_eq!(
            recent_signed_out(),
            json!({ "authenticated": false, "mail": null })
        );
        let mail =
            json!({ "unreadCount": 3, "messages": [], "hasMore": false, "webmailHref": null });
        let answer = recent_answer(mail.clone(), None, false);
        assert_eq!(answer["mail"]["unreadCount"], 3);
        assert_eq!(answer["sessionMode"], Value::Null);
        assert_eq!(answer["authenticated"], true);
        let answer = recent_answer(mail, Some("cas"), true);
        assert_eq!(answer["mail"]["unreadCount"], Value::Null);
        assert_eq!(answer["sessionMode"], "cas");
    }

    #[test]
    fn en_tete_message_id() {
        assert_eq!(
            message_id_header(&json!({ "_attrs": { "Message-ID": " <ABC@x> " } })).as_deref(),
            Some("<abc@x>")
        );
        assert_eq!(
            message_id_header(&json!({ "header": [{ "n": "Other", "_content": "x" }, { "n": "Message-ID", "_content": "<B@x>" }] })).as_deref(),
            Some("<b@x>")
        );
        assert_eq!(
            message_id_header(&json!({ "_attrs": { "Message-ID": "  " }, "mid": "<M@x>" }))
                .as_deref(),
            Some("<m@x>")
        );
        assert_eq!(message_id_header(&json!({ "mid": 5 })), None);
        assert_eq!(
            thread_request("-12"),
            json!({ "c": { "id": "-12", "header": [{ "n": "Message-ID" }] } })
        );
    }

    fn thread() -> Value {
        json!({ "c": [{
            "id": "100",
            "m": [
                { "id": "3", "l": "2", "d": 3000, "su": "Re: x", "e": [{ "t": "f", "a": "b@x.fr" }] },
                { "id": "1", "l": "2", "d": 1000, "su": "x", "e": [{ "t": "f", "a": "b@x.fr" }] },
                { "id": "2", "l": "3", "d": 2000, "su": "x", "e": [{ "t": "f", "a": "b@x.fr" }] },
                { "id": "4", "l": "4", "d": 4000 },
                { "id": "x", "l": "2", "d": 5000 },
            ],
        }] })
    }

    #[test]
    fn fil_trie_sans_corbeille_ni_indesirables() {
        let response = thread();
        let rows = thread_rows(Some(&response), "100", "2", HREF);
        let ids: Vec<&str> = rows.iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["1", "3"]);
        assert_eq!(rows[0]["folder"], "2");
        assert!(rows[0].get("messageIdHeader").is_none());
        // Lu depuis la corbeille : ses mails y sont.
        let rows = thread_rows(Some(&response), "100", "3", HREF);
        let ids: Vec<&str> = rows.iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["1", "2", "3"]);
        assert_eq!(
            thread_answer(&rows)["messages"].as_array().unwrap().len(),
            3
        );
    }

    #[test]
    fn fil_virtuel_d_un_mail_seul() {
        assert_eq!(
            thread_rows(None, "-42", "5", HREF),
            vec![json!({ "id": "42", "messageIds": ["42"], "count": 1, "folder": "5" })]
        );
        assert!(thread_rows(None, "42", "2", HREF).is_empty());
        assert!(thread_rows(Some(&json!({ "c": "x" })), "42", "2", HREF).is_empty());
    }

    #[test]
    fn mail_envoye_a_soi_meme_une_fois() {
        let header = |id: &str, folder: &str| json!({ "id": id, "l": folder, "d": 1000, "_attrs": { "Message-ID": "<same@x>" } });
        let response = json!({ "c": { "m": [header("10", "5"), header("11", "2")] } });
        let ids = |folder: &str| -> Vec<String> {
            thread_rows(Some(&response), "100", folder, None)
                .iter()
                .map(|m| m["id"].as_str().unwrap().to_owned())
                .collect()
        };
        // Lu depuis la boîte : la reçue ; depuis Envoyés : l'envoyée.
        assert_eq!(ids("2"), ["11"]);
        assert_eq!(ids("5"), ["10"]);
        assert_eq!(ids("300"), ["11"]);

        // Sans en-tête : même expéditeur, objet et seconde, une copie envoyée.
        let same = |id: &str, folder: &str, millis: i64| json!({ "id": id, "l": folder, "d": millis, "su": "s", "e": [{ "t": "f", "a": "Me@x.fr" }] });
        let response =
            json!({ "c": { "m": [same("20", "5", 1_000_400), same("21", "2", 1_000_100)] } });
        let rows = thread_rows(Some(&response), "100", "2", None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], "21");
        // Deux mails reçus semblables restent deux.
        let response =
            json!({ "c": { "m": [same("20", "2", 1_000_400), same("21", "2", 1_000_100)] } });
        assert_eq!(thread_rows(Some(&response), "100", "2", None).len(), 2);
    }

    #[test]
    fn dates_hors_limites() {
        assert_eq!(received_at(Some(&json!(0))), None);
        assert_eq!(received_at(Some(&json!(-5))), None);
        assert_eq!(received_at(Some(&json!(9e15))), None);
        assert_eq!(received_at(Some(&json!(253_402_300_800_000_i64))), None);
        assert_eq!(
            received_at(Some(&json!("1792998900000"))).map(|(iso, _)| iso),
            Some("2026-10-26T07:15:00.000Z".to_owned())
        );
        assert_eq!(received_at(Some(&json!(1.9))).map(|(_, ms)| ms), Some(1));
    }
}
