//! Profil du portail complété par le dossier `etudiants-api`, contre un faux portail uPortal 4,
//! un faux CAS et une fausse application de scolarité. Valeurs inventées.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Response, StatusCode, Uri, header};
use chrono::{TimeZone, Utc};
use tonapi_config::{ResolvedUniversity, UniversityFile};
use tonapi_providers::{Context, NetSettings, grades, profile};
use tonapi_session::{SessionData, StoredCookie};

const SESSION: &str = include_str!("fixtures/uportal/session_v4.json");
const PEOPLE: &str = include_str!("fixtures/uportal/people_v4.json");
const NOTES: &str = r#"{"numero":"21000000","nom":"DEMO","prenom":"Camille","etapes":[{"libelle":"Licence 1 Arts","codEtape":"XX1","semestres":{"S1":{"intitule":"Semestre 1","classifiedElementPedagogiques":[{"codElement":"UE1","libElement":"Fondamentaux","niveauElement":1,"noteEpreuve1":"12,5","note":"13,25","resultat":"ADM"}]}}}],"epreuves":{}}"#;

#[derive(Default)]
struct Mock {
    base: String,
    /// L'application de scolarité répond 503.
    down: bool,
    notes_calls: u32,
}

type Shared = Arc<Mutex<Mock>>;

fn query(uri: &Uri, key: &str) -> Option<String> {
    url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

fn reply(status: StatusCode, cookie: Option<&str>, body: &str) -> Response<Body> {
    let mut r = Response::builder().status(status);
    if let Some(cookie) = cookie {
        r = r.header(header::SET_COOKIE, cookie);
    }
    r.body(Body::from(body.to_owned())).unwrap()
}

fn redirect(location: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location)
        .body(Body::empty())
        .unwrap()
}

fn has_cookie(headers: &HeaderMap, pair: &str) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .is_some_and(|c| c.split("; ").any(|p| p == pair))
}

async fn handle(State(state): State<Shared>, uri: Uri, headers: HeaderMap) -> Response<Body> {
    let mut s = state.lock().unwrap();
    let base = s.base.clone();
    match uri.path() {
        "/cas/login" => match query(&uri, "service") {
            Some(service) if has_cookie(&headers, "TGC=TGT-demo") => {
                redirect(&format!("{service}?ticket=ST-1"))
            }
            _ => reply(
                StatusCode::OK,
                None,
                "<form id=\"fm1\"><input name=\"execution\"><input type=\"password\" name=\"password\"></form>",
            ),
        },
        "/uPortal/api/session.json" => reply(StatusCode::OK, None, SESSION),
        "/uPortal/api/people/e00000000000.json" => reply(StatusCode::OK, None, PEOPLE),
        "/scolarite/" => {
            if query(&uri, "ticket").is_some() {
                return reply(
                    StatusCode::OK,
                    Some("SCOL=ok; Path=/scolarite"),
                    "<app-root></app-root>",
                );
            }
            let service: String =
                url::form_urlencoded::byte_serialize(format!("{base}/scolarite/").as_bytes())
                    .collect();
            redirect(&format!("{base}/cas/login?service={service}"))
        }
        "/scolarite/api/etudiants/notes" => {
            s.notes_calls += 1;
            if s.down {
                return reply(StatusCode::SERVICE_UNAVAILABLE, None, "");
            }
            if !has_cookie(&headers, "SCOL=ok") {
                return reply(StatusCode::UNAUTHORIZED, None, "");
            }
            reply(StatusCode::OK, None, NOTES)
        }
        _ => reply(StatusCode::NOT_FOUND, None, ""),
    }
}

async fn start(down: bool) -> (Shared, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let state: Shared = Arc::new(Mutex::new(Mock {
        base: base.clone(),
        down,
        ..Mock::default()
    }));
    let app = Router::new().fallback(handle).with_state(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (state, base)
}

fn university(base: &str) -> ResolvedUniversity {
    let file: UniversityFile = serde_json::from_value(serde_json::json!({
        "id": "test-scol",
        "name": "Université de test",
        "sso": { "kind": "cas", "base-url": format!("{base}/cas/") },
        "portal": { "kind": "uportal4-prolongation", "base-url": format!("{base}/uPortal/") },
        "services": {
            "profile": {
                "provider": "portal",
                "record": { "kind": "etudiants-api", "base-url": format!("{base}/scolarite/") }
            },
            "grades": { "provider": "etudiants-api", "base-url": format!("{base}/scolarite/") }
        },
    }))
    .unwrap();
    ResolvedUniversity {
        dir: std::path::PathBuf::new(),
        university: file,
        establishments: BTreeMap::new(),
    }
}

fn session() -> SessionData {
    SessionData {
        university: "test-scol".to_owned(),
        username: "e00000000000".to_owned(),
        establishment: None,
        cookies: vec![StoredCookie {
            domain: "127.0.0.1".to_owned(),
            path: "/".to_owned(),
            name: "TGC".to_owned(),
            value: "TGT-demo".to_owned(),
            host_only: true,
            expires: None,
        }],
        tokens: BTreeMap::new(),
        pending: false,
        issued_at: 0,
        expires_at: i64::MAX,
    }
}

fn context<'a>(university: &'a ResolvedUniversity, session: &'a mut SessionData) -> Context<'a> {
    Context {
        university,
        session,
        now: Utc.with_ymd_and_hms(2026, 10, 28, 8, 0, 0).unwrap(),
        net: NetSettings { https_only: false },
    }
}

#[tokio::test]
async fn numero_et_formation_ajoutes_au_profil_du_portail() {
    let (state, base) = start(false).await;
    let uni = university(&base);
    let mut session = session();

    let fetched = profile(&mut context(&uni, &mut session)).await.unwrap();
    assert_eq!(fetched.provider, "portal");
    let p = fetched.data;
    assert_eq!(p.last_name.as_deref(), Some("Demo"));
    assert_eq!(p.student_number.as_deref(), Some("21000000"));
    assert_eq!(p.program.as_deref(), Some("Licence 1 Arts"));
    // Premier appel sans session, entrée par le CAS, second appel.
    assert_eq!(state.lock().unwrap().notes_calls, 2);
    assert!(session.cookies.iter().any(|c| c.name == "SCOL"));
}

#[tokio::test]
async fn dossier_indisponible_le_profil_du_portail_reste() {
    let (_state, base) = start(true).await;
    let uni = university(&base);
    let p = profile(&mut context(&uni, &mut session()))
        .await
        .unwrap()
        .data;
    assert_eq!(p.last_name.as_deref(), Some("Demo"));
    assert_eq!(p.student_number, None);
    assert_eq!(p.program, None);
}

#[tokio::test]
async fn notes_par_le_cas() {
    let (_state, base) = start(false).await;
    let uni = university(&base);
    let fetched = grades(&mut context(&uni, &mut session())).await.unwrap();
    assert_eq!(fetched.provider, "etudiants-api");
    let semester = &fetched.data.semesters[0];
    assert_eq!(semester.name, "Semestre 1");
    assert_eq!(
        semester.modules[0].average.as_ref().map(|g| g.value),
        Some(13.25)
    );
}

#[tokio::test]
async fn notes_application_indisponible() {
    let (_state, base) = start(true).await;
    let uni = university(&base);
    let problem = grades(&mut context(&uni, &mut session()))
        .await
        .unwrap_err();
    assert_eq!(problem.code, tonapi_core::ErrorCode::ServiceUnavailable);
}
