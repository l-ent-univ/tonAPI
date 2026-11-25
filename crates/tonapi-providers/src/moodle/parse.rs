//! Analyse pure des réponses de Moodle, sans réseau : erreurs (`errorcode`), réponses AJAX et
//! REST, page web (`sesskey`, connecté ou non), jeton de `login/token.php` et événements → devoirs.

use std::collections::HashSet;

use chrono::TimeZone;
use chrono_tz::Tz;
use serde_json::Value;
use tonapi_core::{ErrorCode, Homework, Problem};
use url::Url;

use super::{AJAX, CallError, WEBSERVICE};
use crate::net::upstream;

/// `errorcode` qui disent « la session web est perdue » (Moodle, tafac et lent-ios).
const SESSION_CODES: [&str; 5] = [
    "servicerequireslogin",
    "requireloginerror",
    "invalidsesskey",
    "sessionerroruser",
    "sessiontimedout",
];
/// `errorcode` qui disent « le jeton est refusé » : `accessexception` couvre aussi un jeton expiré.
const TOKEN_CODES: [&str; 2] = ["invalidtoken", "accessexception"];

/// Fin de libellé ajoutée par Moodle à l'événement (« … doit être rendu »).
const NAME_SUFFIXES: [&str; 5] = [
    "doit être rendu",
    "doit être effectué",
    "est dû",
    "ferme",
    "se termine",
];

/// `errorcode` de Moodle : à la racine (REST, `token.php`) ou dans `exception` (AJAX).
fn error_code(value: &Value) -> Option<&str> {
    value.get("errorcode").and_then(Value::as_str).or_else(|| {
        value
            .get("exception")
            .and_then(|e| e.get("errorcode"))
            .and_then(Value::as_str)
    })
}

/// Ne garde que ce qui peut former un identifiant d'erreur Moodle.
pub(super) fn safe_code(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(48)
        .collect()
}

/// Erreur Moodle → problème. Ni `message` ni `debuginfo` : ils peuvent citer le compte.
fn moodle_error(provider: &str, value: &Value, session_codes: &[&str]) -> CallError {
    let code = error_code(value)
        .map(safe_code)
        .filter(|code| !code.is_empty());
    match code.as_deref() {
        Some(code) if session_codes.contains(&code) => CallError::SessionLost,
        Some(code @ ("servicenotavailable" | "sitemaintenance")) => {
            CallError::Failed(Problem::new(ErrorCode::ServiceUnavailable).detail(format!(
                "{provider} : fonction ou site indisponible (`{code}`)"
            )))
        }
        Some(code) => CallError::Failed(upstream(provider, format!("erreur Moodle `{code}`"))),
        None => CallError::Failed(upstream(provider, "erreur Moodle sans code")),
    }
}

/// Contenu utile d'une réponse de `lib/ajax/service.php`.
///
/// Formes vues : `[{"error":false,"data":…}]`, `[{"error":true,"exception":{…}}]` et, quand toute
/// la requête est refusée, un objet `{"error":"…","errorcode":"…"}`.
pub(super) fn ajax_data(response: Value) -> Result<Value, CallError> {
    ajax_pair(response).map(|(data, _)| data)
}

/// Réponse à un lot de deux appels : le contenu du premier, obligatoire, et celui du second s'il a
/// abouti.
///
/// Moodle répond dans l'ordre des appels et s'arrête au premier qui échoue : un second élément
/// absent ou en erreur ne retire rien au premier.
pub(super) fn ajax_pair(response: Value) -> Result<(Value, Option<Value>), CallError> {
    let (first, second) = match response {
        Value::Array(items) if !items.is_empty() => {
            let mut items = items.into_iter();
            (items.next().unwrap_or_default(), items.next())
        }
        other => (other, None),
    };
    Ok((
        ajax_item(first)?,
        second.and_then(|item| ajax_item(item).ok()),
    ))
}

fn ajax_item(mut item: Value) -> Result<Value, CallError> {
    let refused = item.get("error") == Some(&Value::Bool(true)) || error_code(&item).is_some();
    if refused {
        return Err(moodle_error(AJAX, &item, &SESSION_CODES));
    }
    match item.get_mut("data") {
        Some(data) => Ok(data.take()),
        None => Err(upstream(AJAX, "réponse sans `data`").into()),
    }
}

/// Contenu utile d'une réponse de `webservice/rest/server.php` (erreur : `exception`).
pub(super) fn webservice_data(response: Value) -> Result<Value, CallError> {
    if response.get("exception").is_some() || response.get("errorcode").is_some() {
        return Err(moodle_error(WEBSERVICE, &response, &TOKEN_CODES));
    }
    Ok(response)
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PageState {
    LoggedIn(String),
    /// `<body class="… notloggedin">` : page d'invité ou de connexion.
    NotLoggedIn,
    NoSesskey,
}

pub(super) fn page_state(html: &str) -> PageState {
    if body_is_not_logged_in(html) {
        return PageState::NotLoggedIn;
    }
    sesskey_of(html).map_or(PageState::NoSesskey, PageState::LoggedIn)
}

fn body_is_not_logged_in(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    let Some(start) = lower.find("<body") else {
        return false;
    };
    let end = lower
        .get(start..)
        .and_then(|rest| rest.find('>'))
        .map_or(lower.len(), |e| start + e);
    lower
        .get(start..end)
        .is_some_and(|tag| tag.contains("notloggedin"))
}

/// `"sesskey":"<clé>"` dans le `M.cfg` de la page.
pub(super) fn sesskey_of(html: &str) -> Option<String> {
    const NEEDLE: &str = "\"sesskey\"";
    let mut rest = html;
    while let Some(at) = rest.find(NEEDLE) {
        rest = rest.get(at + NEEDLE.len()..)?;
        let value = rest
            .trim_start()
            .strip_prefix(':')
            .map(str::trim_start)
            .and_then(|v| v.strip_prefix('"'));
        if let Some(value) = value {
            let key: String = value
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            if key.len() >= 6 && value.get(key.len()..).is_some_and(|r| r.starts_with('"')) {
                return Some(key);
            }
        }
    }
    None
}

/// Réponse de `login/token.php` : le jeton, ou un problème sans le message de Moodle.
pub(super) fn parse_token(response: &Value) -> Result<String, Problem> {
    if let Some(token) = response
        .get("token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        return Ok(token.to_owned());
    }
    let code = error_code(response).map(safe_code).unwrap_or_default();
    Err(match code.as_str() {
        "invalidlogin" => Problem::new(ErrorCode::InvalidCredentials),
        "passwordisexpired" | "restoredaccountresetpassword" | "usernotconfirmed" => {
            Problem::new(ErrorCode::AccountActionRequired).detail(format!(
                "Moodle demande une action sur le compte (`{code}`) : à faire sur son site"
            ))
        }
        "sitemaintenance" | "servicenotavailable" | "enablewsdescription" => {
            Problem::new(ErrorCode::ServiceUnavailable)
                .detail(format!("{WEBSERVICE} : jeton indisponible (`{code}`)"))
        }
        "" => upstream(WEBSERVICE, "réponse sans jeton ni code d'erreur"),
        _ => upstream(WEBSERVICE, format!("erreur Moodle `{code}`")),
    })
}

/// Événements de `core_calendar_get_action_events_by_timesort` → devoirs, triés par échéance.
///
/// - `kind` reprend le `modulename` de Moodle (`assign`, `quiz`, `forum`…).
/// - `id`, un titre et une date (`timesort`, sinon `timestart`) sont obligatoires : sinon
///   `upstream_changed`. Un événement dont la date vaut 0 n'a pas d'échéance : ignoré.
/// - Moodle ne renvoie que les événements encore « à faire » : un devoir (`assign`) ou un quiz y
///   figure donc non rendu (`submitted = false`) ; pour les autres modules, l'information manque.
/// - Le même devoir publié dans plusieurs espaces de cours (même titre, même cours, même minute)
///   n'apparaît qu'une fois.
pub(crate) fn parse_homework(
    events: &[Value],
    tz: Tz,
    provider: &str,
) -> Result<Vec<Homework>, Problem> {
    parse_homework_with_upcoming(events, &[], tz, provider)
}

/// Comme [`parse_homework`], complété par les devoirs rendus que montre
/// `core_calendar_get_calendar_upcoming_view` (`upcoming`).
///
/// Un événement à venir compte comme rendu (`submitted = true`) s'il vient d'un devoir ou d'un
/// quiz, n'a plus d'`action` et ne figure pas parmi les « à faire » : Moodle retire l'action une
/// fois le travail déposé. C'est une déduction, pas un statut lu : une action retirée pour une
/// autre raison (activité verrouillée) donne aussi « rendu ».
///
/// `upcoming` est une liste d'appoint : un événement illisible y est ignoré, sans erreur.
pub(crate) fn parse_homework_with_upcoming(
    events: &[Value],
    upcoming: &[Value],
    tz: Tz,
    provider: &str,
) -> Result<Vec<Homework>, Problem> {
    let mut seen_ids = HashSet::new();
    let mut seen_content = HashSet::new();
    let mut homework = Vec::new();
    let mut keep = |item: Homework| {
        let content = (
            item.title.to_lowercase(),
            item.course.clone().unwrap_or_default().to_lowercase(),
            item.due.timestamp() / 60,
        );
        if seen_ids.insert(item.id.clone()) && seen_content.insert(content) {
            homework.push(item);
        }
    };
    for event in events {
        if let Some(item) = parse_event(event, tz, provider)? {
            keep(item);
        }
    }
    for event in upcoming.iter().filter(|event| looks_submitted(event)) {
        if let Ok(Some(item)) = parse_event(event, tz, provider) {
            keep(Homework {
                submitted: Some(true),
                ..item
            });
        }
    }
    homework.sort_by(|a, b| a.due.cmp(&b.due).then_with(|| a.title.cmp(&b.title)));
    Ok(homework)
}

/// Devoir ou quiz à venir sans `action`. La date d'ouverture d'un quiz (`eventtype = open`) n'est
/// pas une échéance : elle ne donne pas de ligne « rendu ».
fn looks_submitted(event: &Value) -> bool {
    let module = event.get("modulename").and_then(Value::as_str);
    matches!(module, Some("assign" | "quiz"))
        && !event.get("action").is_some_and(Value::is_object)
        && event.get("eventtype").and_then(Value::as_str) != Some("open")
}

fn parse_event(event: &Value, tz: Tz, provider: &str) -> Result<Option<Homework>, Problem> {
    if !event.is_object() {
        return Err(upstream(provider, "événement illisible"));
    }
    let id = match event.get("id") {
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::String(text)) if !text.is_empty() => text.clone(),
        _ => return Err(upstream(provider, "événement sans `id`")),
    };
    let timestamp = event
        .get("timesort")
        .or_else(|| event.get("timestart"))
        .and_then(epoch)
        .ok_or_else(|| upstream(provider, "événement sans date (`timesort`)"))?;
    if timestamp <= 0 {
        return Ok(None);
    }
    let due = tz
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|d| d.fixed_offset())
        .ok_or_else(|| upstream(provider, "date d'événement hors limites"))?;
    let title = text_field(event, "activityname")
        .or_else(|| text_field(event, "name").map(|name| clean_name(&name)))
        .ok_or_else(|| upstream(provider, "événement sans titre (`name`)"))?;

    let course = event
        .get("course")
        .and_then(|c| text_field(c, "fullname").or_else(|| text_field(c, "shortname")));
    let description = event
        .get("description")
        .and_then(Value::as_str)
        .map(html_to_text)
        .filter(|text| !text.is_empty());
    // Page du devoir, sinon page de l'action ; une adresse qui n'est pas http(s) est écartée.
    let url = [
        event.get("url"),
        event.get("action").and_then(|a| a.get("url")),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .find(|u| Url::parse(u).is_ok_and(|u| matches!(u.scheme(), "http" | "https")))
    .map(str::to_owned);
    let module = event
        .get("modulename")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|module| !module.is_empty());
    let submitted = match module {
        Some("assign" | "quiz") if event.get("action").is_some_and(Value::is_object) => Some(false),
        _ => None,
    };
    Ok(Some(Homework {
        id,
        title,
        course,
        due,
        description,
        url,
        submitted,
        kind: module.map(str::to_owned),
    }))
}

/// Secondes Unix, en nombre ou en chaîne.
fn epoch(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// Champ texte non vide, entités HTML décodées.
fn text_field(object: &Value, key: &str) -> Option<String> {
    let text = decode_entities(object.get(key)?.as_str()?);
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Retire « doit être rendu », « ferme »… que Moodle ajoute au nom de l'activité.
pub(super) fn clean_name(name: &str) -> String {
    let name = name.trim();
    let total = name.chars().count();
    for suffix in NAME_SUFFIXES {
        let length = suffix.chars().count();
        let Some(keep) = total.checked_sub(length).filter(|keep| *keep > 0) else {
            continue;
        };
        let tail: String = name.chars().skip(keep).collect();
        if tail.to_lowercase() == suffix {
            return name
                .chars()
                .take(keep)
                .collect::<String>()
                .trim_end()
                .to_owned();
        }
    }
    name.to_owned()
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        "laquo" => Some('«'),
        "raquo" => Some('»'),
        _ => {
            let digits = name.strip_prefix('#')?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// `&amp;`, `&#039;`, `&#x27;`… ; une entité inconnue reste telle quelle.
pub(super) fn decode_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('&') {
        out.push_str(rest.get(..start).unwrap_or_default());
        let tail = rest.get(start..).unwrap_or_default();
        // `;` cherché dans les 11 premiers octets seulement : un `&` sans `;` proche ne relit pas
        // toute la fin du texte (quadratique sur des milliers de `&`).
        let decoded = tail
            .as_bytes()
            .iter()
            .take(11)
            .position(|b| *b == b';')
            .and_then(|end| Some((entity(tail.get(1..end)?)?, end + 1)));
        if let Some((character, length)) = decoded {
            out.push(character);
            rest = tail.get(length..).unwrap_or_default();
        } else {
            out.push('&');
            rest = tail.get(1..).unwrap_or_default();
        }
    }
    out.push_str(rest);
    out
}

/// Texte d'un fragment HTML : balises retirées (un espace pour les blocs), entités décodées,
/// espaces réduits.
pub(super) fn html_to_text(html: &str) -> String {
    const BLOCKS: [&str; 12] = [
        "br", "p", "div", "li", "ul", "ol", "tr", "td", "h1", "h2", "h3", "h4",
    ];
    let mut text = String::with_capacity(html.len());
    let mut chars = html.chars();
    while let Some(c) = chars.next() {
        if c != '<' {
            text.push(c);
            continue;
        }
        let tag: String = chars.by_ref().take_while(|t| *t != '>').collect();
        let name = tag
            .trim_start_matches('/')
            .split(|t: char| !t.is_ascii_alphanumeric())
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if BLOCKS.contains(&name.as_str()) {
            text.push(' ');
        }
    }
    decode_entities(&text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
