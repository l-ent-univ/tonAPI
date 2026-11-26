//! Session CAS déjà ouverte : savoir si elle vit encore (`still_signed_in`) et la fermer
//! (`logout`), contre un faux CAS local qui lie son ticket au `User-Agent`. Aucun test ne contacte
//! un vrai service.

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Response, StatusCode, Uri, header};
use serde_json::json;
use tonapi_core::ErrorCode;
use tonapi_providers::{NetSettings, logout, still_signed_in};
use tonapi_session::{SessionData, StoredCookie};

use common::has_cookie;

/// `User-Agent` de la connexion : le faux CAS ne reconnaît son ticket que de lui.
const CLIENT: &str = "client-de-test";

#[derive(Default)]
struct Mock {
    /// Le CAS répond 503 à tout.
    down: bool,
    /// La session a été fermée par `/cas/logout`.
    closed: bool,
    /// Requêtes reçues : chemin et `User-Agent`.
    seen: Vec<(String, String)>,
}

type Shared = Arc<Mutex<Mock>>;

const LOGIN_PAGE: &str = r#"<html><body><form method="post" id="fm1" action="login">
    <input name="username"><input type="password" name="password">
    <input type="hidden" name="execution" value="E1"></form></body></html>"#;

fn html(status: StatusCode, body: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

async fn handle(
    State(state): State<Shared>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response<Body> {
    let mut s = state.lock().unwrap();
    let agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    s.seen.push((uri.path().to_owned(), agent.clone()));
    if s.down {
        return html(StatusCode::SERVICE_UNAVAILABLE, "<p>Maintenance</p>");
    }
    // Le ticket ne vaut que pour le `User-Agent` qui s'est connecté.
    let known = has_cookie(&headers, "TGC=tgt") && agent == CLIENT && !s.closed;
    match (method, uri.path()) {
        (Method::GET, "/cas/login") if known => html(StatusCode::OK, "<p>Connexion réussie</p>"),
        (Method::GET, "/cas/login") => html(StatusCode::OK, LOGIN_PAGE),
        (Method::GET, "/cas/logout") => {
            s.closed |= known;
            html(StatusCode::OK, "<p>Déconnexion réussie</p>")
        }
        _ => html(StatusCode::NOT_FOUND, ""),
    }
}

async fn start() -> (Shared, tonapi_config::ResolvedUniversity) {
    let (listener, base) = common::listen().await;
    let state: Shared = Arc::default();
    common::spawn(
        listener,
        Router::new().fallback(handle).with_state(state.clone()),
    );
    let university = common::university(json!({
        "schema-version": 1,
        "id": "universite-test",
        "name": "Université de test",
        "timezone": "Europe/Paris",
        "sso": { "kind": "cas", "base-url": format!("{base}/cas/") },
        "services": {}
    }));
    (state, university)
}

/// Session d'un client qui a gardé le TGC.
fn session(cookies: Vec<StoredCookie>) -> SessionData {
    common::session("universite-test", "etu", cookies)
}

fn ticket() -> Vec<StoredCookie> {
    vec![common::stored_cookie("TGC", "tgt", "/cas/")]
}

fn net(user_agent: Option<&'static str>) -> NetSettings {
    NetSettings {
        https_only: false,
        user_agent,
    }
}

#[tokio::test]
async fn le_cas_reconnait_la_session_tant_que_le_ticket_vit() {
    let (state, university) = start().await;
    let held = session(ticket());
    assert_eq!(
        still_signed_in(&university, &held, net(Some(CLIENT))).await,
        Ok(true)
    );
    // Sans ticket, le CAS montre son formulaire : il faut se reconnecter.
    assert_eq!(
        still_signed_in(&university, &session(Vec::new()), net(Some(CLIENT))).await,
        Ok(false)
    );
    // La question ne change pas la session et n'ouvre aucun service.
    assert_eq!(held, session(ticket()));
    let seen = state.lock().unwrap().seen.clone();
    assert!(seen.iter().all(|(path, _)| path == "/cas/login"));
}

#[tokio::test]
async fn un_autre_user_agent_que_celui_de_la_connexion_n_est_pas_reconnu() {
    let (_, university) = start().await;
    // Sans réglage, tonAPI se présente sous son propre `User-Agent` : le ticket d'un autre
    // programme n'est pas le sien.
    assert_eq!(
        still_signed_in(&university, &session(ticket()), net(None)).await,
        Ok(false)
    );
}

#[tokio::test]
async fn une_panne_du_cas_ne_dit_rien_de_la_session() {
    let (state, university) = start().await;
    state.lock().unwrap().down = true;
    let problem = still_signed_in(&university, &session(ticket()), net(Some(CLIENT)))
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::UpstreamChanged);
}

#[tokio::test]
async fn la_deconnexion_se_presente_comme_la_connexion() {
    let (state, university) = start().await;
    let held = session(ticket());

    // Sous un autre `User-Agent`, le CAS ne ferme rien.
    logout(&university, &held, net(None)).await;
    assert!(!state.lock().unwrap().closed);
    assert_eq!(
        still_signed_in(&university, &held, net(Some(CLIENT))).await,
        Ok(true)
    );

    logout(&university, &held, net(Some(CLIENT))).await;
    assert!(state.lock().unwrap().closed);
    assert_eq!(
        still_signed_in(&university, &held, net(Some(CLIENT))).await,
        Ok(false)
    );
    let seen = state.lock().unwrap().seen.clone();
    assert!(seen.contains(&("/cas/logout".to_owned(), CLIENT.to_owned())));
}
