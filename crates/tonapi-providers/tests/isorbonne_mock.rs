//! Fournisseur `portal` (iSorbonne) contre un faux portail local et un faux CAS : la page
//! d'accueil `ENT/secure/index.xhtml` renvoie au CAS sans session et ouvre la sienne au retour du
//! ticket. Aucune requête ne quitte cet ordinateur.

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

const HOME: &str = include_str!("fixtures/isorbonne/accueil.html");
const HOME_PATH: &str = "/ENT/secure/index.xhtml";

// Faux portail

#[derive(Default)]
struct Portal {
    /// Adresse du faux CAS (`http://localhost:<port>/cas/`).
    cas: String,
    /// Sa propre adresse (`http://127.0.0.1:<port>`).
    base: String,
    /// Sessions ouvertes jusqu'ici : la dernière est la seule reconnue.
    sessions: u32,
    /// Prochaines pages d'accueil servies sans le lien de déconnexion ni les tuiles.
    bare_pages: u32,
    /// Le portail répond 503 à tout.
    down: bool,
    /// Requêtes reçues sur la page d'accueil.
    home: u32,
}

type Shared = Arc<Mutex<Portal>>;

fn page(status: StatusCode, body: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

async fn portal_handler(
    State(state): State<Shared>,
    uri: Uri,
    headers: HeaderMap,
) -> Response<Body> {
    let mut portal = state.lock().unwrap();
    if uri.path() != HOME_PATH {
        return page(StatusCode::NOT_FOUND, "<html>404</html>");
    }
    portal.home += 1;
    if portal.down {
        return page(StatusCode::SERVICE_UNAVAILABLE, "<html>Maintenance</html>");
    }
    let home = format!("{}{HOME_PATH}", portal.base);
    // Retour du CAS : le ticket ouvre une session, puis la page sans le ticket.
    if query(&uri, "ticket").as_deref() == Some(CAS_TICKET) {
        portal.sessions += 1;
        let cookie = format!("JSESSIONID=s{}; Path=/ENT", portal.sessions);
        let mut response = common::redirect(&home);
        response
            .headers_mut()
            .insert(header::SET_COOKIE, cookie.parse().unwrap());
        return response;
    }
    let signed_in =
        portal.sessions > 0 && has_cookie(&headers, &format!("JSESSIONID=s{}", portal.sessions));
    if !signed_in {
        let service: String = url::form_urlencoded::byte_serialize(home.as_bytes()).collect();
        return common::redirect(&format!("{}login?service={service}", portal.cas));
    }
    if portal.bare_pages > 0 {
        portal.bare_pages -= 1;
        return page(StatusCode::OK, "<html><body>Bienvenue</body></html>");
    }
    page(StatusCode::OK, HOME)
}

/// Faux portail et faux CAS, et l'université qui les désigne.
struct Campus {
    portal: Shared,
    cas: Cas,
    university: ResolvedUniversity,
}

async fn campus() -> Campus {
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
    let university = common::university(json!({
        "id": "test",
        "name": "Université de test",
        "sso": { "kind": "cas", "base-url": cas_base },
        "portal": { "kind": "isorbonne", "base-url": format!("{portal_base}/ENT/") },
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
        common::stored_cookie("JSESSIONID", &format!("s{}", portal.sessions), "/ENT")
    }

    fn home_requests(&self) -> u32 {
        self.portal.lock().unwrap().home
    }
}

fn session(cookies: Vec<StoredCookie>) -> SessionData {
    common::session("test", "cdemo", cookies)
}

fn context<'a>(university: &'a ResolvedUniversity, session: &'a mut SessionData) -> Context<'a> {
    common::context(university, session, Utc::now())
}

fn portal_session(session: &SessionData) -> Option<&str> {
    session
        .cookies
        .iter()
        .find(|c| c.name == "JSESSIONID")
        .map(|c| c.value.as_str())
}

// Tests

#[tokio::test]
async fn connexion_par_la_page_d_accueil_puis_profil_et_tuiles() {
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
    {
        let log = campus.cas.log.lock().unwrap();
        assert_eq!(log.password_posts, 1);
        assert!(log.services.first().unwrap().ends_with(HOME_PATH));
    }
    assert!(session.cookies.iter().any(|c| c.name == "TGC"));
    assert_eq!(portal_session(&session), Some("s1"));

    let fetched = profile(&mut context(&campus.university, &mut session))
        .await
        .unwrap();
    assert_eq!(fetched.provider, "isorbonne");
    // Le portail ne nomme pas l'étudiant : l'identifiant est celui de la connexion.
    assert_eq!(fetched.data.username, CAS_USER);
    assert_eq!(
        fetched.data.display_name.as_deref(),
        Some("Camille Le Demo")
    );
    assert_eq!(fetched.data.first_name.as_deref(), Some("Camille"));
    assert_eq!(fetched.data.last_name.as_deref(), Some("Le Demo"));

    let fetched = apps(&mut context(&campus.university, &mut session))
        .await
        .unwrap();
    assert_eq!(fetched.provider, "isorbonne");
    let read: Vec<_> = fetched
        .data
        .iter()
        .map(|app| (app.id.as_str(), app.category.as_deref()))
        .collect();
    assert_eq!(
        read,
        [
            ("35", Some("Accueil")),
            ("54", Some("Etudiant")),
            ("61", Some("Etudiant"))
        ]
    );
    // Un lien relatif part de la racine de l'hôte du portail.
    assert_eq!(
        fetched.data.get(2).unwrap().url,
        format!(
            "{}/ENT/secure/aide.xhtml",
            campus.portal.lock().unwrap().base
        )
    );
}

#[tokio::test]
async fn une_requete_avec_la_seule_session_du_portail() {
    let campus = campus().await;
    let mut session = session(vec![campus.portal_cookie()]);

    let profile = profile(&mut context(&campus.university, &mut session))
        .await
        .unwrap()
        .data;
    assert_eq!(profile.username, "cdemo");
    assert_eq!(profile.display_name.as_deref(), Some("Camille Le Demo"));
    assert_eq!(campus.home_requests(), 1);
    assert!(campus.cas.log.lock().unwrap().services.is_empty());
}

#[tokio::test]
async fn session_du_portail_perdue_rouverte_avec_le_tgc() {
    let campus = campus().await;
    let stale = common::stored_cookie("JSESSIONID", "perdu", "/ENT");
    let mut session = session(vec![stale, common::tgc()]);

    let fetched = apps(&mut context(&campus.university, &mut session))
        .await
        .unwrap();
    assert_eq!(fetched.data.len(), 3);
    // Accueil sans session, retour du CAS avec le ticket, accueil.
    assert_eq!(campus.home_requests(), 3);
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
    let stale = common::stored_cookie("JSESSIONID", "perdu", "/ENT");
    let mut session = session(vec![stale]);

    let error = profile(&mut context(&campus.university, &mut session))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::SessionExpired);
    assert_eq!(campus.home_requests(), 1);
    let log = campus.cas.log.lock().unwrap();
    assert_eq!((log.forms, log.password_posts), (1, 0));
}

#[tokio::test]
async fn page_sans_les_tuiles_un_seul_nouvel_essai() {
    let campus = campus().await;
    let mut session = session(vec![campus.portal_cookie(), common::tgc()]);
    campus.portal.lock().unwrap().bare_pages = 1;

    // La page servie sans le lien de déconnexion, puis, sans les anciens cookies du portail, le
    // CAS, son ticket et la page.
    let fetched = apps(&mut context(&campus.university, &mut session))
        .await
        .unwrap();
    assert_eq!(fetched.data.len(), 3);
    assert_eq!(campus.home_requests(), 4);
    assert_eq!(portal_session(&session), Some("s2"));

    // Encore sans les tuiles après le nouvel essai : `session_expired`, sans boucle.
    campus.portal.lock().unwrap().home = 0;
    campus.portal.lock().unwrap().bare_pages = 10;
    let error = apps(&mut context(&campus.university, &mut session))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::SessionExpired);
    assert_eq!(campus.home_requests(), 4);
}

#[tokio::test]
async fn portail_en_panne() {
    let campus = campus().await;
    let mut session = session(vec![campus.portal_cookie(), common::tgc()]);
    campus.portal.lock().unwrap().down = true;

    let error = profile(&mut context(&campus.university, &mut session))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ServiceUnavailable);
    assert_eq!(campus.home_requests(), 1);
}
