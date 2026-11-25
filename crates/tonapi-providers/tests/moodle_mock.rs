//! Fournisseurs Moodle contre un faux Moodle local : entrée par le CAS puis par Shibboleth,
//! `sesskey`, nouvel essai unique sur `invalidsesskey`, devoirs rendus lus dans le même lot que les
//! « à faire », jeton du web service.
//!
//! Le faux serveur joue aussi le CAS, le WAYF et l'IdP (même hôte, chemins différents).

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Response, StatusCode, Uri, header};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use tonapi_config::{
    CasFlavor, CasSso, Credentials, HomeworkService, Links, MoodleEntry, PortalConfig,
    ResolvedUniversity, SamlConfig, ServiceKind, ServiceOptions, Services, SignIn, Sso, Status,
    UniversityFile,
};
use tonapi_core::{ErrorCode, Homework, Problem};
use tonapi_providers::{Context, Fetched, NetSettings, homework, service_login};
use tonapi_session::SessionData;
use url::Url;

use common::{has_cookie, query};

const AJAX_EVENTS: &str = include_str!("fixtures/moodle/ajax_events.json");
const AJAX_UPCOMING: &str = include_str!("fixtures/moodle/ajax_upcoming.json");
const WS_EVENTS: &str = include_str!("fixtures/moodle/ws_events.json");
const INVALID_SESSKEY: &str = include_str!("fixtures/moodle/ajax_invalidsesskey.json");
const SERVICE_NOT_AVAILABLE: &str = include_str!("fixtures/moodle/ajax_servicenotavailable.json");
const INVALID_TOKEN: &str = include_str!("fixtures/moodle/ws_invalidtoken.json");
const ACCESS_EXCEPTION: &str = include_str!("fixtures/moodle/ws_accessexception.json");
const GUEST_PAGE: &str = include_str!("fixtures/moodle/my_page_guest.html");

const TODO_FUNCTION: &str = "core_calendar_get_action_events_by_timesort";
const UPCOMING_FUNCTION: &str = "core_calendar_get_calendar_upcoming_view";
const WS_TOKEN: &str = "0123456789abcdef0123456789abcdef";
const IDP: &str = "urn:test:idp";

/// Ce que le faux Moodle répond aux appels de données.
#[derive(Default, Clone, Copy, PartialEq)]
enum Reply {
    #[default]
    Normal,
    /// Tous les `sesskey` sont refusés, même ceux d'une session toute neuve.
    InvalidSesskey,
    NotAvailable,
    Paged,
    Http503,
    TokenRefused,
    AccessException,
}

/// Ce que le faux Moodle répond au second appel d'un lot (événements à venir).
#[derive(Default, Clone, Copy, PartialEq)]
enum Upcoming {
    /// Aucun événement à venir.
    #[default]
    Empty,
    /// La fixture : deux devoirs rendus parmi d'autres événements.
    Submitted,
    /// Fonction refusée : un élément d'erreur.
    Refused,
    /// Second appel sans réponse : un seul élément dans le tableau.
    Missing,
}

#[derive(Default)]
struct Mock {
    reply: Reply,
    upcoming: Upcoming,
    /// Sessions web ouvertes sur Moodle (tickets CAS ou SAML acceptés).
    sessions: u32,
    cas_hits: u32,
    /// `(sesskey, arguments du premier appel)` de chaque requête AJAX.
    ajax: Vec<(String, Value)>,
    /// Fonctions demandées par chaque requête AJAX, dans l'ordre du lot.
    ajax_functions: Vec<Vec<String>>,
    /// `(URI, corps)` de chaque appel `server.php`.
    ws: Vec<(String, String)>,
    /// Appels à `login/token.php`.
    token_posts: u32,
    wayf_posts: Vec<String>,
    /// Moodle répond par une page d'invité, comme un CAS `gateway=true` sans TGC.
    guest_landing: bool,
    /// Le formulaire du WAYF vise un hôte étranger.
    wayf_foreign: bool,
}

impl Mock {
    fn sesskey(&self) -> String {
        format!("KEYSESS{:03}", self.sessions)
    }

    fn cookie(&self) -> String {
        format!("MoodleSession=sess{}", self.sessions)
    }
}

type Shared = Arc<Mutex<Mock>>;

fn redirect(location: &str, cookie: Option<&str>) -> Response<Body> {
    let mut response = Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location);
    if let Some(cookie) = cookie {
        response = response.header(header::SET_COOKIE, cookie);
    }
    response.body(Body::empty()).unwrap()
}

fn page(body: impl Into<String>) -> Response<Body> {
    Response::builder()
        .header(header::CONTENT_TYPE, "text/html")
        .body(Body::from(body.into()))
        .unwrap()
}

fn json_body(status: StatusCode, body: impl Into<String>) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.into()))
        .unwrap()
}

fn enc(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn my_page(sesskey: &str) -> String {
    format!(
        r#"<html><head><script>M.cfg = {{"sesskey":"{sesskey}","userId":4242}};</script></head>
        <body class="path-my"><p>Camille Demo</p></body></html>"#
    )
}

const CAS_FORM: &str = r#"<html><body><form method="post" id="fm1" action="login">
    <input name="username"><input type="password" name="password">
    <input type="hidden" name="execution" value="E1"></form></body></html>"#;

fn wayf_page(action: &str) -> String {
    format!(
        r#"<html><body><form action="{action}" method="post">
        <input type="hidden" name="return" value="/Shibboleth.sso/Login">
        <select name="user_idp"><option value="urn:autre">Autre</option><option value="{IDP}">Test</option></select>
        <input type="submit" name="Select" value="Sélection"></form></body></html>"#
    )
}

const IDP_PAGE: &str = r#"<html><body onload="document.forms[0].submit()">
    <form method="post" action="/Shibboleth.sso/SAML2/POST">
    <input type="hidden" name="RelayState" value="rs"><input type="hidden" name="SAMLResponse" value="FAKE-SAML">
    </form></body></html>"#;

/// Réponse AJAX : `Normal` sert la fixture, `Paged` deux pages (50 puis 1 événement). Un lot de
/// deux appels reçoit en plus l'élément choisi par `Mock::upcoming`.
fn ajax_reply(s: &Mock, sesskey: &str, args: &Value, calls: usize) -> Response<Body> {
    let valid = s.sessions > 0 && sesskey == s.sesskey();
    match s.reply {
        Reply::Http503 => json_body(StatusCode::SERVICE_UNAVAILABLE, "{}"),
        Reply::InvalidSesskey => json_body(StatusCode::OK, INVALID_SESSKEY),
        Reply::NotAvailable => json_body(StatusCode::OK, SERVICE_NOT_AVAILABLE),
        _ if !valid => json_body(StatusCode::OK, INVALID_SESSKEY),
        Reply::Paged => {
            let (range, last) = if args.get("aftereventid").is_some() {
                (51..=51, 51)
            } else {
                (1..=50, 50)
            };
            let events: Vec<Value> = range
                .map(|i| json!({ "id": i, "name": format!("Devoir {i}"), "timesort": 1_792_533_540 + i * 3600 }))
                .collect();
            let data = json!([{ "error": false, "data": { "events": events, "lastid": last } }]);
            json_body(StatusCode::OK, data.to_string())
        }
        _ => {
            let mut items: Vec<Value> = serde_json::from_str(AJAX_EVENTS).unwrap();
            if calls > 1 {
                match s.upcoming {
                    Upcoming::Empty => {
                        items.push(json!({ "error": false, "data": { "events": [] } }));
                    }
                    Upcoming::Submitted => items.push(serde_json::from_str(AJAX_UPCOMING).unwrap()),
                    Upcoming::Refused => items.push(json!({
                        "error": true,
                        "exception": { "message": "SECRET-MESSAGE", "errorcode": "servicenotavailable" }
                    })),
                    Upcoming::Missing => {}
                }
            }
            json_body(StatusCode::OK, Value::Array(items).to_string())
        }
    }
}

#[allow(clippy::too_many_lines)] // un faux serveur entier, route par route
async fn handle(
    State(state): State<Shared>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response<Body> {
    let mut s = state.lock().unwrap();
    let host = headers
        .get(header::HOST)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let origin = format!("http://{host}");

    match (method, uri.path()) {
        // Moodle, entrée CAS
        (Method::GET, "/login/index.php") => {
            if s.guest_landing {
                page(GUEST_PAGE)
            } else if query(&uri, "ticket").as_deref() == Some("ST-1") {
                s.sessions += 1;
                let cookie = format!("{}; Path=/", s.cookie());
                redirect("/my/", Some(&cookie))
            } else if s.sessions > 0 && has_cookie(&headers, &s.cookie()) {
                page(my_page(&s.sesskey()))
            } else {
                let service = format!("{origin}/login/index.php?authCAS=CAS");
                redirect(&format!("/cas/login?service={}", enc(&service)), None)
            }
        }
        (Method::GET, "/my/") => {
            if s.sessions > 0 && has_cookie(&headers, &s.cookie()) {
                page(my_page(&s.sesskey()))
            } else {
                page(GUEST_PAGE)
            }
        }
        (Method::GET, "/cas/login") => {
            s.cas_hits += 1;
            if has_cookie(&headers, "TGC=tgt-ok") {
                let service = query(&uri, "service").unwrap_or_default();
                let sep = if service.contains('?') { '&' } else { '?' };
                redirect(&format!("{service}{sep}ticket=ST-1"), None)
            } else {
                page(CAS_FORM)
            }
        }
        // Moodle, entrée Shibboleth
        (Method::GET, "/auth/shibboleth/index.php") => redirect("/Shibboleth.sso/Login", None),
        (Method::GET, "/Shibboleth.sso/Login") => redirect("/wayf", None),
        (Method::GET, "/wayf") => {
            let action = if s.wayf_foreign {
                "http://hote-etranger.invalid/collecte"
            } else {
                "/wayf/select"
            };
            page(wayf_page(action))
        }
        (Method::POST, "/wayf/select") => {
            s.wayf_posts.push(body.clone());
            let chosen = url::form_urlencoded::parse(body.as_bytes())
                .find(|(k, _)| k == "user_idp")
                .map(|(_, v)| v.into_owned());
            if chosen.as_deref() == Some(IDP) {
                redirect("/idp/sso", None)
            } else {
                json_body(StatusCode::BAD_REQUEST, "{}")
            }
        }
        (Method::GET, "/idp/sso") => {
            if query(&uri, "ticket").is_some() {
                page(IDP_PAGE)
            } else {
                let service = format!("{origin}/idp/sso");
                redirect(&format!("/cas/login?service={}", enc(&service)), None)
            }
        }
        (Method::POST, "/Shibboleth.sso/SAML2/POST") => {
            if body.contains("SAMLResponse=FAKE-SAML") {
                s.sessions += 1;
                let cookie = format!("{}; Path=/", s.cookie());
                redirect("/my/", Some(&cookie))
            } else {
                json_body(StatusCode::BAD_REQUEST, "{}")
            }
        }
        // Données
        (Method::POST, "/lib/ajax/service.php") => {
            let sesskey = query(&uri, "sesskey").unwrap_or_default();
            let calls = serde_json::from_str::<Vec<Value>>(&body).unwrap_or_default();
            let args = calls
                .first()
                .and_then(|call| call["args"].as_object().cloned())
                .map_or(Value::Null, Value::Object);
            s.ajax.push((sesskey.clone(), args.clone()));
            s.ajax_functions.push(
                calls
                    .iter()
                    .map(|call| call["methodname"].as_str().unwrap_or_default().to_owned())
                    .collect(),
            );
            // La session web doit accompagner le `sesskey`.
            let session_ok = s.sessions > 0 && has_cookie(&headers, &s.cookie());
            if s.reply == Reply::Normal && !session_ok {
                return json_body(StatusCode::OK, INVALID_SESSKEY);
            }
            ajax_reply(&s, &sesskey, &args, calls.len())
        }
        (Method::POST, "/moodle/login/token.php") => {
            s.token_posts += 1;
            let good = body.contains("username=camille.demo")
                && body.contains("password=mot-de-passe-de-test")
                && body.contains("service=moodle_mobile_app");
            if good {
                json_body(
                    StatusCode::OK,
                    format!(r#"{{"token":"{WS_TOKEN}","privatetoken":"prive"}}"#),
                )
            } else {
                json_body(
                    StatusCode::OK,
                    r#"{"error":"Identifiants incorrects","errorcode":"invalidlogin"}"#,
                )
            }
        }
        (Method::POST, "/moodle/webservice/rest/server.php") => {
            s.ws.push((uri.to_string(), body.clone()));
            let token_ok = body.contains(&format!("wstoken={WS_TOKEN}"));
            match s.reply {
                Reply::TokenRefused => json_body(StatusCode::OK, INVALID_TOKEN),
                Reply::AccessException => json_body(StatusCode::OK, ACCESS_EXCEPTION),
                _ if !token_ok => json_body(StatusCode::OK, INVALID_TOKEN),
                _ => json_body(StatusCode::OK, WS_EVENTS),
            }
        }
        _ => json_body(StatusCode::NOT_FOUND, "{}"),
    }
}

async fn start(setup: impl FnOnce(&mut Mock)) -> (Shared, Url) {
    let (listener, root) = common::listen().await;
    let base = Url::parse(&format!("{root}/")).unwrap();
    let mut mock = Mock::default();
    setup(&mut mock);
    let state: Shared = Arc::new(Mutex::new(mock));
    let app = Router::new().fallback(handle).with_state(state.clone());
    common::spawn(listener, app);
    (state, base)
}

fn university_with(
    base: &Url,
    homework: HomeworkService,
    saml: Option<SamlConfig>,
) -> ResolvedUniversity {
    ResolvedUniversity {
        dir: PathBuf::new(),
        university: UniversityFile {
            schema_version: 1,
            id: "universite-test".to_owned(),
            name: "Université de test".to_owned(),
            short_name: None,
            status: Status::default(),
            timezone: "Europe/Paris".to_owned(),
            links: Links::default(),
            sso: Sso::Cas(Box::new(CasSso {
                base_url: base.join("cas/").unwrap(),
                flavor: CasFlavor::Apereo,
                mfa: None,
                saml,
            })),
            portal: PortalConfig::None,
            services: Services {
                homework: Some(homework),
                ..Services::default()
            },
            tls: None,
        },
        establishments: BTreeMap::new(),
    }
}

fn ajax(base: &Url, entry: MoodleEntry) -> HomeworkService {
    HomeworkService::MoodleAjax {
        base_url: base.clone(),
        entry,
        options: ServiceOptions::default(),
    }
}

fn webservice(base: &Url, sign_in: SignIn) -> HomeworkService {
    HomeworkService::MoodleWebservice {
        base_url: base.join("moodle/").unwrap(),
        sign_in,
        options: ServiceOptions::default(),
    }
}

const LOCAL: SignIn = SignIn::LocalPassword {
    credentials: Credentials::University,
};

fn saml() -> SamlConfig {
    SamlConfig {
        wayf_url: Url::parse("http://wayf.invalid/").unwrap(),
        idp_entity_id: IDP.to_owned(),
        allowed_hosts: Vec::new(),
    }
}

/// Session d'un étudiant, avec ou sans ticket-granting du CAS.
fn session(with_tgc: bool) -> SessionData {
    let tgc = common::stored_cookie("TGC", "tgt-ok", "/cas/");
    let cookies = if with_tgc { vec![tgc] } else { Vec::new() };
    SessionData {
        expires_at: 3600,
        ..common::session("universite-test", "e00000000000", cookies)
    }
}

async fn get_homework(
    university: &ResolvedUniversity,
    session: &mut SessionData,
) -> Result<Fetched<Vec<Homework>>, Problem> {
    let mut ctx = Context {
        university,
        session,
        now: Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap(),
        net: NetSettings {
            https_only: false,
            user_agent: None,
        },
    };
    homework(&mut ctx).await
}

fn slot(base: &Url, kind: &str) -> String {
    format!(
        "moodle.{kind}.{}:{}",
        base.host_str().unwrap(),
        base.port().unwrap()
    )
}

fn titles(fetched: &Fetched<Vec<Homework>>) -> Vec<&str> {
    fetched.data.iter().map(|h| h.title.as_str()).collect()
}

// moodle-ajax, entrée par le CAS

#[tokio::test]
async fn ajax_ouvre_la_session_par_le_cas_puis_la_reutilise() {
    let (state, base) = start(|_| {}).await;
    let university = university_with(&base, ajax(&base, MoodleEntry::Cas), None);
    let mut session = session(true);

    let fetched = get_homework(&university, &mut session).await.unwrap();
    assert_eq!(fetched.provider, "moodle-ajax");
    assert_eq!(titles(&fetched), ["Rendu TP1", "Quiz de synthèse"]);
    {
        let s = state.lock().unwrap();
        assert_eq!(s.sessions, 1);
        // Le sesskey et les cookies de Moodle repartent dans la session de l'étudiant.
        assert_eq!(
            session.tokens.get(&slot(&base, "sesskey")),
            Some(&s.sesskey())
        );
        assert_eq!(s.ajax.len(), 1);
        // Une seule requête porte les « à faire » et les événements à venir.
        assert_eq!(s.ajax_functions[0], [TODO_FUNCTION, UPCOMING_FUNCTION]);
        let args = &s.ajax[0].1;
        assert_eq!(args["limitnum"], 50);
        assert_eq!(args["limittononsuspendedevents"], true);
        assert!(args["timesortfrom"].as_i64().unwrap() < args["timesortto"].as_i64().unwrap());
    }
    assert!(session.cookies.iter().any(|c| c.name == "MoodleSession"));
    assert!(session.cookies.iter().any(|c| c.name == "TGC"));

    // Deuxième appel : ni passage par le CAS ni nouvelle session.
    let cas_before = state.lock().unwrap().cas_hits;
    let again = get_homework(&university, &mut session).await.unwrap();
    assert_eq!(again.data, fetched.data);
    let s = state.lock().unwrap();
    assert_eq!(s.sessions, 1);
    assert_eq!(s.cas_hits, cas_before);
    assert_eq!(s.ajax.len(), 2);
}

#[tokio::test]
async fn ajax_un_sesskey_perime_ne_coute_qu_un_nouvel_essai() {
    let (state, base) = start(|_| {}).await;
    let university = university_with(&base, ajax(&base, MoodleEntry::Cas), None);
    let mut session = session(true);
    session
        .tokens
        .insert(slot(&base, "sesskey"), "STALE00001".to_owned());

    let fetched = get_homework(&university, &mut session).await.unwrap();
    assert_eq!(fetched.data.len(), 2);
    let s = state.lock().unwrap();
    // Un appel refusé, la session rouverte une fois, un appel réussi.
    assert_eq!(s.ajax.len(), 2);
    assert_eq!(s.ajax[0].0, "STALE00001");
    assert_eq!(s.ajax[1].0, s.sesskey());
    assert_eq!(s.sessions, 1);
    assert_eq!(
        session.tokens.get(&slot(&base, "sesskey")),
        Some(&s.sesskey())
    );
}

#[tokio::test]
async fn ajax_ne_boucle_pas_si_la_session_neuve_est_refusee() {
    let (state, base) = start(|s| s.reply = Reply::InvalidSesskey).await;
    let university = university_with(&base, ajax(&base, MoodleEntry::Cas), None);
    let mut session = session(true);

    let problem = get_homework(&university, &mut session).await.unwrap_err();
    assert_eq!(problem.code, ErrorCode::SessionExpired);
    assert!(!format!("{problem:?}").contains("SECRET"));
    let s = state.lock().unwrap();
    // Session ouverte une fois, appel refusé, pas de seconde ouverture ni de troisième appel.
    assert_eq!(s.sessions, 1);
    assert_eq!(s.ajax.len(), 1);
    assert!(!session.tokens.contains_key(&slot(&base, "sesskey")));
}

#[tokio::test]
async fn ajax_sesskey_perime_puis_refuse_apres_reconnexion_s_arrete_a_deux_appels() {
    let (state, base) = start(|s| s.reply = Reply::InvalidSesskey).await;
    let university = university_with(&base, ajax(&base, MoodleEntry::Cas), None);
    let mut session = session(true);
    session
        .tokens
        .insert(slot(&base, "sesskey"), "STALE00001".to_owned());

    let problem = get_homework(&university, &mut session).await.unwrap_err();
    assert_eq!(problem.code, ErrorCode::SessionExpired);
    let s = state.lock().unwrap();
    assert_eq!(s.ajax.len(), 2);
    assert_eq!(s.sessions, 1);
}

#[tokio::test]
async fn ajax_cas_qui_redemande_le_mot_de_passe_donne_session_expiree() {
    let (state, base) = start(|_| {}).await;
    let university = university_with(&base, ajax(&base, MoodleEntry::Cas), None);
    let mut session = session(false);

    let problem = get_homework(&university, &mut session).await.unwrap_err();
    assert_eq!(problem.code, ErrorCode::SessionExpired);
    let s = state.lock().unwrap();
    assert!(s.ajax.is_empty());
    assert_eq!(s.sessions, 0);
}

#[tokio::test]
async fn ajax_page_d_invite_n_est_pas_prise_pour_une_session() {
    let (state, base) = start(|s| s.guest_landing = true).await;
    let university = university_with(&base, ajax(&base, MoodleEntry::Cas), None);
    let mut session = session(true);

    let problem = get_homework(&university, &mut session).await.unwrap_err();
    assert_eq!(problem.code, ErrorCode::SessionExpired);
    // Le sesskey de l'invité n'est ni utilisé ni gardé.
    assert!(state.lock().unwrap().ajax.is_empty());
    assert!(session.tokens.is_empty());
}

#[tokio::test]
async fn ajax_erreurs_de_moodle_sans_leur_message() {
    let (_, base) = start(|s| s.reply = Reply::NotAvailable).await;
    let university = university_with(&base, ajax(&base, MoodleEntry::Cas), None);
    let problem = get_homework(&university, &mut session(true))
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::ServiceUnavailable);
    assert!(!format!("{problem:?}").contains("SECRET"));

    let (_, base) = start(|s| s.reply = Reply::Http503).await;
    let university = university_with(&base, ajax(&base, MoodleEntry::Cas), None);
    let problem = get_homework(&university, &mut session(true))
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::ServiceUnavailable);
}

#[tokio::test]
async fn ajax_lit_les_pages_suivantes() {
    let (state, base) = start(|s| s.reply = Reply::Paged).await;
    let university = university_with(&base, ajax(&base, MoodleEntry::Cas), None);
    let mut session = session(true);

    let fetched = get_homework(&university, &mut session).await.unwrap();
    assert_eq!(fetched.data.len(), 51);
    assert_eq!(fetched.data[50].title, "Devoir 51");
    let s = state.lock().unwrap();
    assert_eq!(s.ajax.len(), 2);
    assert_eq!(s.ajax[1].1["aftereventid"], 50);
    // Les événements à venir ne sont demandés qu'avec la première page.
    assert_eq!(s.ajax_functions[1], [TODO_FUNCTION]);
}

#[tokio::test]
async fn ajax_ajoute_les_devoirs_rendus_sans_requete_de_plus() {
    let (state, base) = start(|s| s.upcoming = Upcoming::Submitted).await;
    let university = university_with(&base, ajax(&base, MoodleEntry::Cas), None);
    let mut session = session(true);

    let fetched = get_homework(&university, &mut session).await.unwrap();
    let seen: Vec<(&str, Option<bool>, Option<&str>)> = fetched
        .data
        .iter()
        .map(|h| (h.title.as_str(), h.submitted, h.kind.as_deref()))
        .collect();
    assert_eq!(
        seen,
        [
            ("Rendu TP1", Some(false), Some("assign")),
            ("Compte rendu réseaux", Some(true), Some("assign")),
            ("QCM chapitre 2", Some(true), Some("quiz")),
            ("Quiz de synthèse", Some(false), Some("quiz")),
        ]
    );
    let s = state.lock().unwrap();
    assert_eq!(s.ajax.len(), 1);
    assert_eq!(s.ajax_functions[0], [TODO_FUNCTION, UPCOMING_FUNCTION]);
}

#[tokio::test]
async fn ajax_garde_les_devoirs_a_faire_si_les_evenements_a_venir_manquent() {
    for upcoming in [Upcoming::Refused, Upcoming::Missing] {
        let (state, base) = start(|s| s.upcoming = upcoming).await;
        let university = university_with(&base, ajax(&base, MoodleEntry::Cas), None);
        let mut session = session(true);

        let fetched = get_homework(&university, &mut session).await.unwrap();
        assert_eq!(titles(&fetched), ["Rendu TP1", "Quiz de synthèse"]);
        // Ni nouvel essai ni reconnexion pour ce second appel.
        let s = state.lock().unwrap();
        assert_eq!(s.ajax.len(), 1);
        assert_eq!(s.sessions, 1);
    }
}

// moodle-ajax, entrée Shibboleth

#[tokio::test]
async fn ajax_shibboleth_traverse_wayf_idp_et_saml() {
    let (state, base) = start(|_| {}).await;
    let university = university_with(&base, ajax(&base, MoodleEntry::Shibboleth), Some(saml()));
    let mut session = session(true);

    let fetched = get_homework(&university, &mut session).await.unwrap();
    assert_eq!(fetched.data.len(), 2);
    let s = state.lock().unwrap();
    assert_eq!(s.sessions, 1);
    // Le WAYF reçoit l'entité de l'université et le bouton, une seule fois.
    assert_eq!(s.wayf_posts.len(), 1);
    assert!(s.wayf_posts[0].contains("user_idp=urn%3Atest%3Aidp"));
    assert!(s.wayf_posts[0].contains("Select="));
    assert_eq!(
        session.tokens.get(&slot(&base, "sesskey")),
        Some(&s.sesskey())
    );
    // Aucun mot de passe n'a transité : l'étudiant n'en a fourni aucun.
    assert!(session.cookies.iter().any(|c| c.name == "MoodleSession"));
}

#[tokio::test]
async fn ajax_shibboleth_sans_tgc_donne_session_expiree() {
    let (state, base) = start(|_| {}).await;
    let university = university_with(&base, ajax(&base, MoodleEntry::Shibboleth), Some(saml()));
    let problem = get_homework(&university, &mut session(false))
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::SessionExpired);
    assert_eq!(state.lock().unwrap().sessions, 0);
}

#[tokio::test]
async fn ajax_shibboleth_sans_configuration_saml() {
    let (_, base) = start(|_| {}).await;
    let university = university_with(&base, ajax(&base, MoodleEntry::Shibboleth), None);
    let problem = get_homework(&university, &mut session(true))
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::Internal);
}

#[tokio::test]
async fn ajax_shibboleth_ne_poste_jamais_hors_des_hotes_autorises() {
    let (state, base) = start(|s| s.wayf_foreign = true).await;
    let university = university_with(&base, ajax(&base, MoodleEntry::Shibboleth), Some(saml()));
    let problem = get_homework(&university, &mut session(true))
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::UpstreamChanged);
    assert!(state.lock().unwrap().wayf_posts.is_empty());
}

// moodle-webservice

#[tokio::test]
async fn webservice_utilise_le_jeton_de_la_session() {
    let (state, base) = start(|_| {}).await;
    let university = university_with(&base, webservice(&base, LOCAL), None);
    let mut session = session(false);
    let slot = format!("moodle.token.127.0.0.1:{}", base.port().unwrap());
    session.tokens.insert(slot, WS_TOKEN.to_owned());

    let fetched = get_homework(&university, &mut session).await.unwrap();
    assert_eq!(fetched.provider, "moodle-webservice");
    assert_eq!(titles(&fetched), ["Projet BUT"]);
    assert_eq!(
        fetched.data[0].due.to_rfc3339(),
        "2026-10-22T12:00:00+02:00"
    );

    let s = state.lock().unwrap();
    assert_eq!(s.ws.len(), 1);
    let (uri, body) = &s.ws[0];
    // Le jeton voyage dans le corps, jamais dans l'URL.
    assert!(!uri.contains(WS_TOKEN), "{uri}");
    assert!(body.contains("wsfunction=core_calendar_get_action_events_by_timesort"));
    assert!(!body.contains(UPCOMING_FUNCTION));
    assert!(body.contains("moodlewsrestformat=json"));
    assert!(body.contains("limitnum=50"));
}

#[tokio::test]
async fn connexion_au_service_puis_devoirs_avec_le_seul_jeton() {
    let (state, base) = start(|_| {}).await;
    let university = university_with(&base, webservice(&base, LOCAL), None);
    let mut session = session(false);
    {
        let mut ctx = Context {
            university: &university,
            session: &mut session,
            now: Utc::now(),
            net: NetSettings {
                https_only: false,
                user_agent: None,
            },
        };
        service_login(
            &mut ctx,
            ServiceKind::Homework,
            "camille.demo",
            "mot-de-passe-de-test",
        )
        .await
        .unwrap();
    }
    let slot = format!("moodle.token.127.0.0.1:{}", base.port().unwrap());
    assert_eq!(
        session.tokens.get(&slot).map(String::as_str),
        Some(WS_TOKEN)
    );
    // Ni le mot de passe ni le jeton privé ne sont gardés.
    let stored = format!("{:?}{:?}", session.tokens, session.cookies);
    assert!(!stored.contains("mot-de-passe") && !stored.contains("prive"));

    let fetched = get_homework(&university, &mut session).await.unwrap();
    assert_eq!(titles(&fetched), ["Projet BUT"]);
    assert_eq!(state.lock().unwrap().token_posts, 1);
}

#[tokio::test]
async fn connexion_au_service_refusee_sans_jeton() {
    let (state, base) = start(|_| {}).await;
    let university = university_with(&base, webservice(&base, LOCAL), None);
    let mut session = session(false);
    let mut ctx = Context {
        university: &university,
        session: &mut session,
        now: Utc::now(),
        net: NetSettings {
            https_only: false,
            user_agent: None,
        },
    };
    let problem = service_login(&mut ctx, ServiceKind::Homework, "camille.demo", "faux")
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::InvalidCredentials);
    assert!(session.tokens.is_empty());
    assert_eq!(state.lock().unwrap().token_posts, 1);
}

#[tokio::test]
async fn webservice_sans_jeton_demande_une_connexion_au_service() {
    let (state, base) = start(|_| {}).await;
    let university = university_with(&base, webservice(&base, LOCAL), None);
    let problem = get_homework(&university, &mut session(false))
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::ServiceLoginRequired);
    assert!(state.lock().unwrap().ws.is_empty());
}

#[tokio::test]
async fn webservice_jeton_refuse_est_oublie() {
    for reply in [Reply::TokenRefused, Reply::AccessException] {
        let (state, base) = start(|s| s.reply = reply).await;
        let university = university_with(&base, webservice(&base, LOCAL), None);
        let mut session = session(false);
        let slot = format!("moodle.token.127.0.0.1:{}", base.port().unwrap());
        session.tokens.insert(slot.clone(), WS_TOKEN.to_owned());

        let problem = get_homework(&university, &mut session).await.unwrap_err();
        assert_eq!(problem.code, ErrorCode::ServiceLoginRequired);
        assert!(!format!("{problem:?}").contains("SECRET"));
        assert!(!session.tokens.contains_key(&slot));

        // Sans jeton, plus aucun appel : pas de nouvel essai possible sans mot de passe.
        let again = get_homework(&university, &mut session).await.unwrap_err();
        assert_eq!(again.code, ErrorCode::ServiceLoginRequired);
        assert_eq!(state.lock().unwrap().ws.len(), 1);
    }
}

#[tokio::test]
async fn webservice_par_le_cas_n_a_pas_de_jeton() {
    let (state, base) = start(|_| {}).await;
    let university = university_with(
        &base,
        webservice(&base, SignIn::UniversityCas { service: None }),
        None,
    );
    let problem = get_homework(&university, &mut session(true))
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::NotImplemented);
    assert!(state.lock().unwrap().ws.is_empty());
}
