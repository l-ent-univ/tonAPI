//! Fournisseur `zimbra-rest` contre un faux webmail local : page d'entrée avec préauthentification
//! derrière un faux CAS, puis SOAP en JSON. Aucun test ne contacte un vrai service.

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Response, StatusCode, Uri, header};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use tonapi_config::ResolvedUniversity;
use tonapi_core::{ErrorCode, MailInbox};
use tonapi_providers::{Context, NetSettings, mail_inbox};
use tonapi_session::SessionData;

use common::query;

const SEARCH: &str = include_str!("fixtures/zimbra/search_response.json");
const FOLDER: &str = include_str!("fixtures/zimbra/folder_response.json");
const FAULT_AUTH: &str = include_str!("fixtures/zimbra/fault_session_expiree.json");
const FAULT_AUTRE: &str = include_str!("fixtures/zimbra/fault_autre.json");

/// Ce que fait le faux CAS quand le webmail y envoie l'étudiant.
#[derive(Clone, Copy, PartialEq)]
enum Cas {
    /// Le TGC est valable : retour immédiat avec un ticket.
    SignedIn,
    /// Plus de session CAS : formulaire de mot de passe.
    LoginForm,
    /// Second facteur redemandé.
    SecondFactor,
}

/// Ce que renvoie le faux webmail après le ticket.
#[derive(Clone, Copy, PartialEq)]
enum Entry {
    /// Préauthentification : cookie `ZM_AUTH_TOKEN` posé.
    Preauth,
    /// Page d'une fédération SAML, sans cookie.
    Saml,
}

struct Mock {
    cas: Cas,
    entry: Entry,
    /// Cookie accepté par le SOAP ; `None` : aucun cookie n'est accepté.
    valid_token: Option<String>,
    /// Le cookie émis par la préauthentification devient-il valable ?
    accept_fresh_token: bool,
    /// Faute forcée sur `SearchRequest` (corps de la réponse HTTP 500).
    search_fault: Option<&'static str>,
    /// Statut forcé sur `SearchRequest`, avec un corps HTML.
    search_status: Option<u16>,
    folder_fails: bool,
    signins: u32,
    searches: u32,
    folders: u32,
    search_cookies: Vec<String>,
    last_search: Option<Value>,
}

impl Mock {
    fn new(valid_token: Option<&str>) -> Self {
        Mock {
            cas: Cas::SignedIn,
            entry: Entry::Preauth,
            valid_token: valid_token.map(str::to_owned),
            accept_fresh_token: true,
            search_fault: None,
            search_status: None,
            folder_fails: false,
            signins: 0,
            searches: 0,
            folders: 0,
            search_cookies: Vec::new(),
            last_search: None,
        }
    }
}

type Shared = Arc<Mutex<Mock>>;

const FRESH_TOKEN: &str = "jeton-neuf-synthetique";
const STALE_TOKEN: &str = "jeton-perime-synthetique";

fn response(status: u16, content_type: &str, body: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body.to_owned()))
        .unwrap()
}

fn redirect(location: &str, cookie: Option<&str>) -> Response<Body> {
    let mut r = Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location);
    if let Some(cookie) = cookie {
        r = r.header(header::SET_COOKIE, cookie);
    }
    r.body(Body::empty()).unwrap()
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split("; ")
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")).map(str::to_owned))
}

const LOGIN_PAGE: &str = r#"<html><body><form method="post" id="fm1" action="login">
    <input id="username" name="username"><input type="password" name="password">
    <input type="hidden" name="execution" value="E1"><input type="hidden" name="_eventId" value="submit">
    </form></body></html>"#;

fn otp_page(base: &str) -> String {
    format!(
        r#"<html><body><form method="post" id="fm1">
        <input id="ticket" name="ticket" value="TK" type="hidden"><input name="token"></form>
        <script>start({{"apiUrl":"{base}/otp/","uid":"e1","userHash":"H"}})</script></body></html>"#
    )
}

const SAML_PAGE: &str = r#"<html><body onload="document.forms[0].submit()">
    <form method="post" action="https://sp.example.org/saml/acs">
    <input type="hidden" name="SAMLResponse" value="UEhBU0U="><input type="hidden" name="RelayState" value="r">
    </form></body></html>"#;

async fn handle(
    State((state, base)): State<(Shared, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response<Body> {
    let mut s = state.lock().unwrap();
    match (method.as_str(), uri.path()) {
        // Faux CAS.
        ("GET", "/cas/login") => match s.cas {
            Cas::SignedIn => {
                let service = query(&uri, "service").unwrap_or_default();
                redirect(&format!("{service}?ticket=ST-1"), None)
            }
            Cas::LoginForm => response(200, "text/html", LOGIN_PAGE),
            Cas::SecondFactor => response(200, "text/html", &otp_page(&base)),
        },
        // Page d'entrée du webmail.
        ("GET", "/preauth") => {
            if query(&uri, "ticket").is_none() {
                return redirect(&format!("{base}/cas/login?service={base}/preauth"), None);
            }
            if s.entry == Entry::Saml {
                return response(200, "text/html", SAML_PAGE);
            }
            s.signins += 1;
            if s.accept_fresh_token {
                s.valid_token = Some(FRESH_TOKEN.to_owned());
            }
            redirect(
                "/mail",
                Some(&format!("ZM_AUTH_TOKEN={FRESH_TOKEN}; Path=/")),
            )
        }
        ("GET", "/mail") => response(200, "text/html", "<html><body>Zimbra</body></html>"),
        // SOAP.
        ("POST", "/service/soap/SearchRequest") => {
            s.searches += 1;
            s.search_cookies
                .push(cookie(&headers, "ZM_AUTH_TOKEN").unwrap_or_default());
            s.last_search = serde_json::from_str(&body).ok();
            if let Some(status) = s.search_status {
                return response(status, "text/html", "<html>indisponible</html>");
            }
            let accepted =
                s.valid_token.is_some() && cookie(&headers, "ZM_AUTH_TOKEN") == s.valid_token;
            if !accepted {
                return response(500, "application/json", FAULT_AUTH);
            }
            match s.search_fault {
                Some(fault) => response(500, "application/json", fault),
                None => response(200, "application/json", SEARCH),
            }
        }
        ("POST", "/service/soap/GetFolderRequest") => {
            s.folders += 1;
            if s.folder_fails {
                return response(500, "application/json", FAULT_AUTRE);
            }
            response(200, "application/json", FOLDER)
        }
        _ => response(404, "text/plain", "introuvable"),
    }
}

/// Démarre le faux webmail ; renvoie son adresse (`http://127.0.0.1:port`).
async fn serve(state: &Shared) -> String {
    let (listener, base) = common::listen().await;
    let app = Router::new()
        .fallback(handle)
        .with_state((Arc::clone(state), base.clone()));
    common::spawn(listener, app);
    base
}

fn university(base: &str) -> ResolvedUniversity {
    common::university(json!({
        "id": "demo-zimbra",
        "name": "Université Démo Zimbra",
        "timezone": "Europe/Paris",
        "sso": { "kind": "cas", "base-url": format!("{base}/cas/") },
        "services": { "mail": {
            "provider": "zimbra-rest",
            "base-url": format!("{base}/"),
            "entry-url": format!("{base}/preauth"),
        } },
    }))
}

/// Session d'étudiant : TGC du faux CAS et, si demandé, un cookie Zimbra déjà obtenu.
fn session(zimbra_token: Option<&str>) -> SessionData {
    let mut cookies = vec![common::stored_cookie("TGC", "tgc-synthetique", "/cas/")];
    if let Some(token) = zimbra_token {
        cookies.push(common::stored_cookie("ZM_AUTH_TOKEN", token, "/"));
    }
    SessionData {
        expires_at: 0,
        ..common::session("demo-zimbra", "camille.demo", cookies)
    }
}

async fn run(
    state: &Shared,
    session: &mut SessionData,
) -> Result<tonapi_providers::Fetched<MailInbox>, tonapi_core::Problem> {
    let base = serve(state).await;
    let university = university(&base);
    let mut ctx = Context {
        university: &university,
        session,
        now: Utc.with_ymd_and_hms(2026, 10, 27, 10, 0, 0).unwrap(),
        net: NetSettings { https_only: false },
    };
    mail_inbox(&mut ctx).await
}

fn token_of(session: &SessionData) -> Option<&str> {
    session
        .cookies
        .iter()
        .find(|c| c.name == "ZM_AUTH_TOKEN")
        .map(|c| c.value.as_str())
}

fn shared(mock: Mock) -> Shared {
    Arc::new(Mutex::new(mock))
}

#[tokio::test]
async fn cookie_valide_lit_la_boite_sans_reconnexion() {
    let state = shared(Mock::new(Some(STALE_TOKEN)));
    let mut session = session(Some(STALE_TOKEN));
    let fetched = run(&state, &mut session).await.unwrap();

    assert_eq!(fetched.provider, "zimbra-rest");
    let inbox = fetched.data;
    assert_eq!(inbox.messages.len(), 4);
    assert_eq!(inbox.unread, 2);
    assert_eq!(inbox.messages[0].subject, "Réunion de rentrée");
    assert_eq!(
        inbox.messages[0].date.to_rfc3339(),
        "2026-10-26T08:15:00+01:00"
    );
    assert!(inbox.webmail_url.unwrap().starts_with("http://127.0.0.1:"));

    let s = state.lock().unwrap();
    assert_eq!(s.signins, 0, "le cookie de la session suffit");
    assert_eq!(s.searches, 1);
    assert_eq!(s.search_cookies, [STALE_TOKEN]);
    // La requête est bien celle de la boîte de réception, triée par date, limitée.
    let request = s.last_search.as_ref().unwrap();
    assert_eq!(request["Body"]["SearchRequest"]["query"], "in:inbox");
    assert_eq!(request["Body"]["SearchRequest"]["limit"], 20);
}

#[tokio::test]
async fn sans_cookie_zimbra_la_connexion_passe_par_le_cas() {
    let state = shared(Mock::new(None));
    let mut session = session(None);
    let inbox = run(&state, &mut session).await.unwrap().data;

    assert_eq!(inbox.messages.len(), 4);
    let s = state.lock().unwrap();
    assert_eq!(s.signins, 1);
    assert_eq!(s.searches, 1);
    assert_eq!(s.search_cookies, [FRESH_TOKEN]);
    // Le cookie obtenu repart dans la session, avec le TGC.
    assert_eq!(token_of(&session), Some(FRESH_TOKEN));
    assert!(session.cookies.iter().any(|c| c.name == "TGC"));
}

#[tokio::test]
async fn cookie_perime_un_seul_nouvel_essai() {
    // La session porte un cookie que le webmail n'accepte plus.
    let state = shared(Mock::new(Some("autre-jeton")));
    let mut session = session(Some(STALE_TOKEN));
    let inbox = run(&state, &mut session).await.unwrap().data;

    assert_eq!(inbox.messages.len(), 4);
    let s = state.lock().unwrap();
    assert_eq!(s.signins, 1);
    assert_eq!(s.searches, 2, "un refus, puis la lecture rejouée");
    assert_eq!(s.search_cookies, [STALE_TOKEN, FRESH_TOKEN]);
    assert_eq!(token_of(&session), Some(FRESH_TOKEN));
}

#[tokio::test]
async fn cookie_encore_refuse_apres_reconnexion_ne_boucle_pas() {
    let mut mock = Mock::new(None);
    mock.accept_fresh_token = false;
    let state = shared(mock);
    let mut session = session(Some(STALE_TOKEN));
    let error = run(&state, &mut session).await.unwrap_err();

    assert_eq!(error.code, ErrorCode::UpstreamChanged);
    let s = state.lock().unwrap();
    assert_eq!(s.signins, 1);
    assert_eq!(s.searches, 2);
    // Aucun jeton dans le message d'erreur.
    let detail = error.detail.unwrap_or_default();
    assert!(!detail.contains(FRESH_TOKEN) && !detail.contains(STALE_TOKEN));
}

#[tokio::test]
async fn jeton_refuse_juste_apres_la_premiere_connexion() {
    let mut mock = Mock::new(None);
    mock.accept_fresh_token = false;
    let state = shared(mock);
    let mut session = session(None);
    let error = run(&state, &mut session).await.unwrap_err();

    assert_eq!(error.code, ErrorCode::UpstreamChanged);
    let s = state.lock().unwrap();
    assert_eq!(s.signins, 1);
    assert_eq!(
        s.searches, 1,
        "pas de seconde connexion juste après la première"
    );
}

#[tokio::test]
async fn cas_redemande_le_mot_de_passe() {
    let mut mock = Mock::new(None);
    mock.cas = Cas::LoginForm;
    let state = shared(mock);
    let mut session = session(None);
    let error = run(&state, &mut session).await.unwrap_err();

    assert_eq!(error.code, ErrorCode::SessionExpired);
    let s = state.lock().unwrap();
    assert_eq!((s.signins, s.searches), (0, 0));
}

#[tokio::test]
async fn cas_redemande_le_second_facteur() {
    let mut mock = Mock::new(Some("autre-jeton"));
    mock.cas = Cas::SecondFactor;
    let state = shared(mock);
    // Cookie Zimbra périmé : la reconnexion tombe sur le second facteur.
    let mut session = session(Some(STALE_TOKEN));
    let error = run(&state, &mut session).await.unwrap_err();

    assert_eq!(error.code, ErrorCode::SessionExpired);
    assert!(error.detail.unwrap_or_default().contains("second facteur"));
    let s = state.lock().unwrap();
    assert_eq!(s.searches, 1, "pas de nouvelle lecture sans session");
}

#[tokio::test]
async fn federation_saml_pas_encore_prise_en_charge() {
    let mut mock = Mock::new(None);
    mock.entry = Entry::Saml;
    let state = shared(mock);
    let mut session = session(None);
    let error = run(&state, &mut session).await.unwrap_err();

    assert_eq!(error.code, ErrorCode::NotImplemented);
    assert!(error.detail.unwrap_or_default().contains("SAML"));
}

#[tokio::test]
async fn autre_faute_zimbra_n_est_pas_rejouee() {
    let mut mock = Mock::new(Some(STALE_TOKEN));
    mock.search_fault = Some(FAULT_AUTRE);
    let state = shared(mock);
    let mut session = session(Some(STALE_TOKEN));
    let error = run(&state, &mut session).await.unwrap_err();

    assert_eq!(error.code, ErrorCode::UpstreamChanged);
    assert!(error.detail.unwrap_or_default().contains("service.FAILURE"));
    let s = state.lock().unwrap();
    assert_eq!((s.signins, s.searches), (0, 1));
}

#[tokio::test]
async fn serveur_indisponible() {
    let mut mock = Mock::new(Some(STALE_TOKEN));
    mock.search_status = Some(503);
    let state = shared(mock);
    let mut session = session(Some(STALE_TOKEN));
    let error = run(&state, &mut session).await.unwrap_err();

    assert_eq!(error.code, ErrorCode::ServiceUnavailable);
    let detail = error.detail.unwrap_or_default();
    assert!(
        !detail.contains("indisponible"),
        "le corps reçu n'est pas repris"
    );
    assert_eq!(state.lock().unwrap().signins, 0);
}

#[tokio::test]
async fn dossier_en_echec_les_non_lus_viennent_des_messages() {
    let mut mock = Mock::new(Some(STALE_TOKEN));
    mock.folder_fails = true;
    let state = shared(mock);
    let mut session = session(Some(STALE_TOKEN));
    let inbox = run(&state, &mut session).await.unwrap().data;

    assert_eq!(inbox.messages.len(), 4);
    assert_eq!(inbox.unread, 2);
    assert_eq!(state.lock().unwrap().folders, 1);
}
