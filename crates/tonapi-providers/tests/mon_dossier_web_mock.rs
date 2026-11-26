//! Fournisseur `mon-dossier-web` contre un faux CAS et une fausse application Vaadin : page
//! d'accueil derrière le CAS, ouverture de l'interface, clic sur « Inscriptions », puis PDF.
//! Valeurs inventées.

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Response, StatusCode, Uri, header};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use tonapi_config::ResolvedUniversity;
use tonapi_core::{DocumentKind, ErrorCode};
use tonapi_providers::{Context, document, documents};
use tonapi_session::SessionData;

use common::{has_cookie, query, redirect};

const PAGE: &str = include_str!("fixtures/mon_dossier_web/page.html");
const OPENING: &str = include_str!("fixtures/mon_dossier_web/ouverture.json");
const MENU: &str = include_str!("fixtures/mon_dossier_web/inscriptions.json");
const NEWEST: &str =
    "/mdw/APP/connector/3/646/dl/Certificat_de_Scolarit%C3%A9_X0001_2026-2027_CAMILLE_DEMO.pdf";

#[derive(Default)]
struct Mock {
    base: String,
    /// Sessions ouvertes par l'application (une par ticket validé).
    sessions: u32,
    /// Interfaces ouvertes, clics reçus et fichiers servis.
    openings: Vec<String>,
    clicks: Vec<Value>,
    downloads: Vec<String>,
    /// L'onglet « Inscriptions » ne propose aucun fichier.
    no_certificate: bool,
    /// La page d'accueil n'est pas l'application.
    maintenance: bool,
    /// Le fichier servi n'est pas un PDF.
    broken_file: bool,
}

type Shared = Arc<Mutex<Mock>>;

fn reply(status: StatusCode, content_type: &str, body: impl Into<Body>) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(body.into())
        .unwrap()
}

fn encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn signed_in(headers: &HeaderMap, state: &Mock) -> bool {
    (1..=state.sessions).any(|n| has_cookie(headers, &format!("JSESSIONID=mdw-{n}")))
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
    let service = format!("{base}/mdw/login/cas");
    match uri.path() {
        "/cas/login" => {
            if has_cookie(&headers, "TGC=TGT-demo") {
                let target = query(&uri, "service").unwrap_or_default();
                redirect(&format!("{target}?ticket=ST-mdw"))
            } else {
                reply(
                    StatusCode::OK,
                    "text/html",
                    r#"<form id="fm1" method="post"><input name="username"><input type="password" name="password"><input type="hidden" name="execution" value="e1"></form>"#,
                )
            }
        }
        "/mdw/login/cas" => {
            if query(&uri, "ticket").as_deref() != Some("ST-mdw") {
                return reply(StatusCode::FORBIDDEN, "text/plain", "");
            }
            s.sessions += 1;
            Response::builder()
                .status(StatusCode::FOUND)
                .header(header::LOCATION, format!("{base}/mdw/"))
                .header(
                    header::SET_COOKIE,
                    format!("JSESSIONID=mdw-{}; Path=/mdw; HttpOnly", s.sessions),
                )
                .body(Body::empty())
                .unwrap()
        }
        "/mdw/" if !signed_in(&headers, &s) => {
            redirect(&format!("{base}/cas/login?service={}", encode(&service)))
        }
        "/mdw/" if method == Method::GET => {
            if s.maintenance {
                return reply(StatusCode::OK, "text/html", "<html>Maintenance</html>");
            }
            reply(StatusCode::OK, "text/html; charset=utf-8", PAGE)
        }
        "/mdw/" if method == Method::POST => {
            s.openings
                .push(format!("{}|{body}", uri.query().unwrap_or_default()));
            reply(StatusCode::OK, "application/json; charset=utf-8", OPENING)
        }
        "/mdw/UIDL/" if method == Method::POST && signed_in(&headers, &s) => {
            assert_eq!(query(&uri, "v-uiId").as_deref(), Some("3"));
            let click: Value = serde_json::from_str(&body).unwrap();
            if click["csrfToken"] != "jeton-csrf-invente" {
                return reply(StatusCode::FORBIDDEN, "application/json", "{}");
            }
            s.clicks.push(click);
            let menu = if s.no_certificate {
                "for(;;);[{\"syncId\":4,\"state\":{}}]"
            } else {
                MENU
            };
            reply(StatusCode::OK, "application/json; charset=utf-8", menu)
        }
        path if path.starts_with("/mdw/APP/connector/") && signed_in(&headers, &s) => {
            s.downloads.push(path.to_owned());
            if s.broken_file {
                return reply(StatusCode::OK, "text/html", "<html>Erreur</html>");
            }
            reply(
                StatusCode::OK,
                "application/pdf",
                "%PDF-1.4 faux certificat\n%%EOF",
            )
        }
        _ => reply(StatusCode::NOT_FOUND, "text/plain", ""),
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
    common::university(json!({
        "id": "test-mdw",
        "name": "Université de test",
        "sso": { "kind": "cas", "base-url": format!("{base}/cas/") },
        "services": { "documents": {
            "provider": "mon-dossier-web",
            // Sans barre finale : la racine est normalisée.
            "base-url": format!("{base}/mdw")
        } },
    }))
}

fn session(tgc: bool) -> SessionData {
    let cookies = if tgc {
        vec![common::stored_cookie("TGC", "TGT-demo", "/cas")]
    } else {
        Vec::new()
    };
    common::session("test-mdw", "e00000000000", cookies)
}

fn context<'a>(university: &'a ResolvedUniversity, session: &'a mut SessionData) -> Context<'a> {
    common::context(
        university,
        session,
        Utc.with_ymd_and_hms(2026, 11, 26, 8, 0, 0).unwrap(),
    )
}

#[tokio::test]
async fn certificats_listes_puis_pdf_du_plus_recent() {
    let (state, base) = start().await;
    let uni = university(&base);
    let mut session = session(true);

    let fetched = documents(&mut context(&uni, &mut session)).await.unwrap();
    assert_eq!(fetched.provider, "mon-dossier-web");
    let ids: Vec<&str> = fetched.data.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, ["certificat-2026", "certificat-2025"]);
    let newest = &fetched.data[0];
    assert_eq!(newest.title, "Certificat de scolarité 2026-2027");
    assert_eq!(newest.kind, DocumentKind::Certificate);
    assert_eq!(newest.year.as_deref(), Some("2026"));
    assert!(newest.downloadable);
    {
        let s = state.lock().unwrap();
        assert_eq!(s.sessions, 1);
        // L'interface est ouverte à l'heure du contexte, avec l'identifiant lu dans la page.
        let [opening] = s.openings.as_slice() else {
            panic!("une seule interface ouverte");
        };
        assert!(opening.starts_with("v-1795680000000|"), "{opening}");
        assert!(opening.contains("v-appId=ROOT-1234567") && opening.contains("theme=valo-ul"));
        // Le clic vise le bouton « Inscriptions » de cette interface.
        assert_eq!(s.clicks[0]["rpc"][0][0], "547");
        assert!(s.downloads.is_empty());
    }
    // La session de l'application reste dans celle de l'étudiant.
    assert!(
        session
            .cookies
            .iter()
            .any(|c| c.name == "JSESSIONID" && c.value == "mdw-1")
    );

    let file = document(&mut context(&uni, &mut session), "certificat-2026")
        .await
        .unwrap()
        .data;
    assert_eq!(file.content_type, "application/pdf");
    assert_eq!(
        file.file_name,
        "Certificat_de_Scolarité_X0001_2026-2027_CAMILLE_DEMO.pdf"
    );
    assert!(file.bytes.starts_with(b"%PDF"));
    let s = state.lock().unwrap();
    assert_eq!(s.sessions, 1, "le CAS n'est pas redemandé");
    assert_eq!(
        s.openings.len(),
        2,
        "une interface neuve pour le téléchargement"
    );
    assert_eq!(s.downloads, [NEWEST]);
}

#[tokio::test]
async fn aucun_certificat_et_identifiant_inconnu() {
    let (state, base) = start().await;
    let uni = university(&base);
    let mut session = session(true);

    for id in ["certificat-2024", "certificat-2026.2", "../APP", "", "a/b"] {
        let problem = document(&mut context(&uni, &mut session), id)
            .await
            .unwrap_err();
        assert_eq!(problem.code, ErrorCode::NotFound, "{id}");
    }
    assert!(state.lock().unwrap().downloads.is_empty());

    state.lock().unwrap().no_certificate = true;
    let none = documents(&mut context(&uni, &mut session)).await.unwrap();
    assert!(none.data.is_empty());
    let problem = document(&mut context(&uni, &mut session), "certificat-2026")
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn session_expiree_page_inattendue_et_fichier_qui_n_est_pas_un_pdf() {
    let (state, base) = start().await;
    let uni = university(&base);

    // Sans TGC, le CAS redemande le mot de passe : rien n'est posté.
    let problem = documents(&mut context(&uni, &mut session(false)))
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::SessionExpired);

    let mut session = session(true);
    state.lock().unwrap().broken_file = true;
    let problem = document(&mut context(&uni, &mut session), "certificat-2026")
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::UpstreamChanged);
    // Aucune donnée de l'étudiant dans le message.
    assert!(!problem.detail.clone().unwrap_or_default().contains("DEMO"));

    // Une page qui n'est pas l'application : aucune interface n'est ouverte.
    state.lock().unwrap().maintenance = true;
    let opened = state.lock().unwrap().openings.len();
    let problem = documents(&mut context(&uni, &mut session))
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::UpstreamChanged);
    assert_eq!(state.lock().unwrap().openings.len(), opened);
}
