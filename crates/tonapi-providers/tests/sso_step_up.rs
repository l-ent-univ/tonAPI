//! Second facteur exigé par un seul service (`[sso.mfa] step-up-url`), contre un faux CAS local :
//! la connexion par mot de passe passe sans second facteur, mais le « mail » le demande. Aucun
//! test ne contacte un vrai service.

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Response, Uri, header};
use chrono::Utc;
use serde_json::json;
use tonapi_providers::{LoginInput, LoginOutcome, NetSettings, login};

use common::{has_cookie, query, redirect};

#[derive(Default)]
struct Mock {
    base: String,
    password_posts: u32,
    mail_opened: u32,
    /// Le second facteur est déjà validé dans la session CAS (cookie `otp=ok`).
    otp_done: bool,
}

type Shared = Arc<Mutex<Mock>>;

const LOGIN_PAGE: &str = r#"<html><body><form method="post" id="fm1" action="login">
    <input name="username"><input type="password" name="password">
    <input type="hidden" name="execution" value="E1"></form></body></html>"#;

fn html(body: String, cookie: Option<&str>) -> Response<Body> {
    let mut r = Response::builder().header(header::CONTENT_TYPE, "text/html");
    if let Some(cookie) = cookie {
        r = r.header(header::SET_COOKIE, cookie);
    }
    r.body(Body::from(body)).unwrap()
}

fn otp_page(base: &str) -> String {
    format!(
        r#"<html><head><title>Authentification renforcée</title></head><body>
        <form method="post" id="fm1"><input name="ticket" value="TK" type="hidden">
        <input type="hidden" name="uid_hint" value=""><input name="token"></form>
        <script>start({{"apiUrl":"{base}/otp/","uid":"e1","userHash":"H"}})</script></body></html>"#
    )
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
    let mail = format!("{base}/mail/entry");
    let signed = has_cookie(&headers, "TGC=tgt");
    match (method, uri.path()) {
        (Method::GET, "/cas/login") => {
            let service = query(&uri, "service");
            match service {
                None if signed => html("<p>Connexion réussie</p>".into(), None),
                Some(service) if service == mail && signed && !s.otp_done => {
                    html(otp_page(&base), None)
                }
                Some(service) if signed => redirect(&format!("{service}?ticket=ST-2")),
                _ => html(LOGIN_PAGE.into(), None),
            }
        }
        (Method::POST, "/cas/login") => {
            s.password_posts += 1;
            let ok = url::form_urlencoded::parse(body.as_bytes())
                .any(|(k, v)| k == "password" && v == "s3cret");
            if ok {
                html(
                    "<p>Connexion réussie</p>".into(),
                    Some("TGC=tgt; Path=/cas/; HttpOnly"),
                )
            } else {
                html(LOGIN_PAGE.into(), None)
            }
        }
        (Method::GET, "/mail/entry") if query(&uri, "ticket").is_some() => {
            s.mail_opened += 1;
            html("<p>boîte</p>".into(), Some("ZM_AUTH_TOKEN=z; Path=/"))
        }
        (Method::GET, "/mail/entry") => {
            let service: String = url::form_urlencoded::byte_serialize(mail.as_bytes()).collect();
            redirect(&format!("/cas/login?service={service}"))
        }
        _ => Response::builder().status(404).body(Body::empty()).unwrap(),
    }
}

async fn start(otp_done: bool) -> (Shared, String) {
    let (listener, base) = common::listen().await;
    let state: Shared = Arc::new(Mutex::new(Mock {
        base: base.clone(),
        otp_done,
        ..Mock::default()
    }));
    common::spawn(
        listener,
        Router::new().fallback(handle).with_state(state.clone()),
    );
    (state, base)
}

fn university(base: &str, step_up: bool) -> tonapi_config::ResolvedUniversity {
    let mut mfa = json!({ "api-url": format!("{base}/otp/") });
    if step_up {
        mfa["step-up-url"] = json!(format!("{base}/mail/entry"));
    }
    common::university(json!({
        "schema-version": 1,
        "id": "universite-test",
        "name": "Université de test",
        "timezone": "Europe/Paris",
        "sso": { "kind": "cas", "base-url": format!("{base}/cas/"), "mfa": mfa },
        "services": {}
    }))
}

fn input(password: &str) -> LoginInput<'_> {
    LoginInput {
        username: "etu",
        password,
        establishment: None,
        previous_cookies: Vec::new(),
        now: Utc::now(),
        ttl_secs: 3600,
        net: NetSettings {
            https_only: false,
            user_agent: None,
        },
    }
}

/// Bug du 2026-11-09 : la connexion passait sans second facteur, et le mail renvoyait ensuite
/// `session_expired` à chaque essai. La connexion doit demander le second facteur tout de suite.
#[tokio::test]
async fn le_service_qui_exige_le_second_facteur_le_fait_demander_a_la_connexion() {
    let (state, base) = start(false).await;
    let outcome = login(&university(&base, true), input("s3cret"))
        .await
        .unwrap();
    let LoginOutcome::SecondFactor(pending, _) = outcome else {
        panic!("second facteur attendu");
    };
    assert!(pending.pending);
    let challenge: serde_json::Value =
        serde_json::from_str(pending.tokens.get("otp").unwrap()).unwrap();
    assert_eq!(challenge["password_posted"], true);
    assert!(pending.cookies.iter().any(|c| c.name == "TGC"));
    assert_eq!(state.lock().unwrap().password_posts, 1);
}

#[tokio::test]
async fn second_facteur_deja_valide_connexion_directe() {
    let (state, base) = start(true).await;
    let outcome = login(&university(&base, true), input("s3cret"))
        .await
        .unwrap();
    let LoginOutcome::SignedIn(session) = outcome else {
        panic!("connexion attendue");
    };
    assert!(!session.pending);
    assert_eq!(state.lock().unwrap().mail_opened, 1);
}

#[tokio::test]
async fn sans_step_up_url_rien_ne_change() {
    let (state, base) = start(false).await;
    let outcome = login(&university(&base, false), input("s3cret"))
        .await
        .unwrap();
    assert!(matches!(outcome, LoginOutcome::SignedIn(_)));
    assert_eq!(state.lock().unwrap().mail_opened, 0);
}

#[tokio::test]
async fn mauvais_mot_de_passe_ne_va_pas_jusqu_au_service() {
    let (state, base) = start(false).await;
    let problem = login(&university(&base, true), input("faux"))
        .await
        .unwrap_err();
    assert_eq!(problem.code, tonapi_core::ErrorCode::InvalidCredentials);
    assert_eq!(state.lock().unwrap().mail_opened, 0);
}
