//! Fournisseur `ade-portal-rest` contre un faux CAS et un faux portail ADE locaux : ouverture de la
//! session par le TGC, jeton SSO, `ssologin`, renouvellement du jeton sur 401 (un seul nouvel essai),
//! sélection, événements et arbre des groupes. Aucune donnée réelle, aucun accès à un vrai service.

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Response, StatusCode, Uri, header};
use chrono::{NaiveDate, Utc};
use serde_json::json;
use tonapi_config::ResolvedUniversity;
use tonapi_core::ErrorCode;
use tonapi_providers::{Context, NetSettings, timetable, timetable_groups, timetable_of};
use tonapi_session::SessionData;
use url::Url;

use common::{has_cookie, query};

const EVENTS: &str = include_str!("fixtures/ade_portal_rest/events.json");
const TREE: &str = include_str!("fixtures/ade_portal_rest/resourcestree.json");
const EVENTS_PATH: &str =
    "resources;14961/startdate;09012026/enddate;09302026/level;=8/field;Nom;CODEZ_RESOURCE;OWNER;";

#[derive(Default)]
struct Mock {
    base: String,
    /// Jeton d'API accepté et jeton de rafraîchissement valable.
    valid_access: String,
    valid_refresh: String,
    issued: u32,
    /// Valeur de `PortalResourcesAllSelection`.
    selection: String,
    /// ADE refuse tout jeton sur les événements.
    events_always_401: bool,
    cas_logins: u32,
    sso_calls: u32,
    ssologin_calls: u32,
    refresh_calls: u32,
    refresh_had_authorization: bool,
    events_calls: u32,
    events_paths: Vec<String>,
    selection_calls: u32,
}

type Shared = Arc<Mutex<Mock>>;

fn response(status: StatusCode, content_type: &str, body: String) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .unwrap()
}

fn json_ok(body: &serde_json::Value) -> Response<Body> {
    response(StatusCode::OK, "application/json", body.to_string())
}

fn status(code: StatusCode) -> Response<Body> {
    Response::builder()
        .status(code)
        .body(Body::empty())
        .unwrap()
}

fn bearer_is(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        == Some(format!("Bearer {token}").as_str())
}

/// Jetons émis : `acc-<n>` et `ref-<n>`.
fn issue(mock: &mut Mock) -> (String, String) {
    mock.issued += 1;
    mock.valid_access = format!("acc-{}", mock.issued);
    mock.valid_refresh = format!("ref-{}", mock.issued);
    (mock.valid_access.clone(), mock.valid_refresh.clone())
}

async fn handle(
    State(shared): State<Shared>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response<Body> {
    let mut mock = shared.lock().unwrap();
    let path = uri.path().to_owned();
    match (method.as_str(), path.as_str()) {
        // CAS : le TGC ouvre la session, sinon le formulaire de mot de passe revient.
        ("GET", "/cas/login") => {
            if has_cookie(&headers, "TGC=tgc-demo") {
                mock.cas_logins += 1;
                let service = query(&uri, "service").unwrap_or_default();
                Response::builder()
                    .status(StatusCode::FOUND)
                    .header(header::LOCATION, format!("{service}?ticket=ST-1"))
                    .body(Body::empty())
                    .unwrap()
            } else {
                response(
                    StatusCode::OK,
                    "text/html",
                    r#"<html><body><form method="post" id="fm1" action="login">
                    <input id="username" name="username"><input type="password" name="password">
                    <input type="hidden" name="execution" value="E1"></form></body></html>"#
                        .to_owned(),
                )
            }
        }
        // Portail : le ticket crée la session `/portal`.
        ("GET", "/portal/") => {
            if query(&uri, "ticket").is_some() {
                Response::builder()
                    .header(header::SET_COOKIE, "PSESSION=p1; Path=/portal")
                    .header(header::CONTENT_TYPE, "text/html")
                    .body(Body::from("<html><body>portail</body></html>"))
                    .unwrap()
            } else if has_cookie(&headers, "PSESSION=p1") {
                response(StatusCode::OK, "text/html", "<html></html>".to_owned())
            } else {
                let mut cas = Url::parse(&format!("{}/cas/login", mock.base)).unwrap();
                cas.query_pairs_mut()
                    .append_pair("service", &format!("{}/portal/", mock.base));
                Response::builder()
                    .status(StatusCode::FOUND)
                    .header(header::LOCATION, cas.as_str())
                    .body(Body::empty())
                    .unwrap()
            }
        }
        ("POST", "/portal/sso/accessToken") => {
            mock.sso_calls += 1;
            if has_cookie(&headers, "PSESSION=p1") {
                json_ok(&json!({"token": "sso-ok"}))
            } else {
                json_ok(&json!({"token": "", "errMsg": "No session allowed!"}))
            }
        }
        ("POST", "/portal/RestApi/portal/ssologin") => {
            if !bearer_is(&headers, "sso-ok") {
                return status(StatusCode::UNAUTHORIZED);
            }
            mock.ssologin_calls += 1;
            let (access, refresh) = issue(&mut mock);
            json_ok(&json!({"data": {"projectId": 0, "authData": {
                "projectId": 1, "accessToken": access, "refreshToken": refresh, "resources": []
            }}}))
        }
        ("GET", "/portal/RestApi/portal/refreshtoken") => {
            mock.refresh_calls += 1;
            mock.refresh_had_authorization = headers.contains_key(header::AUTHORIZATION);
            if query(&uri, "refresh_token").as_deref() == Some(mock.valid_refresh.as_str()) {
                let (access, refresh) = issue(&mut mock);
                json_ok(&json!({"data": {"accessToken": access, "refreshToken": refresh}}))
            } else {
                status(StatusCode::UNAUTHORIZED)
            }
        }
        ("GET", p) if p.starts_with("/portal/RestApi/portal/projects/1/") => {
            let rest = p
                .trim_start_matches("/portal/RestApi/portal/projects/1/")
                .to_owned();
            if !bearer_is(&headers, &mock.valid_access) {
                return status(StatusCode::UNAUTHORIZED);
            }
            if rest == "user/properties/name/PortalResourcesAllSelection" {
                mock.selection_calls += 1;
                json_ok(&json!({"data": mock.selection}))
            } else if rest == "resourcestree" {
                response(StatusCode::OK, "application/json", TREE.to_owned())
            } else if let Some(filter) = rest.strip_prefix("resourcesEventsPortal/") {
                mock.events_calls += 1;
                mock.events_paths.push(filter.to_owned());
                if mock.events_always_401 {
                    return status(StatusCode::UNAUTHORIZED);
                }
                response(StatusCode::OK, "application/json", EVENTS.to_owned())
            } else {
                status(StatusCode::NOT_FOUND)
            }
        }
        _ => status(StatusCode::NOT_FOUND),
    }
}

async fn start(selection: &str) -> (Shared, String) {
    let (listener, base) = common::listen().await;
    let shared: Shared = Arc::new(Mutex::new(Mock {
        base: base.clone(),
        selection: selection.to_owned(),
        ..Mock::default()
    }));
    let app = Router::new().fallback(handle).with_state(shared.clone());
    common::spawn(listener, app);
    (shared, base)
}

fn university(base: &str) -> ResolvedUniversity {
    common::university(json!({
        "id": "test",
        "name": "Université de test",
        "sso": {"kind": "cas", "base-url": format!("{base}/cas/")},
        "services": {"timetable": {"provider": "ade-portal-rest", "base-url": format!("{base}/portal/")}},
    }))
}

/// Session d'un étudiant déjà connecté au CAS : seul le TGC est connu.
fn session() -> SessionData {
    common::session(
        "test",
        "e00000000000",
        vec![common::stored_cookie("TGC", "tgc-demo", "/cas")],
    )
}

fn token_key(base: &str) -> String {
    format!("ade-rest.{}", base.trim_start_matches("http://"))
}

fn september() -> (NaiveDate, NaiveDate) {
    (
        NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
    )
}

macro_rules! ctx {
    ($university:expr, $session:expr) => {
        Context {
            university: &$university,
            session: &mut $session,
            now: Utc::now(),
            net: NetSettings { https_only: false },
        }
    };
}

#[tokio::test]
async fn connexion_complete_puis_cours() {
    let (mock, base) = start("14961").await;
    let university = university(&base);
    let mut session = session();
    let (from, to) = september();

    let fetched = timetable(&mut ctx!(university, session), from, to)
        .await
        .unwrap();

    assert_eq!(fetched.provider, "ade-portal-rest");
    let courses = fetched.data;
    assert_eq!(courses.len(), 2);
    assert_eq!(courses[0].id, "501");
    assert_eq!(courses[0].title, "Développement web");
    assert_eq!(courses[0].start.to_rfc3339(), "2026-09-02T08:00:00+02:00");
    assert_eq!(courses[0].rooms, ["Salle 12"]);
    assert_eq!(courses[1].start.to_rfc3339(), "2026-09-03T14:00:00+02:00");

    let mock = mock.lock().unwrap();
    // Premier jeton SSO vide, CAS par le TGC, second jeton SSO, puis `ssologin`.
    assert_eq!(
        (mock.sso_calls, mock.cas_logins, mock.ssologin_calls),
        (2, 1, 1)
    );
    assert_eq!(mock.events_paths, [EVENTS_PATH]);
    // Jetons et cookies du portail repartent dans la session ; pas de mot de passe.
    let stored = session.tokens.get(&token_key(&base)).expect("jetons ADE");
    assert!(stored.contains("acc-1") && stored.contains("ref-1"));
    assert!(session.cookies.iter().any(|c| c.name == "PSESSION"));
}

#[tokio::test]
async fn jetons_gardes_reutilises_sans_nouvelle_connexion() {
    let (mock, base) = start("14961").await;
    let university = university(&base);
    let mut session = session();
    let (from, to) = september();

    timetable(&mut ctx!(university, session), from, to)
        .await
        .unwrap();
    timetable(&mut ctx!(university, session), from, to)
        .await
        .unwrap();

    let mock = mock.lock().unwrap();
    assert_eq!(
        (mock.sso_calls, mock.ssologin_calls, mock.cas_logins),
        (2, 1, 1)
    );
    assert_eq!(mock.events_calls, 2);
}

#[tokio::test]
async fn jeton_expire_renouvele_par_refreshtoken_puis_succes() {
    let (mock, base) = start("14961").await;
    let university = university(&base);
    let mut session = session();
    let (from, to) = september();
    timetable(&mut ctx!(university, session), from, to)
        .await
        .unwrap();
    // ADE n'accepte plus le jeton d'API (30 min écoulées) ; le jeton de rafraîchissement vaut encore.
    mock.lock().unwrap().valid_access = "expire".into();

    let courses = timetable(&mut ctx!(university, session), from, to)
        .await
        .unwrap()
        .data;

    assert_eq!(courses.len(), 2);
    let mock = mock.lock().unwrap();
    assert_eq!(mock.refresh_calls, 1);
    assert!(
        !mock.refresh_had_authorization,
        "refreshtoken sans Authorization"
    );
    assert_eq!(mock.ssologin_calls, 1, "pas de nouvelle connexion");
    let stored = session.tokens.get(&token_key(&base)).unwrap();
    assert!(stored.contains("acc-2") && stored.contains("ref-2"));
}

#[tokio::test]
async fn refresh_refuse_rouvre_la_session_puis_succes() {
    let (mock, base) = start("14961").await;
    let university = university(&base);
    let mut session = session();
    let (from, to) = september();
    timetable(&mut ctx!(university, session), from, to)
        .await
        .unwrap();
    {
        let mut mock = mock.lock().unwrap();
        mock.valid_access = "expire".into();
        mock.valid_refresh = "expire".into();
    }

    let courses = timetable(&mut ctx!(university, session), from, to)
        .await
        .unwrap()
        .data;

    assert_eq!(courses.len(), 2);
    let mock = mock.lock().unwrap();
    assert_eq!(mock.refresh_calls, 1);
    assert_eq!(mock.ssologin_calls, 2);
    // La session `/portal` valait encore : pas de nouveau passage par le CAS.
    assert_eq!(mock.cas_logins, 1);
}

#[tokio::test]
async fn un_seul_nouvel_essai_sur_401() {
    let (mock, base) = start("14961").await;
    mock.lock().unwrap().events_always_401 = true;
    let university = university(&base);
    let mut session = session();
    let (from, to) = september();

    let error = timetable(&mut ctx!(university, session), from, to)
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::SessionExpired);
    assert_eq!(
        mock.lock().unwrap().events_calls,
        2,
        "requête + un seul rejeu"
    );
    assert!(
        !session.tokens.contains_key(&token_key(&base)),
        "jetons refusés oubliés"
    );
}

#[tokio::test]
async fn cas_qui_redemande_le_mot_de_passe_est_une_session_expiree() {
    let (mock, base) = start("14961").await;
    let university = university(&base);
    let mut session = session();
    session.cookies.clear();
    let (from, to) = september();

    let error = timetable(&mut ctx!(university, session), from, to)
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::SessionExpired);
    assert_eq!(mock.lock().unwrap().ssologin_calls, 0);
    assert!(session.tokens.is_empty());
}

#[tokio::test]
async fn selection_vide_donne_aucun_cours_sans_requete_d_evenements() {
    let (mock, base) = start("").await;
    let university = university(&base);
    let mut session = session();
    let (from, to) = september();

    let courses = timetable(&mut ctx!(university, session), from, to)
        .await
        .unwrap()
        .data;

    assert!(courses.is_empty());
    assert_eq!(mock.lock().unwrap().events_calls, 0);
}

#[tokio::test]
async fn plusieurs_ressources_une_requete_chacune_sans_doublon() {
    let (mock, base) = start("14961;14962").await;
    let university = university(&base);
    let mut session = session();
    let (from, to) = september();

    let courses = timetable(&mut ctx!(university, session), from, to)
        .await
        .unwrap()
        .data;

    // Le même événement revient sous chaque ressource : une seule séance.
    assert_eq!(courses.len(), 2);
    assert_eq!(mock.lock().unwrap().events_calls, 2);
}

#[tokio::test]
async fn ressources_demandees_a_la_place_de_la_selection() {
    // La sélection de l'étudiant (14961) n'est ni lue ni interrogée.
    let (mock, base) = start("14961").await;
    let university = university(&base);
    let mut session = session();
    let (from, to) = september();
    let asked = ["777".to_owned(), "888".to_owned(), "777".to_owned()];

    let fetched = timetable_of(&mut ctx!(university, session), from, to, &asked)
        .await
        .unwrap();

    assert_eq!(fetched.provider, "ade-portal-rest");
    assert_eq!(fetched.data.len(), 2);
    assert_eq!(fetched.data[0].color.as_deref(), Some("120,140,160"));
    assert_eq!(fetched.data[0].activity_id.as_deref(), Some("42"));
    let mock = mock.lock().unwrap();
    assert_eq!(mock.selection_calls, 0);
    let asked_of_ade: Vec<&str> = mock
        .events_paths
        .iter()
        .map(|path| path.split('/').next().unwrap())
        .collect();
    assert_eq!(asked_of_ade, ["resources;777", "resources;888"]);
}

#[tokio::test]
async fn ressource_demandee_invalide_refusee_sans_requete() {
    let (mock, base) = start("14961").await;
    let university = university(&base);
    let mut session = session();
    let (from, to) = september();

    for asked in [
        vec!["-5".to_owned()],
        vec!["12".to_owned(), "tp1".to_owned()],
    ] {
        let error = timetable_of(&mut ctx!(university, session), from, to, &asked)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }
    let too_many: Vec<String> = (1..=31).map(|id| id.to_string()).collect();
    let error = timetable_of(&mut ctx!(university, session), from, to, &too_many)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert_eq!(mock.lock().unwrap().sso_calls, 0);
}

#[tokio::test]
async fn plage_inversee_refusee_sans_requete() {
    let (mock, base) = start("14961").await;
    let university = university(&base);
    let mut session = session();
    let (from, to) = september();

    let error = timetable(&mut ctx!(university, session), to, from)
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert_eq!(mock.lock().unwrap().sso_calls, 0);
}

#[tokio::test]
async fn groupes_avec_selection() {
    let (_mock, base) = start("14962").await;
    let university = university(&base);
    let mut session = session();

    let fetched = timetable_groups(&mut ctx!(university, session))
        .await
        .unwrap();

    assert_eq!(fetched.provider, "ade-portal-rest");
    let groups: Vec<_> = fetched
        .data
        .iter()
        .map(|g| (g.id.as_str(), g.name.as_str(), g.selected))
        .collect();
    assert_eq!(
        groups,
        [
            ("14961", "TP 1", false),
            ("14962", "TP 2", true),
            ("14970", "Amphi", false)
        ]
    );
    assert_eq!(
        fetched.data[1].path,
        [
            "Département Démo",
            "BUT Informatique 2",
            "Groupe TD 1",
            "TP 2"
        ]
    );
}
