//! Fournisseur `bluemind` de bout en bout contre un faux webmail local : CAS, `webapp/mail/`,
//! `webapp/session-infos.js` et l'API REST. Valeurs inventées.

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Response, StatusCode, Uri, header};
use chrono::{TimeZone, Utc};
use tonapi_config::ResolvedUniversity;
use tonapi_core::{ErrorCode, MailInbox, Problem};
use tonapi_providers::{Context, NetSettings, mail_inbox};
use tonapi_session::SessionData;

use common::{has_cookie, query, redirect};

const DOMAIN: &str = "etu.example.test";
const USER: &str = "0A1B2C3D-0000-0000-0000-000000000000";
const INBOX: &str = "96d1e3c4-0000-0000-0000-000000000000";

#[derive(Default)]
struct Mock {
    base: String,
    /// Numéro de la clé de session en cours ; une clé plus ancienne est refusée (401).
    generation: u32,
    sign_ins: u32,
    api_calls: u32,
}

type Shared = Arc<Mutex<Mock>>;

fn reply(
    status: StatusCode,
    content_type: &str,
    cookie: Option<&str>,
    body: String,
) -> Response<Body> {
    let mut r = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type);
    if let Some(cookie) = cookie {
        r = r.header(header::SET_COOKIE, cookie);
    }
    r.body(Body::from(body)).unwrap()
}

async fn handle(
    State(state): State<Shared>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response<Body> {
    let mut s = state.lock().unwrap();
    let base = s.base.clone();
    let path = uri.path().to_owned();
    match path.as_str() {
        "/cas/login" => match query(&uri, "service") {
            Some(service) if has_cookie(&headers, "TGC=TGT-demo") => {
                redirect(&format!("{service}?ticket=ST-1"))
            }
            _ => reply(
                StatusCode::OK,
                "text/html",
                None,
                r#"<form method="post" id="fm1" action="login"><input name="username"><input type="password" name="password"><input type="hidden" name="execution" value="e1"></form>"#.to_owned(),
            ),
        },
        "/webapp/mail/" => {
            if query(&uri, "ticket").is_some() {
                s.sign_ins += 1;
                s.generation += 1;
                return redirect(&format!("{base}/webapp/mail/"));
            }
            if s.generation == 0 {
                let service: String =
                    url::form_urlencoded::byte_serialize(format!("{base}/webapp/mail/").as_bytes())
                        .collect();
                return redirect(&format!("{base}/cas/login?service={service}"));
            }
            reply(StatusCode::OK, "text/html", Some("BMHPS=ok; Path=/"), "<title>BlueMind</title>".to_owned())
        }
        "/webapp/session-infos.js" => reply(
            StatusCode::OK,
            "application/javascript",
            None,
            format!(
                r#"var bmcSessionInfos = {{ "login": "camille.demo", "sid": "sid-{}", "userId": "{USER}", "domain": "{DOMAIN}" }};"#,
                s.generation
            ),
        ),
        p if p.starts_with("/api/") => {
            s.api_calls += 1;
            let expected = format!("sid-{}", s.generation);
            if headers.get("X-BM-ApiKey").and_then(|v| v.to_str().ok()) != Some(expected.as_str()) {
                return reply(StatusCode::UNAUTHORIZED, "application/json", None, "{}".to_owned());
            }
            let folders = format!("/api/mail_folders/etu_example_test/user.{USER}/_all");
            let answer = if p == folders && method == Method::GET {
                format!(r#"[{{"uid":"aaaa-0000","value":{{"fullName":"Sent"}}}},{{"uid":"{INBOX}","value":{{"fullName":"INBOX"}}}}]"#)
            } else if p == format!("/api/mail_items/{INBOX}/_sorted") {
                if body.contains("Seen") { "[13]".to_owned() } else { "[13,12]".to_owned() }
            } else if p == format!("/api/mail_items/{INBOX}/_mgetById") {
                assert_eq!(body, "[13,12]");
                r#"[
                  {"internalId":12,"value":{"flags":["Seen"],"body":{"subject":"Inscriptions","date":1793120000000,"preview":"Bonjour","recipients":[{"kind":"Originator","address":"scolarite@example.test","dn":"Scolarité"}]}}},
                  {"internalId":13,"value":{"flags":[],"body":{"subject":"Réunion","date":1793200000000,"recipients":[{"kind":"Originator","address":"bde@example.test","dn":"BDE"}]}}}
                ]"#.to_owned()
            } else {
                return reply(StatusCode::NOT_FOUND, "text/plain", None, String::new());
            };
            reply(StatusCode::OK, "application/json", None, answer)
        }
        _ => reply(StatusCode::NOT_FOUND, "text/plain", None, String::new()),
    }
}

async fn start() -> (Shared, String) {
    let (listener, base) = common::listen().await;
    let state: Shared = Arc::new(Mutex::new(Mock {
        base: base.clone(),
        ..Mock::default()
    }));
    let app = Router::new().fallback(handle).with_state(state.clone());
    common::spawn(listener, app);
    (state, base)
}

fn university(base: &str) -> ResolvedUniversity {
    common::university(serde_json::json!({
        "id": "test-bm",
        "name": "Université de test",
        "sso": { "kind": "cas", "base-url": format!("{base}/cas/") },
        "services": { "mail": { "provider": "bluemind", "base-url": format!("{base}/") } },
    }))
}

fn session(tgc: bool) -> SessionData {
    let cookies = if tgc {
        vec![common::stored_cookie("TGC", "TGT-demo", "/")]
    } else {
        Vec::new()
    };
    common::session("test-bm", "camille.demo", cookies)
}

async fn inbox(
    university: &ResolvedUniversity,
    session: &mut SessionData,
) -> Result<MailInbox, Problem> {
    let mut ctx = Context {
        university,
        session,
        now: Utc.with_ymd_and_hms(2026, 10, 28, 8, 0, 0).unwrap(),
        net: NetSettings { https_only: false },
    };
    mail_inbox(&mut ctx).await.map(|f| f.data)
}

#[tokio::test]
async fn boite_de_reception_par_le_cas_puis_l_api() {
    let (state, base) = start().await;
    let uni = university(&base);
    let mut session = session(true);

    let inbox = inbox(&uni, &mut session).await.unwrap();
    assert_eq!(inbox.unread, 1);
    assert_eq!(inbox.messages.len(), 2);
    assert_eq!(inbox.messages[0].subject, "Réunion");
    assert!(inbox.messages[0].unread);
    assert_eq!(inbox.messages[1].from.name.as_deref(), Some("Scolarité"));
    assert_eq!(inbox.webmail_url, Some(format!("{base}/webapp/mail/")));
    let key = format!("bluemind.{}", base.trim_start_matches("http://"));
    assert!(session.tokens[&key].contains("sid-1"));
    assert_eq!(state.lock().unwrap().sign_ins, 1);
}

#[tokio::test]
async fn cle_perimee_relue_une_seule_fois() {
    let (state, base) = start().await;
    let uni = university(&base);
    let mut session = session(true);
    inbox(&uni, &mut session).await.unwrap();

    // Le serveur oublie la clé : nouvelle connexion par le CAS, un seul nouvel essai.
    state.lock().unwrap().generation = 0;
    let again = inbox(&uni, &mut session).await.unwrap();
    assert_eq!(again.messages.len(), 2);
    assert_eq!(state.lock().unwrap().sign_ins, 2);
}

#[tokio::test]
async fn sans_session_cas_la_session_est_expiree() {
    let (_state, base) = start().await;
    let uni = university(&base);
    let problem = inbox(&uni, &mut session(false)).await.unwrap_err();
    assert_eq!(problem.code, ErrorCode::SessionExpired);
}
