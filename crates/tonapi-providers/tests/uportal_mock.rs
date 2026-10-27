//! Fournisseur `portal` (uPortal) contre de faux portails locaux : un faux uPortal 4 (préfixe
//! `/uPortal/`, `layout.jsp`, `session.json`, `people/<id>.json`), un faux uPortal 5 (racine,
//! `layout.json`, `userinfo`) et un faux CAS. Aucune requête ne quitte `127.0.0.1`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Response, StatusCode, Uri, header};
use chrono::Utc;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tonapi_config::{ResolvedUniversity, UniversityFile};
use tonapi_core::ErrorCode;
use tonapi_providers::{Context, NetSettings, apps, profile};
use tonapi_session::{SessionData, StoredCookie};

const LAYOUT_JSONP: &str = include_str!("fixtures/uportal/layout_prolongation.jsonp");
const LAYOUT_V5: &str = include_str!("fixtures/uportal/layout_v5.json");
const LAYOUT_V5_GUEST: &str = include_str!("fixtures/uportal/layout_v5_invite.json");
const SESSION_V4: &str = include_str!("fixtures/uportal/session_v4.json");
const SESSION_V4_GUEST: &str = include_str!("fixtures/uportal/session_v4_invite.json");
const PEOPLE_V4: &str = include_str!("fixtures/uportal/people_v4.json");
const USERINFO: &str = include_str!("fixtures/uportal/userinfo_v5.jwt");

// ---------------------------------------------------------------------------------------------
// Faux portail
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct Hits {
    /// Passages sur `Login`.
    login: u32,
    /// Requêtes de données (layout, session, people, userinfo).
    data: u32,
    /// Chemins des requêtes de données, dans l'ordre.
    paths: Vec<String>,
}

#[derive(Clone)]
struct Portal {
    /// `/uPortal/` ou `/`.
    prefix: &'static str,
    /// `Login` ouvre une session sans ticket (portail sans CAS devant lui).
    auto_login: bool,
    /// `Login` ne donne jamais qu'une session invitée.
    never_authenticated: bool,
    /// Réponse forcée (statut, corps) pour les routes de données.
    data_override: Option<(StatusCode, &'static str)>,
    /// Statut forcé pour `people/<id>.json`.
    people_status: Option<StatusCode>,
    /// `api/v5-1/userinfo` n'existe pas (404), comme sur certains uPortal.
    no_userinfo: bool,
    hits: Arc<Mutex<Hits>>,
}

impl Portal {
    fn new(prefix: &'static str) -> Self {
        Portal {
            prefix,
            auto_login: true,
            never_authenticated: false,
            data_override: None,
            people_status: None,
            no_userinfo: false,
            hits: Arc::default(),
        }
    }
}

fn has_cookie(headers: &HeaderMap, pair: &str) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .is_some_and(|c| c.split("; ").any(|p| p == pair))
}

fn reply(status: StatusCode, body: &str) -> Response<Body> {
    let mut response = Response::new(Body::from(body.to_owned()));
    *response.status_mut() = status;
    response
}

fn redirect(location: &str, cookie: Option<&str>) -> Response<Body> {
    let mut builder = Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location);
    if let Some(cookie) = cookie {
        builder = builder.header(header::SET_COOKIE, cookie);
    }
    builder.body(Body::empty()).unwrap()
}

async fn portal_handler(State(p): State<Portal>, uri: Uri, headers: HeaderMap) -> Response<Body> {
    let rel = uri.path().strip_prefix(p.prefix).unwrap_or("").to_owned();
    if rel == "Login" {
        p.hits.lock().unwrap().login += 1;
        let ticket = uri.query().is_some_and(|q| q.contains("ticket="));
        let session = if !p.never_authenticated && (p.auto_login || ticket) {
            "JSESSIONID=auth; Path=/"
        } else {
            "JSESSIONID=guest; Path=/"
        };
        return redirect(&format!("{}index.html", p.prefix), Some(session));
    }
    if rel == "index.html" {
        return reply(StatusCode::OK, "accueil");
    }
    // Fragment d'un portlet « iframe » : le vrai lien de l'application.
    if rel == "api/v4-3/portlet/webapp-assistance.html" {
        return reply(
            StatusCode::OK,
            "\n\n<iframe src=\"https://aide.exemple.test/assistance.html\" frameBorder=\"0\" height=\"600\">\n <a href=\"https://aide.exemple.test/assistance.html\" target=\"_blank\">Click here</a>\n</iframe>\n",
        );
    }

    let known = matches!(
        rel.as_str(),
        "layout.jsp"
            | "api/v4-3/dlm/layout.json"
            | "api/session.json"
            | "api/v5-1/userinfo"
            | "api/people/e00000000000.json"
    );
    if !known {
        return reply(StatusCode::NOT_FOUND, "");
    }
    {
        let mut hits = p.hits.lock().unwrap();
        hits.data += 1;
        hits.paths.push(rel.clone());
    }
    if let Some((status, body)) = p.data_override {
        return reply(status, body);
    }
    if p.no_userinfo && rel == "api/v5-1/userinfo" {
        return reply(StatusCode::NOT_FOUND, "<html>404</html>");
    }
    let authenticated = has_cookie(&headers, "JSESSIONID=auth");
    let guest = has_cookie(&headers, "JSESSIONID=guest");
    if !authenticated && !guest {
        // Comme `RequireValidSessionFilter` : sans session, redirection vers `/Login`.
        return redirect(&format!("{}Login?refUrl=%2F", p.prefix), None);
    }
    let body = match (rel.as_str(), authenticated) {
        ("layout.jsp", true) => LAYOUT_JSONP,
        // `layout.jsp` redirige vers le CAS quand on n'est pas connecté.
        ("layout.jsp", false) => return redirect("/cas/login", None),
        ("api/v4-3/dlm/layout.json", true) => LAYOUT_V5,
        ("api/v4-3/dlm/layout.json", false) => LAYOUT_V5_GUEST,
        ("api/session.json", true) => SESSION_V4,
        ("api/session.json", false) => SESSION_V4_GUEST,
        ("api/v5-1/userinfo", true) => USERINFO,
        ("api/v5-1/userinfo", false) => include_str!("fixtures/uportal/userinfo_v5_invite.jwt"),
        ("api/people/e00000000000.json", _) => {
            if let Some(status) = p.people_status {
                return reply(status, "");
            }
            PEOPLE_V4
        }
        _ => "",
    };
    reply(StatusCode::OK, body)
}

/// Faux CAS : avec le cookie `TGC=ok`, renvoie vers le service avec un ticket ; sinon page de
/// connexion (le fournisseur n'a pas de mot de passe à y poster).
#[derive(Clone, Default)]
struct Cas {
    services: Arc<Mutex<Vec<String>>>,
}

async fn cas_handler(State(cas): State<Cas>, uri: Uri, headers: HeaderMap) -> Response<Body> {
    let service = url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
        .find(|(k, _)| k == "service")
        .map(|(_, v)| v.into_owned());
    if let Some(service) = &service {
        cas.services.lock().unwrap().push(service.clone());
    }
    match service {
        Some(service) if has_cookie(&headers, "TGC=ok") => {
            let separator = if service.contains('?') { '&' } else { '?' };
            redirect(&format!("{service}{separator}ticket=ST-1-test"), None)
        }
        _ => reply(
            StatusCode::OK,
            r#"<html><body><form method="post" action="/cas/login">
            <input type="hidden" name="execution" value="e1s1">
            <input type="text" name="username"><input type="password" name="password">
            </form></body></html>"#,
        ),
    }
}

async fn serve(router: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_portal(portal: &Portal) -> String {
    serve(
        Router::new()
            .fallback(portal_handler)
            .with_state(portal.clone()),
    )
    .await
}

async fn spawn_cas(cas: &Cas) -> String {
    serve(Router::new().fallback(cas_handler).with_state(cas.clone())).await
}

// ---------------------------------------------------------------------------------------------
// Contexte
// ---------------------------------------------------------------------------------------------

fn university(portal: &Value, sso: &Value) -> ResolvedUniversity {
    let file: UniversityFile = serde_json::from_value(json!({
        "id": "test",
        "name": "Université de test",
        "sso": sso,
        "portal": portal,
        "services": { "apps": { "provider": "portal" }, "profile": { "provider": "portal" } },
    }))
    .unwrap();
    ResolvedUniversity {
        dir: PathBuf::new(),
        university: file,
        establishments: BTreeMap::new(),
    }
}

fn uportal4(root: &str) -> Value {
    json!({ "kind": "uportal4-prolongation", "base-url": format!("{root}/uPortal/") })
}

fn uportal5(root: &str) -> Value {
    json!({ "kind": "uportal5", "base-url": format!("{root}/") })
}

fn no_cas() -> Value {
    json!({ "kind": "demo" })
}

fn session(cookies: Vec<StoredCookie>) -> SessionData {
    SessionData {
        university: "test".into(),
        username: "e00000000000".into(),
        establishment: None,
        cookies,
        tokens: BTreeMap::new(),
        pending: false,
        issued_at: 0,
        expires_at: i64::MAX,
    }
}

fn tgc() -> StoredCookie {
    StoredCookie {
        domain: "127.0.0.1".into(),
        path: "/".into(),
        name: "TGC".into(),
        value: "ok".into(),
        host_only: true,
        expires: None,
    }
}

fn context<'a>(university: &'a ResolvedUniversity, session: &'a mut SessionData) -> Context<'a> {
    Context {
        university,
        session,
        now: Utc::now(),
        net: NetSettings { https_only: false },
    }
}

fn has_session_cookie(session: &SessionData) -> bool {
    session
        .cookies
        .iter()
        .any(|c| c.name == "JSESSIONID" && c.value == "auth")
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn uportal4_applications_puis_session_reutilisee() {
    let portal = Portal::new("/uPortal/");
    let root = spawn_portal(&portal).await;
    let university = university(&uportal4(&root), &no_cas());
    let mut session = session(Vec::new());

    let fetched = apps(&mut context(&university, &mut session)).await.unwrap();
    assert_eq!(fetched.provider, "portal");
    let ids: Vec<&str> = fetched.data.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "compte-demo",
            "moodle-demo",
            "bibliotheque",
            "planning-demo",
            "sans-titre"
        ]
    );
    assert_eq!(
        fetched.data[0].url,
        "https://apps.exemple.test/compte/protected/authenticated"
    );
    // Sans session : redirection vers `/Login`, ouverture du portail, un seul nouvel essai.
    assert!(has_session_cookie(&session));
    {
        let hits = portal.hits.lock().unwrap();
        assert_eq!((hits.login, hits.data), (1, 2));
    }

    // La session mémorisée suffit : pas de nouveau passage par `/Login`.
    let again = apps(&mut context(&university, &mut session)).await.unwrap();
    assert_eq!(again.data.len(), 5);
    let hits = portal.hits.lock().unwrap();
    assert_eq!((hits.login, hits.data), (1, 3));
}

#[tokio::test]
async fn uportal5_sans_userinfo_profil_depuis_session_json_et_people() {
    let mut portal = Portal::new("/");
    portal.no_userinfo = true;
    let root = spawn_portal(&portal).await;
    let university = university(&uportal5(&root), &no_cas());
    let mut session = session(Vec::new());

    let profile = profile(&mut context(&university, &mut session))
        .await
        .unwrap()
        .data;
    assert_eq!(profile.username, "e00000000000");
    assert_eq!(profile.last_name.as_deref(), Some("Demo"));
    assert_eq!(
        profile.email.as_deref(),
        Some("camille.demo@etu.exemple.test")
    );
    let hits = portal.hits.lock().unwrap();
    assert!(hits.paths.contains(&"api/v5-1/userinfo".to_owned()));
    assert_eq!(
        hits.paths.last().map(String::as_str),
        Some("api/people/e00000000000.json")
    );
}

#[tokio::test]
async fn uportal4_profil_depuis_session_json_et_people() {
    let portal = Portal::new("/uPortal/");
    let root = spawn_portal(&portal).await;
    let university = university(&uportal4(&root), &no_cas());
    let mut session = session(Vec::new());

    let fetched = profile(&mut context(&university, &mut session))
        .await
        .unwrap();
    assert_eq!(fetched.provider, "portal");
    let profile = fetched.data;
    assert_eq!(profile.username, "e00000000000");
    assert_eq!(profile.display_name.as_deref(), Some("Camille Demo"));
    assert_eq!(profile.first_name.as_deref(), Some("Camille"));
    assert_eq!(profile.last_name.as_deref(), Some("Demo"));
    assert_eq!(
        profile.email.as_deref(),
        Some("camille.demo@etu.exemple.test")
    );
    assert_eq!(profile.student_number, None);

    let hits = portal.hits.lock().unwrap();
    assert_eq!(
        hits.paths,
        [
            "api/session.json",
            "api/session.json",
            "api/people/e00000000000.json"
        ]
    );
}

#[tokio::test]
async fn uportal4_profil_reduit_quand_people_est_refuse() {
    let mut portal = Portal::new("/uPortal/");
    portal.people_status = Some(StatusCode::FORBIDDEN);
    let root = spawn_portal(&portal).await;
    let university = university(&uportal4(&root), &no_cas());
    let mut session = session(Vec::new());

    let profile = profile(&mut context(&university, &mut session))
        .await
        .unwrap()
        .data;
    assert_eq!(profile.username, "e00000000000");
    assert_eq!(profile.display_name.as_deref(), Some("Camille Demo"));
    assert_eq!(profile.first_name, None);
    assert_eq!(profile.email, None);
}

#[tokio::test]
async fn uportal5_applications_et_profil() {
    let portal = Portal::new("/");
    let root = spawn_portal(&portal).await;
    let university = university(&uportal5(&root), &no_cas());
    let mut session = session(Vec::new());

    let apps = apps(&mut context(&university, &mut session))
        .await
        .unwrap()
        .data;
    let ids: Vec<&str> = apps.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "webapp-planning",
            "webapp-courrier",
            "webapp-dossier",
            "webapp-assistance"
        ]
    );
    // Résolu sur la racine du faux portail.
    assert_eq!(apps[2].url, format!("{root}/mdw/"));
    // Lien lu dans le fragment du portlet, à la place de la page du portail.
    assert_eq!(apps[3].url, "https://aide.exemple.test/assistance.html");

    let profile = profile(&mut context(&university, &mut session))
        .await
        .unwrap()
        .data;
    assert_eq!(profile.username, "e00000000000");
    assert_eq!(profile.display_name.as_deref(), Some("Camille Demo"));
    assert_eq!(
        profile.email.as_deref(),
        Some("camille.demo@etu.exemple.test")
    );
    assert_eq!(profile.student_number.as_deref(), Some("00000000"));
    // Une seule ouverture du portail pour les deux appels.
    assert_eq!(portal.hits.lock().unwrap().login, 1);
}

#[tokio::test]
async fn session_toujours_invitee_un_seul_nouvel_essai() {
    for (portal_kind, prefix) in [(4, "/uPortal/"), (5, "/")] {
        let mut portal = Portal::new(prefix);
        portal.never_authenticated = true;
        let root = spawn_portal(&portal).await;
        let config = if portal_kind == 4 {
            uportal4(&root)
        } else {
            uportal5(&root)
        };
        let university = university(&config, &no_cas());

        // uPortal 5 : layout invité en 200. uPortal 4 : `session.json` invité en 200.
        let mut session_a = session(Vec::new());
        let error = if portal_kind == 4 {
            profile(&mut context(&university, &mut session_a))
                .await
                .unwrap_err()
        } else {
            apps(&mut context(&university, &mut session_a))
                .await
                .unwrap_err()
        };
        assert_eq!(
            error.code,
            ErrorCode::SessionExpired,
            "uPortal {portal_kind}"
        );
        let hits = portal.hits.lock().unwrap();
        // 1) sans cookie : redirection ; 2) après `/Login` : invité. Pas de troisième essai.
        assert_eq!((hits.login, hits.data), (1, 2), "uPortal {portal_kind}");
    }
}

#[tokio::test]
async fn portail_en_panne_sans_nouvel_essai() {
    let mut portal = Portal::new("/");
    portal.data_override = Some((StatusCode::SERVICE_UNAVAILABLE, ""));
    let root = spawn_portal(&portal).await;
    let university = university(&uportal5(&root), &no_cas());
    let mut session = session(Vec::new());

    let error = apps(&mut context(&university, &mut session))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ServiceUnavailable);
    let hits = portal.hits.lock().unwrap();
    assert_eq!((hits.login, hits.data), (0, 1));
}

#[tokio::test]
async fn reponse_illisible_donne_upstream_changed_sans_donnee() {
    // Le portail répond 200 avec une page HTML : le corps ne doit pas se retrouver dans l'erreur.
    let mut portal = Portal::new("/uPortal/");
    portal.data_override = Some((StatusCode::OK, "<html>e00000000000</html>"));
    let root = spawn_portal(&portal).await;
    let university = university(&uportal4(&root), &no_cas());
    let mut session = session(Vec::new());

    for error in [
        apps(&mut context(&university, &mut session))
            .await
            .unwrap_err(),
        profile(&mut context(&university, &mut session))
            .await
            .unwrap_err(),
    ] {
        assert_eq!(error.code, ErrorCode::UpstreamChanged);
        assert!(!error.to_string().contains("e00000000000"));
    }
}

#[tokio::test]
async fn avec_cas_le_tgc_ouvre_le_portail() {
    let mut portal = Portal::new("/");
    portal.auto_login = false;
    let root = spawn_portal(&portal).await;
    let cas = Cas::default();
    let cas_root = spawn_cas(&cas).await;
    let sso = json!({ "kind": "cas", "base-url": format!("{cas_root}/cas/") });
    let university = university(&uportal5(&root), &sso);
    let mut session = session(vec![tgc()]);

    let apps = apps(&mut context(&university, &mut session))
        .await
        .unwrap()
        .data;
    assert_eq!(apps.len(), 4);
    assert!(has_session_cookie(&session));
    // Le CAS reçoit le service `<portail>/Login`, et le portail le ticket.
    let services = cas.services.lock().unwrap();
    assert_eq!(services.as_slice(), [format!("{root}/Login")]);
    assert_eq!(portal.hits.lock().unwrap().login, 1);
}

#[tokio::test]
async fn avec_cas_sans_tgc_la_session_est_expiree() {
    let mut portal = Portal::new("/");
    portal.auto_login = false;
    let root = spawn_portal(&portal).await;
    let cas = Cas::default();
    let cas_root = spawn_cas(&cas).await;
    let sso = json!({ "kind": "cas", "base-url": format!("{cas_root}/cas/") });
    let university = university(&uportal5(&root), &sso);
    let mut session = session(Vec::new());

    let error = profile(&mut context(&university, &mut session))
        .await
        .unwrap_err();
    // Le CAS affiche son formulaire : pas de mot de passe à lui donner, pas de nouvel essai.
    assert_eq!(error.code, ErrorCode::SessionExpired);
    assert_eq!(portal.hits.lock().unwrap().login, 0);
    assert_eq!(cas.services.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn portail_non_declare() {
    let university = university(&json!({ "kind": "none" }), &no_cas());
    let mut session = session(Vec::new());
    let error = apps(&mut context(&university, &mut session))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ServiceAbsent);
}
