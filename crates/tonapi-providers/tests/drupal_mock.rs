//! Fournisseur `portal` (Drupal) contre un faux portail local et un faux CAS : la racine et les
//! pages renvoient au CAS sans session, `casservice` reçoit le ticket, `user` mène à
//! `user/<numéro>`, `tous-outils` porte le catalogue. Aucune requête ne quitte cet ordinateur.

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Response, StatusCode, Uri, header};
use chrono::Utc;
use serde_json::json;
use tonapi_config::ResolvedUniversity;
use tonapi_core::ErrorCode;
use tonapi_providers::{Context, LoginInput, LoginOutcome, NetSettings, apps, login, profile};
use tonapi_session::{SessionData, StoredCookie};

use common::{CAS_PASSWORD, CAS_TICKET, CAS_USER, Cas, has_cookie, query};

const USER: &str = include_str!("fixtures/drupal/utilisateur.html");
const CATALOGUE: &str = include_str!("fixtures/drupal/catalogue.html");
const ANONYMOUS: &str =
    r#"<html><body class="user-anonymous path-user">Accès refusé</body></html>"#;

// Faux portail

#[derive(Default)]
struct Portal {
    /// Adresse du faux CAS (`http://localhost:<port>/cas/`).
    cas: String,
    /// Sa propre adresse (`http://127.0.0.1:<port>`).
    base: String,
    /// Sessions ouvertes jusqu'ici : la dernière est la seule reconnue.
    sessions: u32,
    /// Prochaines pages servies sans session (200, classe `user-anonymous`), même à un étudiant
    /// connecté.
    anonymous_pages: u32,
    /// Le portail répond 503 à tout.
    down: bool,
    /// Chemins demandés, dans l'ordre.
    paths: Vec<String>,
}

type Shared = Arc<Mutex<Portal>>;

fn page(status: StatusCode, body: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

fn encoded(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

async fn portal_handler(
    State(state): State<Shared>,
    uri: Uri,
    headers: HeaderMap,
) -> Response<Body> {
    let mut portal = state.lock().unwrap();
    let path = uri.path().to_owned();
    portal.paths.push(path.clone());
    if portal.down {
        return page(StatusCode::SERVICE_UNAVAILABLE, "<html>Maintenance</html>");
    }
    // Retour du CAS : le ticket ouvre une session, puis la page demandée.
    if path == "/casservice" {
        let destination = query(&uri, "destination").unwrap_or_else(|| "/".to_owned());
        if query(&uri, "ticket").as_deref() != Some(CAS_TICKET) {
            return page(StatusCode::FORBIDDEN, "<html>Ticket refusé</html>");
        }
        portal.sessions += 1;
        let cookie = format!("SSESSdemo=s{}; Path=/", portal.sessions);
        let mut response = common::redirect(&destination);
        response
            .headers_mut()
            .insert(header::SET_COOKIE, cookie.parse().unwrap());
        return response;
    }
    let signed_in =
        portal.sessions > 0 && has_cookie(&headers, &format!("SSESSdemo=s{}", portal.sessions));
    if !signed_in {
        // Le portail force la connexion : toute page renvoie au CAS.
        let service = format!("{}/casservice?destination={}", portal.base, encoded(&path));
        return common::redirect(&format!(
            "{}login?service={}",
            portal.cas,
            encoded(&service)
        ));
    }
    if portal.anonymous_pages > 0 && path != "/" {
        portal.anonymous_pages -= 1;
        return page(StatusCode::OK, ANONYMOUS);
    }
    match path.as_str() {
        "/" => page(
            StatusCode::OK,
            r#"<html><body class="user-logged-in path-frontpage">Accueil</body></html>"#,
        ),
        "/user" => common::redirect("/user/42"),
        "/user/42" => page(StatusCode::OK, USER),
        "/tous-outils" => page(StatusCode::OK, CATALOGUE),
        _ => page(
            StatusCode::NOT_FOUND,
            r#"<html><body class="user-logged-in">404</body></html>"#,
        ),
    }
}

/// Faux portail et faux CAS, et l'université qui les désigne.
struct Campus {
    portal: Shared,
    cas: Cas,
    university: ResolvedUniversity,
}

async fn campus() -> Campus {
    campus_with(json!({ "path": "tous-outils", "view": "outils", "item-path": "/outil/" })).await
}

async fn campus_with(catalogue: serde_json::Value) -> Campus {
    let (portal_listener, portal_base) = common::listen().await;
    let (cas_listener, cas_listener_base) = common::listen().await;
    let cas_base = common::cas_base(&cas_listener_base);
    let cas = Cas::default();
    common::spawn_cas(cas_listener, &cas);
    let portal = Arc::new(Mutex::new(Portal {
        cas: cas_base.clone(),
        base: portal_base.clone(),
        ..Portal::default()
    }));
    common::spawn(
        portal_listener,
        Router::new()
            .fallback(portal_handler)
            .with_state(portal.clone()),
    );
    let mut portal_config = json!({
        "kind": "drupal",
        "base-url": format!("{portal_base}/"),
        "name-field": "field-nomprenom",
    });
    if !catalogue.is_null() {
        portal_config["catalogue"] = catalogue;
    }
    let university = common::university(json!({
        "id": "test",
        "name": "Université de test",
        "sso": { "kind": "cas", "base-url": cas_base },
        "portal": portal_config,
        "services": { "apps": { "provider": "portal" }, "profile": { "provider": "portal" } },
    }));
    Campus {
        portal,
        cas,
        university,
    }
}

impl Campus {
    /// Ouvre une session du portail comme le ferait une connexion, et rend son cookie.
    fn portal_cookie(&self) -> StoredCookie {
        let mut portal = self.portal.lock().unwrap();
        portal.sessions += 1;
        common::stored_cookie("SSESSdemo", &format!("s{}", portal.sessions), "/")
    }

    fn paths(&self) -> Vec<String> {
        self.portal.lock().unwrap().paths.clone()
    }
}

fn session(cookies: Vec<StoredCookie>) -> SessionData {
    common::session("test", "e00000000", cookies)
}

fn context<'a>(university: &'a ResolvedUniversity, session: &'a mut SessionData) -> Context<'a> {
    common::context(university, session, Utc::now())
}

fn portal_session(session: &SessionData) -> Option<&str> {
    session
        .cookies
        .iter()
        .find(|c| c.name == "SSESSdemo")
        .map(|c| c.value.as_str())
}

// Tests

#[tokio::test]
async fn connexion_par_la_racine_du_portail_puis_profil() {
    let campus = campus().await;
    let outcome = login(
        &campus.university,
        LoginInput {
            username: CAS_USER,
            password: CAS_PASSWORD,
            establishment: None,
            previous_cookies: Vec::new(),
            now: Utc::now(),
            ttl_secs: 3600,
            net: NetSettings {
                https_only: false,
                user_agent: None,
            },
        },
    )
    .await
    .unwrap();
    let LoginOutcome::SignedIn(mut session) = outcome else {
        panic!("connexion sans second facteur attendue");
    };
    // Le mot de passe est posté une fois, pour le service du portail, et la session garde le
    // TGC et le cookie du portail.
    {
        let log = campus.cas.log.lock().unwrap();
        assert_eq!(log.password_posts, 1);
        assert!(
            log.services
                .first()
                .unwrap()
                .contains("/casservice?destination=")
        );
    }
    assert!(session.cookies.iter().any(|c| c.name == "TGC"));
    assert_eq!(portal_session(&session), Some("s1"));
    assert_eq!(campus.paths(), ["/", "/casservice", "/"]);

    let fetched = profile(&mut context(&campus.university, &mut session))
        .await
        .unwrap();
    assert_eq!(fetched.provider, "drupal");
    // Le portail ne nomme pas l'étudiant : l'identifiant est celui de la connexion.
    assert_eq!(fetched.data.username, CAS_USER);
    assert_eq!(
        fetched.data.display_name.as_deref(),
        Some("Camille Le Demo")
    );
    assert_eq!(fetched.data.first_name.as_deref(), Some("Camille"));
    assert_eq!(fetched.data.last_name.as_deref(), Some("Le Demo"));
    assert_eq!(fetched.data.email, None);
}

#[tokio::test]
async fn profil_et_applications_avec_la_seule_session_du_portail() {
    let campus = campus().await;
    let mut session = session(vec![campus.portal_cookie()]);

    let profile = profile(&mut context(&campus.university, &mut session))
        .await
        .unwrap()
        .data;
    assert_eq!(profile.username, "e00000000");
    assert_eq!(profile.display_name.as_deref(), Some("Camille Le Demo"));

    let fetched = apps(&mut context(&campus.university, &mut session))
        .await
        .unwrap();
    assert_eq!(fetched.provider, "drupal");
    let read: Vec<_> = fetched
        .data
        .iter()
        .map(|app| (app.id.as_str(), app.category.as_deref()))
        .collect();
    assert_eq!(
        read,
        [
            ("planning", Some("Bureau")),
            ("imprimer", Some("Bureau")),
            ("disques", Some("Ressources"))
        ]
    );
    assert_eq!(
        fetched.data.get(1).unwrap().url,
        format!(
            "{}/numerique/impression#accordion-1",
            campus.portal.lock().unwrap().base
        )
    );
    // Une requête par page (et la redirection de `user`), sans passer par le CAS.
    assert_eq!(campus.paths(), ["/user", "/user/42", "/tous-outils"]);
    assert!(campus.cas.log.lock().unwrap().services.is_empty());
    assert_eq!(portal_session(&session), Some("s1"));
}

#[tokio::test]
async fn session_du_portail_perdue_rouverte_avec_le_tgc() {
    let campus = campus().await;
    // Un cookie que le portail ne reconnaît plus, et le TGC de la session.
    let stale = common::stored_cookie("SSESSdemo", "perdu", "/");
    let mut session = session(vec![stale, common::tgc()]);

    let fetched = apps(&mut context(&campus.university, &mut session))
        .await
        .unwrap();
    assert_eq!(fetched.data.len(), 3);
    // Le portail a renvoyé au CAS, qui a rendu un ticket pour la page demandée : la nouvelle
    // session repart dans celle de l'étudiant.
    assert_eq!(
        campus.paths(),
        ["/tous-outils", "/casservice", "/tous-outils"]
    );
    assert_eq!(portal_session(&session), Some("s1"));
    let log = campus.cas.log.lock().unwrap();
    assert_eq!(
        (log.services.len(), log.forms, log.password_posts),
        (1, 0, 0)
    );
}

#[tokio::test]
async fn sans_tgc_la_session_est_expiree_sans_nouvel_essai() {
    let campus = campus().await;
    let stale = common::stored_cookie("SSESSdemo", "perdu", "/");
    let mut session = session(vec![stale]);

    let error = profile(&mut context(&campus.university, &mut session))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::SessionExpired);
    // Le CAS a montré son formulaire une fois ; rien n'y a été posté.
    assert_eq!(campus.paths(), ["/user"]);
    let log = campus.cas.log.lock().unwrap();
    assert_eq!((log.forms, log.password_posts), (1, 0));
}

#[tokio::test]
async fn page_servie_sans_session_un_seul_nouvel_essai_par_la_racine() {
    let campus = campus().await;
    let mut session = session(vec![campus.portal_cookie(), common::tgc()]);
    campus.portal.lock().unwrap().anonymous_pages = 1;

    let profile_read = profile(&mut context(&campus.university, &mut session))
        .await
        .unwrap()
        .data;
    assert_eq!(
        profile_read.display_name.as_deref(),
        Some("Camille Le Demo")
    );
    // La page servie sans session, puis l'entrée par la racine sans les anciens cookies (le CAS
    // rend un ticket), puis la page.
    assert_eq!(
        campus.paths(),
        ["/user", "/", "/casservice", "/", "/user", "/user/42"]
    );
    assert_eq!(portal_session(&session), Some("s2"));

    // Le portail sert encore la page sans session après l'entrée : `session_expired`, sans boucle.
    campus.portal.lock().unwrap().paths.clear();
    campus.portal.lock().unwrap().anonymous_pages = 10;
    let error = profile(&mut context(&campus.university, &mut session))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::SessionExpired);
    let user_pages = campus.paths().iter().filter(|p| *p == "/user").count();
    assert_eq!(user_pages, 2);
}

#[tokio::test]
async fn portail_en_panne() {
    let campus = campus().await;
    let mut session = session(vec![campus.portal_cookie(), common::tgc()]);
    campus.portal.lock().unwrap().down = true;

    let error = apps(&mut context(&campus.university, &mut session))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ServiceUnavailable);
    assert_eq!(campus.paths(), ["/tous-outils"]);
}

#[tokio::test]
async fn sans_catalogue_declare_pas_d_applications() {
    let campus = campus_with(serde_json::Value::Null).await;
    let mut session = session(vec![campus.portal_cookie()]);

    let error = apps(&mut context(&campus.university, &mut session))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ServiceAbsent);
    assert!(campus.paths().is_empty());
    // Le profil, lui, se lit.
    assert!(
        profile(&mut context(&campus.university, &mut session))
            .await
            .is_ok()
    );
}
