//! Profil du portail complété par le dossier `etudiants-api`, contre un faux portail uPortal 4,
//! un faux CAS et une fausse application de scolarité. Valeurs inventées.

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Response, StatusCode, Uri, header};
use chrono::{TimeZone, Utc};
use tonapi_config::ResolvedUniversity;
use tonapi_core::{DocumentKind, ErrorCode};
use tonapi_providers::{Context, document, documents, exams, grades, profile};
use tonapi_session::SessionData;

use common::{has_cookie, query, redirect};

const SESSION: &str = include_str!("fixtures/uportal/session_v4.json");
const PEOPLE: &str = include_str!("fixtures/uportal/people_v4.json");
const NOTES: &str = r#"{"numero":"21000000","nom":"DEMO","prenom":"Camille","etapes":[{"libelle":"Licence 1 Arts","codEtape":"XX1","semestres":{"S1":{"intitule":"Semestre 1","classifiedElementPedagogiques":[{"codElement":"UE1","libElement":"Fondamentaux","niveauElement":1,"noteEpreuve1":"12,5","note":"13,25","resultat":"ADM"}]}}}],"epreuves":{}}"#;

const CALENDAR: &str = r#"{"numero":"21000000","etapes":[],"epreuves":{"P1":[{"libPeriode":"Session 1","codEpreuve":"EP1","libEpreuve":"Lettres","dateEpreuve":"14/12/2026","heureDebutEpreuve":"09:00","dureeEpreuve":"2h00","salleEpreuve":"A1","batEpreuve":"A","infoEpreuve":"","elementPedagogiques":[{"codElement":"EC1","libElement":"Lettres"}]}]}}"#;
const ATTEST_SEM: &str = r#"{"attestations":{"2025":[{"annee":"2025","type":"S1/L1","libelle":"Semestre 1","session":"1"}]},"attestationReussites":{}}"#;
const ATTEST_ANU: &str = r#"{"attestations":{},"attestationReussites":{}}"#;
const ATTEST_REU: &str = r#"{"attestations":{},"attestationReussites":{"7":"Licence Arts"}}"#;
const PDF: &str = "%PDF-1.4 faux document\n%%EOF";

#[derive(Default)]
struct Mock {
    base: String,
    /// L'application de scolarité répond 503.
    down: bool,
    notes_calls: u32,
}

type Shared = Arc<Mutex<Mock>>;

fn reply(status: StatusCode, cookie: Option<&str>, body: &str) -> Response<Body> {
    let mut r = Response::builder().status(status);
    if let Some(cookie) = cookie {
        r = r.header(header::SET_COOKIE, cookie);
    }
    r.body(Body::from(body.to_owned())).unwrap()
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
        path if path.starts_with("/scolarite/api/etudiants/") => {
            if !has_cookie(&headers, "SCOL=ok") {
                return reply(StatusCode::UNAUTHORIZED, None, "");
            }
            let rest = path.trim_start_matches("/scolarite/api/etudiants/");
            match rest {
                "calendrier" => reply(StatusCode::OK, None, CALENDAR),
                "attestations" => reply(StatusCode::OK, None, ATTEST_SEM),
                "attestations/annuelle" => reply(StatusCode::OK, None, ATTEST_ANU),
                "attestations/reussites" => reply(StatusCode::OK, None, ATTEST_REU),
                "attestations/2025_S1L1_1" | "attestations/reussites/7" => Response::builder()
                    .header(header::CONTENT_TYPE, "application/octet-stream")
                    .body(Body::from(PDF))
                    .unwrap(),
                _ => reply(StatusCode::NOT_FOUND, None, ""),
            }
        }
        _ => reply(StatusCode::NOT_FOUND, None, ""),
    }
}

async fn start(down: bool) -> (Shared, String) {
    let (listener, base) = common::listen().await;
    let state: Shared = Arc::new(Mutex::new(Mock {
        base: base.clone(),
        down,
        ..Mock::default()
    }));
    let app = Router::new().fallback(handle).with_state(state.clone());
    common::spawn(listener, app);
    (state, base)
}

fn university(base: &str) -> ResolvedUniversity {
    common::university(serde_json::json!({
        "id": "test-scol",
        "name": "Université de test",
        "sso": { "kind": "cas", "base-url": format!("{base}/cas/") },
        "portal": { "kind": "uportal4-prolongation", "base-url": format!("{base}/uPortal/") },
        "services": {
            "profile": {
                "provider": "portal",
                "record": { "kind": "etudiants-api", "base-url": format!("{base}/scolarite/") }
            },
            "grades": { "provider": "etudiants-api", "base-url": format!("{base}/scolarite/") },
            "exams": { "provider": "etudiants-api", "base-url": format!("{base}/scolarite/") },
            "documents": {
                "provider": "etudiants-api",
                "base-url": format!("{base}/scolarite/"),
                "certificate-url": format!("{base}/certificat")
            }
        },
    }))
}

fn session() -> SessionData {
    common::session(
        "test-scol",
        "e00000000000",
        vec![common::stored_cookie("TGC", "TGT-demo", "/")],
    )
}

fn context<'a>(university: &'a ResolvedUniversity, session: &'a mut SessionData) -> Context<'a> {
    common::context(
        university,
        session,
        Utc.with_ymd_and_hms(2026, 10, 28, 8, 0, 0).unwrap(),
    )
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

#[tokio::test]
async fn examens_par_le_cas() {
    let (_state, base) = start(false).await;
    let uni = university(&base);
    let fetched = exams(&mut context(&uni, &mut session())).await.unwrap();
    assert_eq!(fetched.provider, "etudiants-api");
    let exam = &fetched.data[0];
    assert_eq!(exam.id, "P1-EP1");
    assert_eq!(
        exam.start.map(|d| d.to_rfc3339()).as_deref(),
        Some("2026-12-14T09:00:00+01:00")
    );
    assert_eq!(exam.duration_minutes, Some(120));
}

#[tokio::test]
async fn attestations_listees_puis_telechargees() {
    let (_state, base) = start(false).await;
    let uni = university(&base);
    let mut session = session();
    let list = documents(&mut context(&uni, &mut session))
        .await
        .unwrap()
        .data;
    let ids: Vec<&str> = list.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, ["certificat", "sem.2025_S1L1_1", "reu.7"]);
    assert_eq!(list[0].kind, DocumentKind::Certificate);
    assert!(!list[0].downloadable);
    assert_eq!(
        list[0].url.as_deref(),
        Some(format!("{base}/certificat").as_str())
    );

    let file = document(&mut context(&uni, &mut session), "sem.2025_S1L1_1")
        .await
        .unwrap()
        .data;
    assert_eq!(file.content_type, "application/pdf");
    assert_eq!(file.file_name, "sem-2025_S1L1_1.pdf");
    assert!(file.bytes.starts_with(b"%PDF"));

    for bad in ["sem.../notes", "xyz.1", "sem.", "reu.8"] {
        let problem = document(&mut context(&uni, &mut session), bad)
            .await
            .unwrap_err();
        assert_eq!(problem.code, ErrorCode::NotFound, "{bad}");
    }
}
