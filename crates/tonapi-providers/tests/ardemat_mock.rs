//! Fournisseur `ardemat` contre un faux CAS (détour `pre-otp` comme à Montpellier) et une fausse
//! API : ticket demandé sans être consommé, à usage unique, puis liste et PDF. Valeurs inventées.

mod common;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Response, StatusCode, Uri, header};
use chrono::{TimeZone, Utc};
use tonapi_config::ResolvedUniversity;
use tonapi_core::{DocumentKind, ErrorCode};
use tonapi_providers::{Context, document, documents};
use tonapi_session::SessionData;

use common::{query, redirect};

#[derive(Default)]
struct Mock {
    base: String,
    issued: u32,
    /// Tickets émis et pas encore validés.
    valid: BTreeSet<String>,
    /// Requêtes reçues par l'application elle-même (ne doit jamais arriver).
    app_hits: u32,
    blocked: bool,
}

type Shared = Arc<Mutex<Mock>>;

fn reply(status: StatusCode, content_type: &str, body: impl Into<Body>) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(body.into())
        .unwrap()
}

fn has_tgc(headers: &HeaderMap) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .is_some_and(|c| c.split("; ").any(|p| p == "TGC=TGT-demo"))
}

fn encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn student(blocked: bool) -> String {
    serde_json::json!({
        "codeEtu": "00000000", "codeInd": "1", "nom": "DEMO", "email": "demo@exemple.test",
        "estInterdit": blocked,
        "attestations": [{
            "academie": "X", "annee": "2025", "attestationToken": "tok-att", "code": "L1ARTS",
            "codeUtilisateur": "u", "commune": "Ville", "composante": "C", "cy3": "", "etablissement": "E",
            "etudiantToken": "tok-etu", "libUtilisateur": "", "libelle": "Licence 1 Arts",
            "report": "", "session": "1", "type": "AR", "version": 1
        }]
    })
    .to_string()
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
    let service = format!("{base}/ardemat/");
    match uri.path() {
        "/cas/login" => {
            if !has_tgc(&headers) {
                return reply(
                    StatusCode::OK,
                    "text/html",
                    r#"<form id="fm1" method="post"><input name="username"><input type="password" name="password"><input type="hidden" name="execution" value="e1"></form>"#,
                );
            }
            let target = query(&uri, "service").unwrap_or_default();
            let inner = format!(
                "{base}/cas/login-abc?service={}&auth_checked",
                encode(&target)
            );
            redirect(&format!("{base}/pre-otp/login?service={}", encode(&inner)))
        }
        "/pre-otp/login" => redirect(&format!(
            "{}&ticket=PRE-1",
            query(&uri, "service").unwrap_or_default()
        )),
        "/cas/login-abc" => {
            s.issued += 1;
            let ticket = format!("ST-{}", s.issued);
            s.valid.insert(ticket.clone());
            let target = query(&uri, "service").unwrap_or_default();
            assert_eq!(
                target, service,
                "ticket demandé pour le service de l'application"
            );
            redirect(&format!("{target}?ticket={ticket}"))
        }
        "/ardemat/" => {
            s.app_hits += 1;
            reply(StatusCode::OK, "text/html", "<app-root></app-root>")
        }
        path if path.starts_with("/ardemat-api/api/etudiants/") => {
            // L'API valide le ticket elle-même, sans cookie, une seule fois.
            assert!(headers.get(header::COOKIE).is_none() || !has_tgc(&headers));
            let ticket = path.trim_start_matches("/ardemat-api/api/etudiants/");
            if !s.valid.remove(ticket) {
                return reply(StatusCode::UNAUTHORIZED, "application/json", "{}");
            }
            reply(StatusCode::OK, "application/json", student(s.blocked))
        }
        "/ardemat-api/api/attestations/download" if method == Method::POST => {
            let dto: serde_json::Value = serde_json::from_str(&body).unwrap();
            if dto["attestationToken"] != "tok-att" || dto["etudiantToken"] != "tok-etu" {
                return reply(StatusCode::FORBIDDEN, "application/json", "{}");
            }
            reply(
                StatusCode::OK,
                "application/octet-stream",
                "%PDF-1.4 faux\n%%EOF",
            )
        }
        _ => reply(StatusCode::NOT_FOUND, "text/plain", ""),
    }
}

async fn start(blocked: bool) -> (Shared, String) {
    let (listener, base) = common::listen().await;
    let state: Shared = Arc::new(Mutex::new(Mock {
        base: base.clone(),
        blocked,
        ..Mock::default()
    }));
    let app = Router::new().fallback(handle).with_state(state.clone());
    common::spawn(listener, app);
    (state, base)
}

fn university(base: &str) -> ResolvedUniversity {
    common::university(serde_json::json!({
        "id": "test-ardemat",
        "name": "Université de test",
        "sso": { "kind": "cas", "base-url": format!("{base}/cas/") },
        "services": { "documents": {
            "provider": "ardemat",
            "base-url": format!("{base}/ardemat-api/"),
            "service-url": format!("{base}/ardemat/")
        } },
    }))
}

fn session(tgc: bool) -> SessionData {
    let cookies = if tgc {
        vec![common::stored_cookie("TGC", "TGT-demo", "/cas")]
    } else {
        Vec::new()
    };
    common::session("test-ardemat", "e00000000000", cookies)
}

fn context<'a>(university: &'a ResolvedUniversity, session: &'a mut SessionData) -> Context<'a> {
    common::context(
        university,
        session,
        Utc.with_ymd_and_hms(2026, 10, 28, 8, 0, 0).unwrap(),
    )
}

#[tokio::test]
async fn ticket_non_consomme_puis_liste_et_pdf() {
    let (state, base) = start(false).await;
    let uni = university(&base);
    let mut session = session(true);

    let fetched = documents(&mut context(&uni, &mut session)).await.unwrap();
    assert_eq!(fetched.provider, "ardemat");
    let doc = &fetched.data[0];
    assert_eq!(doc.id, "2025-L1ARTS-1-AR-1");
    assert_eq!(doc.title, "Licence 1 Arts");
    assert_eq!(doc.kind, DocumentKind::Achievement);
    assert!(doc.downloadable);

    let file = document(&mut context(&uni, &mut session), &doc.id)
        .await
        .unwrap()
        .data;
    assert_eq!(file.content_type, "application/pdf");
    assert_eq!(file.file_name, "attestation-2025-L1ARTS-1-AR-1.pdf");
    assert!(file.bytes.starts_with(b"%PDF"));

    let s = state.lock().unwrap();
    assert_eq!(s.issued, 2, "un ticket neuf par appel");
    assert_eq!(
        s.app_hits, 0,
        "le ticket n'est jamais remis à l'application"
    );
}

#[tokio::test]
async fn identifiant_inconnu_ou_malforme() {
    let (_state, base) = start(false).await;
    let uni = university(&base);
    let mut session = session(true);
    for id in ["2024-X-1-AR-1", "../api", ""] {
        let problem = document(&mut context(&uni, &mut session), id)
            .await
            .unwrap_err();
        assert_eq!(problem.code, ErrorCode::NotFound, "{id}");
    }
}

#[tokio::test]
async fn compte_bloque_et_session_expiree() {
    let (_state, base) = start(true).await;
    let uni = university(&base);
    let mut session_ok = session(true);
    let list = documents(&mut context(&uni, &mut session_ok))
        .await
        .unwrap()
        .data;
    assert!(!list[0].downloadable);
    let problem = document(&mut context(&uni, &mut session_ok), &list[0].id)
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::AccountActionRequired);

    let problem = documents(&mut context(&uni, &mut session(false)))
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::SessionExpired);
}
