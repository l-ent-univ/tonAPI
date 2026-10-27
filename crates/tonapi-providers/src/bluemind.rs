//! Fournisseur `bluemind` : boîte de réception d'un webmail BlueMind.
//!
//! Connexion : le webmail (`webapp/mail/`) passe par le CAS de l'université ; la page
//! `webapp/session-infos.js` donne alors la clé de session (`sid`), l'utilisateur (`userId`) et
//! le domaine. L'API REST s'appelle ensuite avec l'en-tête `X-BM-ApiKey` :
//! - `GET api/mail_folders/<domaine_>/user.<userId>/_all` : dossiers, dont `INBOX` ;
//! - `POST api/mail_items/<dossier>/_sorted` : identifiants triés, filtrés par drapeaux ;
//! - `POST api/mail_items/<dossier>/_mgetById` : messages (sujet, date, expéditeur, aperçu).
//!
//! La clé est gardée dans la session ; un refus (401, 403) la fait relire une seule fois.
//! [vu] Paul-Valéry, BlueMind 5.7.5, 27/10/2026.

use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tonapi_core::{ErrorCode, MailAddress, MailInbox, MailMessage, Problem};
use url::Url;

use crate::Context;
use crate::net::{Net, upstream};

const PROVIDER: &str = "bluemind";
const MAX_MESSAGES: usize = 20;
const PREVIEW_CHARS: usize = 280;

/// Clé de session BlueMind et identité du compte, lues dans `session-infos.js`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Session {
    sid: String,
    user: String,
    domain: String,
}

fn token_key(base: &Url) -> String {
    match base.port() {
        Some(port) => format!("bluemind.{}:{port}", base.host_str().unwrap_or_default()),
        None => format!("bluemind.{}", base.host_str().unwrap_or_default()),
    }
}

fn root(base_url: &Url) -> Url {
    let mut base = base_url.clone();
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base.set_query(None);
    base.set_fragment(None);
    base
}

// ---------------------------------------------------------------------------------------------
// Récupération
// ---------------------------------------------------------------------------------------------

pub(crate) async fn fetch_inbox(
    ctx: &mut Context<'_>,
    base_url: &Url,
) -> Result<MailInbox, Problem> {
    let tz: Tz = ctx.university.university.timezone.parse().map_err(|_| {
        Problem::new(ErrorCode::Internal).detail("fuseau horaire de l'université inconnu")
    })?;
    let base = root(base_url);
    let key = token_key(&base);
    let stored: Option<Session> = ctx
        .session
        .tokens
        .get(&key)
        .and_then(|raw| serde_json::from_str(raw).ok());

    let mut net = Net::open(ctx, PROVIDER)?;
    let result = read(&mut net, &base, stored, tz).await;
    net.save(ctx);
    let (inbox, session) = result?;
    if let Ok(raw) = serde_json::to_string(&session) {
        ctx.session.tokens.insert(key, raw);
    }
    Ok(inbox)
}

/// Erreur d'un appel : session refusée (à rouvrir une fois) ou autre problème.
enum CallError {
    Refused,
    Problem(Problem),
}

impl From<Problem> for CallError {
    fn from(problem: Problem) -> Self {
        CallError::Problem(problem)
    }
}

async fn read(
    net: &mut Net,
    base: &Url,
    stored: Option<Session>,
    tz: Tz,
) -> Result<(MailInbox, Session), Problem> {
    if let Some(session) = stored {
        match inbox(net, base, &session, tz).await {
            Ok(inbox) => return Ok((inbox, session)),
            Err(CallError::Problem(problem)) => return Err(problem),
            Err(CallError::Refused) => {}
        }
    }
    let session = sign_in(net, base).await?;
    match inbox(net, base, &session, tz).await {
        Ok(inbox) => Ok((inbox, session)),
        Err(CallError::Refused) => Err(Problem::new(ErrorCode::SessionExpired)
            .detail("BlueMind refuse la session juste après la connexion")),
        Err(CallError::Problem(problem)) => Err(problem),
    }
}

/// Ouvre le webmail par le CAS et lit la clé de session.
async fn sign_in(net: &mut Net, base: &Url) -> Result<Session, Problem> {
    let webmail = base
        .join("webapp/mail/")
        .map_err(|_| upstream(PROVIDER, "adresse du webmail illisible"))?;
    net.open_protected(webmail).await?;
    let infos = base
        .join("webapp/session-infos.js")
        .map_err(|_| upstream(PROVIDER, "adresse du webmail illisible"))?;
    let (status, _, body) = net
        .request(Method::GET, infos, HeaderMap::new(), None)
        .await?;
    if !status.is_success() {
        return Err(
            Problem::new(ErrorCode::SessionExpired).detail("le webmail n'a pas ouvert de session")
        );
    }
    parse_session_infos(&body).ok_or_else(|| {
        Problem::new(ErrorCode::SessionExpired).detail("le webmail n'a pas ouvert de session")
    })
}

async fn api(
    net: &mut Net,
    base: &Url,
    session: &Session,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, CallError> {
    let url = base
        .join("api/")
        .and_then(|api| api.join(path))
        .map_err(|_| upstream(PROVIDER, "adresse de l'API illisible"))?;
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    if body.is_some() {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
    }
    let key = HeaderValue::from_str(&session.sid)
        .map_err(|_| upstream(PROVIDER, "clé de session illisible"))?;
    headers.insert("X-BM-ApiKey", key);
    let (status, _, text) = net
        .request(method, url, headers, body.map(|b| b.to_string()))
        .await?;
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(CallError::Refused);
    }
    if status.is_server_error() {
        return Err(Problem::new(ErrorCode::ServiceUnavailable)
            .detail(format!("{PROVIDER} : l'API répond HTTP {status}"))
            .into());
    }
    if !status.is_success() {
        return Err(upstream(PROVIDER, format!("réponse HTTP {status} inattendue")).into());
    }
    serde_json::from_str(&text).map_err(|_| upstream(PROVIDER, "réponse de l'API non JSON").into())
}

async fn inbox(
    net: &mut Net,
    base: &Url,
    session: &Session,
    tz: Tz,
) -> Result<MailInbox, CallError> {
    let owner = format!("user.{}", session.user);
    let domain = session.domain.replace('.', "_");
    let folders = api(
        net,
        base,
        session,
        Method::GET,
        &format!("mail_folders/{domain}/{owner}/_all"),
        None,
    )
    .await?;
    let folder = inbox_uid(&folders)?;
    let ids = api(
        net,
        base,
        session,
        Method::POST,
        &format!("mail_items/{folder}/_sorted"),
        Some(sorted(&["Deleted"])),
    )
    .await?;
    let ids = id_list(&ids)?;
    let unseen = api(
        net,
        base,
        session,
        Method::POST,
        &format!("mail_items/{folder}/_sorted"),
        Some(sorted(&["Deleted", "Seen"])),
    )
    .await?;
    let unread = u32::try_from(id_list(&unseen)?.len()).unwrap_or(u32::MAX);
    let first: Vec<i64> = ids.into_iter().take(MAX_MESSAGES).collect();
    let items = if first.is_empty() {
        Value::Array(Vec::new())
    } else {
        api(
            net,
            base,
            session,
            Method::POST,
            &format!("mail_items/{folder}/_mgetById"),
            Some(json!(first)),
        )
        .await?
    };
    let mut messages = parse_items(&items, tz)?;
    messages.sort_by_key(|m| std::cmp::Reverse(m.date));
    Ok(MailInbox {
        unread,
        messages,
        webmail_url: base.join("webapp/mail/").ok().map(String::from),
    })
}

fn sorted(must_not: &[&str]) -> Value {
    json!({
        "fields": [{ "column": "internal_date", "dir": "Desc" }],
        "filter": { "must": [], "mustNot": must_not },
    })
}

// ---------------------------------------------------------------------------------------------
// Analyse (sans réseau)
// ---------------------------------------------------------------------------------------------

/// Valeur d'une clé `"cle": "valeur"` de `session-infos.js`.
fn js_string(text: &str, key: &str) -> Option<String> {
    let quoted = format!("\"{key}\"");
    let rest = text.get(text.find(&quoted)? + quoted.len()..)?.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let value = &rest[..rest.find('"')?];
    (!value.is_empty()).then(|| value.to_owned())
}

fn parse_session_infos(text: &str) -> Option<Session> {
    let user = js_string(text, "userId")?;
    // L'identifiant sert dans des chemins : on refuse tout caractère inattendu.
    let safe = |v: &str| {
        v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
    };
    let domain = js_string(text, "domain")?;
    (safe(&user) && safe(&domain)).then_some(())?;
    Some(Session {
        sid: js_string(text, "sid")?,
        user,
        domain,
    })
}

fn inbox_uid(folders: &Value) -> Result<String, CallError> {
    folders
        .as_array()
        .and_then(|list| {
            list.iter()
                .find(|f| f.pointer("/value/fullName").and_then(Value::as_str) == Some("INBOX"))
        })
        .and_then(|f| f.get("uid").and_then(Value::as_str))
        .filter(|uid| uid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .map(str::to_owned)
        .ok_or_else(|| upstream(PROVIDER, "dossier INBOX introuvable").into())
}

fn id_list(value: &Value) -> Result<Vec<i64>, CallError> {
    value
        .as_array()
        .map(|list| list.iter().filter_map(Value::as_i64).collect())
        .ok_or_else(|| upstream(PROVIDER, "liste d'identifiants illisible").into())
}

fn text_of(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
}

fn parse_items(items: &Value, tz: Tz) -> Result<Vec<MailMessage>, Problem> {
    let list = items
        .as_array()
        .ok_or_else(|| upstream(PROVIDER, "liste de messages illisible"))?;
    let mut out = Vec::with_capacity(list.len());
    for (n, item) in list.iter().enumerate() {
        let missing = |what: &str| upstream(PROVIDER, format!("message n° {} sans {what}", n + 1));
        let id = item
            .get("internalId")
            .and_then(Value::as_i64)
            .ok_or_else(|| missing("identifiant"))?;
        let body = item
            .pointer("/value/body")
            .ok_or_else(|| missing("corps"))?;
        let millis = body
            .get("date")
            .and_then(Value::as_i64)
            .or_else(|| item.pointer("/value/internalDate").and_then(Value::as_i64))
            .ok_or_else(|| missing("date"))?;
        let date: DateTime<Utc> = Utc
            .timestamp_millis_opt(millis)
            .single()
            .ok_or_else(|| missing("date lisible"))?;
        let from = body
            .get("recipients")
            .and_then(Value::as_array)
            .and_then(|r| {
                r.iter()
                    .find(|p| p.get("kind").and_then(Value::as_str) == Some("Originator"))
            });
        let flags: Vec<&str> = item
            .pointer("/value/flags")
            .or_else(|| item.get("flags"))
            .and_then(Value::as_array)
            .map(|f| f.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        out.push(MailMessage {
            id: id.to_string(),
            subject: text_of(body.get("subject")).unwrap_or_default(),
            from: MailAddress {
                name: text_of(from.and_then(|f| f.get("dn"))),
                address: text_of(from.and_then(|f| f.get("address"))).unwrap_or_default(),
            },
            date: date.with_timezone(&tz).fixed_offset(),
            unread: !flags.contains(&"Seen"),
            preview: text_of(body.get("preview")).map(|p| p.chars().take(PREVIEW_CHARS).collect()),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Europe::Paris;

    const INFOS: &str = r#"
      var bmcSessionInfos = {
        "login": "camille.demo",
        "sid": "00000000-aaaa-bbbb-cccc-000000000000",
        "userId": "0A1B2C3D-0000-0000-0000-000000000000",
        "domain": "etu.example.test",
        "bmVersion" : "5.7.5"
      };"#;

    #[test]
    fn cle_de_session() {
        let s = parse_session_infos(INFOS).unwrap();
        assert_eq!(s.user, "0A1B2C3D-0000-0000-0000-000000000000");
        assert_eq!(s.domain, "etu.example.test");
        assert!(parse_session_infos("<html>connexion</html>").is_none());
        assert!(parse_session_infos(&INFOS.replace("etu.example.test", "../x")).is_none());
    }

    #[test]
    fn messages_dates_expediteur_et_drapeaux() {
        let items = json!([
            { "internalId": 12, "value": { "flags": ["Seen"], "body": {
                "subject": "Réunion   de rentrée", "date": 1_793_120_000_000_i64, "preview": "Bonjour à tous,\n la réunion",
                "recipients": [
                    { "kind": "Primary", "address": "camille.demo@etu.example.test", "dn": "Camille Demo" },
                    { "kind": "Originator", "address": "scolarite@example.test", "dn": "Scolarité" }
                ] } } },
            { "internalId": 13, "value": { "flags": [], "body": { "date": 1_774_746_000_000_i64, "recipients": [] } } }
        ]);
        let messages = parse_items(&items, Paris).unwrap();
        assert_eq!(messages[0].subject, "Réunion de rentrée");
        assert_eq!(messages[0].from.address, "scolarite@example.test");
        assert_eq!(messages[0].from.name.as_deref(), Some("Scolarité"));
        assert!(!messages[0].unread);
        assert_eq!(
            messages[0].preview.as_deref(),
            Some("Bonjour à tous, la réunion")
        );
        assert_eq!(messages[0].date.offset().local_minus_utc(), 3600);
        // Sans sujet ni expéditeur : valeurs vides, non lu ; heure d'été le 29 mars 2026.
        assert_eq!(messages[1].subject, "");
        assert!(messages[1].unread);
        assert_eq!(messages[1].date.offset().local_minus_utc(), 7200);
        assert!(parse_items(&json!([{ "value": {} }]), Paris).is_err());
    }

    #[test]
    fn dossier_inbox() {
        let folders = json!([
            { "uid": "aaaa-1", "value": { "fullName": "Sent" } },
            { "uid": "96d1e3c4-0000-0000-0000-000000000000", "value": { "fullName": "INBOX" } }
        ]);
        assert!(matches!(inbox_uid(&folders), Ok(uid) if uid.starts_with("96d1e3c4")));
        assert!(
            inbox_uid(&json!([{ "uid": "x/../y", "value": { "fullName": "INBOX" } }])).is_err()
        );
    }
}
