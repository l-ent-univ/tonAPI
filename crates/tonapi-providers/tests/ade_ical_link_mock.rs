//! Fournisseur `ade-ical-link` de bout en bout contre un faux serveur local qui joue trois rôles :
//! le CAS (`/cas/login`), la page prose-etudiant qui porte le lien (`/prose/ical`) et le serveur
//! ADE qui sert le flux (`/jsp/custom/modules/plannings/direct_cal.jsp`). Aucune donnée réelle.

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Response, StatusCode, Uri, header};
use chrono::{NaiveDate, TimeZone, Utc};
use tonapi_config::ResolvedUniversity;
use tonapi_core::{Course, ErrorCode, Problem};
use tonapi_providers::{Context, NetSettings, timetable, timetable_groups};
use tonapi_session::SessionData;

use common::{query, redirect};

const SEMAINE: &str = include_str!("fixtures/ade_ical_link/semaine.ics");
const PAGE: &str = include_str!("fixtures/ade_ical_link/page-ical.html");
const FEED_PATH: &str = "/jsp/custom/modules/plannings/direct_cal.jsp";
const TOKEN_KEY: &str = "ade.ical";
/// `data=` que le faux serveur ADE accepte (valeur inventée, voir la fixture de la page).
fn fresh_data() -> String {
    format!("{},1", "0123456789abcdef".repeat(16))
}

#[derive(Clone, Copy, PartialEq)]
enum Feed {
    /// Calendrier pour le bon `data=`, page HTML pour tout autre.
    Normal,
    /// Toujours une page HTML (générateur désactivé).
    Html,
    /// Erreur 503.
    Down,
}

struct Mock {
    base: String,
    feed: Feed,
    page_hits: u32,
    feed_hits: u32,
    /// Prose récent : la page n'a pas le lien, il arrive après une navigation Vaadin.
    vaadin: bool,
    /// Emplacements des navigations Vaadin reçues.
    navigations: Vec<String>,
}

const VAADIN_KEY: &str = "cle-vaadin-de-test";

type Shared = Arc<Mutex<Mock>>;

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn reply(
    status: StatusCode,
    content_type: &str,
    cookie: Option<&str>,
    body: String,
) -> Response<Body> {
    let mut r = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type);
    if let Some(cookie) = cookie {
        r = r.header(header::SET_COOKIE, cookie);
    }
    r.body(Body::from(body)).unwrap()
}

const LOGIN_PAGE: &str = r#"<html><body><form method="post" id="fm1" action="login">
    <input name="username"><input type="password" name="password">
    <input type="hidden" name="execution" value="E1"><input type="hidden" name="_eventId" value="submit">
    </form></body></html>"#;

async fn handle(
    State(state): State<Shared>,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response<Body> {
    let mut s = state.lock().unwrap();
    match uri.path() {
        "/prose/" if query(&uri, "v-r").as_deref() == Some("uidl") => {
            let request: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            if request["csrfToken"] != VAADIN_KEY || query(&uri, "v-uiId").as_deref() != Some("1") {
                return reply(StatusCode::FORBIDDEN, "text/plain", None, String::new());
            }
            let location = request["rpc"][0]["location"]
                .as_str()
                .unwrap_or("")
                .to_owned();
            // Le lien n'est envoyé qu'en revenant sur la vue depuis une autre.
            let back = location == "ical" && s.navigations.iter().any(|l| l != "ical");
            s.navigations.push(location);
            let sync = s.navigations.len();
            let changes = if back {
                format!(
                    r#"[{{"node":3,"type":"put","key":"text","feat":7,"value":"{}{FEED_PATH}?data={}"}}]"#,
                    s.base,
                    fresh_data()
                )
            } else {
                "[]".to_owned()
            };
            reply(
                StatusCode::OK,
                "application/json; charset=UTF-8",
                None,
                format!(r#"for(;;);[{{"syncId":{sync},"clientId":{sync},"changes":{changes}}}]"#),
            )
        }
        "/cas/login" => {
            let has_tgc = headers
                .get(header::COOKIE)
                .and_then(|c| c.to_str().ok())
                .is_some_and(|c| c.split("; ").any(|p| p == "TGC=TGT-demo"));
            match (has_tgc, query(&uri, "service")) {
                (true, Some(service)) => redirect(&format!("{service}?ticket=ST-1")),
                _ => reply(StatusCode::OK, "text/html", None, LOGIN_PAGE.to_owned()),
            }
        }
        "/prose/ical" => {
            if query(&uri, "ticket").is_none() {
                let service = format!("{}/prose/ical", s.base);
                return redirect(&format!("{}/cas/login?service={}", s.base, enc(&service)));
            }
            s.page_hits += 1;
            let page = if s.vaadin {
                format!(
                    r#"<html><head><title>IcalView</title></head><body><script>var uidl = {{"syncId":0,"clientId":0,"changes":[],"Vaadin-Security-Key":"{VAADIN_KEY}"}}; var config = {{"v-uiId":1}}; window.Vaadin.Flow.initApplication("prose", config);</script></body></html>"#
                )
            } else {
                PAGE.replace("https://ade.example.test", &s.base)
            };
            reply(
                StatusCode::OK,
                "text/html;charset=utf-8",
                Some("PROSE=1; Path=/"),
                page,
            )
        }
        FEED_PATH => {
            s.feed_hits += 1;
            let cookie = Some("JSESSIONID=ade-demo; Path=/jsp; HttpOnly");
            let fresh = query(&uri, "data").is_some_and(|d| d == fresh_data());
            match s.feed {
                Feed::Down => reply(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "text/html",
                    None,
                    String::new(),
                ),
                Feed::Normal if fresh => reply(
                    StatusCode::OK,
                    "text/calendar;charset=UTF-8",
                    cookie,
                    SEMAINE.to_owned(),
                ),
                _ => reply(
                    StatusCode::OK,
                    "text/html",
                    cookie,
                    "<!DOCTYPE html><html></html>".to_owned(),
                ),
            }
        }
        _ => reply(StatusCode::NOT_FOUND, "text/plain", None, String::new()),
    }
}

async fn start(feed: Feed) -> (Shared, String) {
    start_with(feed, false).await
}

async fn start_with(feed: Feed, vaadin: bool) -> (Shared, String) {
    let (listener, base) = common::listen().await;
    let state: Shared = Arc::new(Mutex::new(Mock {
        base: base.clone(),
        feed,
        page_hits: 0,
        feed_hits: 0,
        vaadin,
        navigations: Vec::new(),
    }));
    let app = Router::new().fallback(handle).with_state(state.clone());
    common::spawn(listener, app);
    (state, base)
}

/// Université de test : CAS et lien ADE sur le faux serveur. `feed_host` remplace l'hôte du flux.
fn university(base: &str, feed_host: Option<&str>) -> ResolvedUniversity {
    let host = base.trim_start_matches("http://");
    common::university(serde_json::json!({
        "id": "test-ade",
        "name": "Université de test",
        "sso": { "kind": "cas", "base-url": format!("{base}/cas/") },
        "services": { "timetable": {
            "provider": "ade-ical-link",
            "link-page": format!("{base}/prose/ical"),
            "feed-host": feed_host.unwrap_or(host),
        } },
    }))
}

/// Session d'un étudiant fictif ; `tgc` : le CAS le reconnaît encore.
fn session(tgc: bool) -> SessionData {
    let cookies = if tgc {
        vec![common::stored_cookie("TGC", "TGT-demo", "/")]
    } else {
        Vec::new()
    };
    common::session("test-ade", "e00000000000", cookies)
}

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

async fn week(
    university: &ResolvedUniversity,
    session: &mut SessionData,
) -> Result<Vec<Course>, Problem> {
    let mut ctx = Context {
        university,
        session,
        now: Utc.with_ymd_and_hms(2026, 10, 6, 9, 0, 0).unwrap(),
        net: NetSettings { https_only: false },
    };
    let fetched = timetable(&mut ctx, day(10, 5), day(10, 11)).await?;
    assert_eq!(fetched.provider, "ade-ical-link");
    Ok(fetched.data)
}

fn hits(state: &Shared) -> (u32, u32) {
    let s = state.lock().unwrap();
    (s.page_hits, s.feed_hits)
}

#[tokio::test]
async fn une_ressource_choisie_n_est_pas_prise_en_charge() {
    let (state, base) = start(Feed::Normal).await;
    let university = university(&base, None);
    let mut session = session(true);
    let mut ctx = Context {
        university: &university,
        session: &mut session,
        now: Utc.with_ymd_and_hms(2026, 10, 6, 9, 0, 0).unwrap(),
        net: NetSettings { https_only: false },
    };
    let asked = ["14961".to_owned()];
    let problem = tonapi_providers::timetable_of(&mut ctx, day(10, 5), day(10, 11), &asked)
        .await
        .unwrap_err();
    // Jamais l'emploi du temps de l'étudiant à la place de celui demandé, et aucune requête.
    assert_eq!(problem.code, ErrorCode::NotImplemented);
    assert_eq!(hits(&state), (0, 0));
}

#[tokio::test]
async fn lit_le_lien_sur_la_page_puis_le_garde() {
    let (state, base) = start(Feed::Normal).await;
    let uni = university(&base, None);
    let mut session = session(true);

    let courses = week(&uni, &mut session).await.unwrap();
    assert_eq!(courses.len(), 4);
    assert_eq!(courses[0].title, "Programmation web");
    assert_eq!(courses[0].start.to_rfc3339(), "2026-10-06T08:00:00+02:00");
    assert_eq!(hits(&state), (1, 1));

    // Le lien est gardé dans la session ; ni le TGC ni le cookie de la page ne se perdent, et le
    // `JSESSIONID` du serveur ADE n'y entre pas.
    assert_eq!(
        session.tokens.get(TOKEN_KEY).map(String::as_str),
        Some(format!("{base}{FEED_PATH}?data={}", fresh_data()).as_str())
    );
    let names: Vec<&str> = session.cookies.iter().map(|c| c.name.as_str()).collect();
    assert!(
        names.contains(&"TGC") && names.contains(&"PROSE"),
        "{names:?}"
    );
    assert!(!names.contains(&"JSESSIONID"), "{names:?}");

    // Deuxième appel : le lien suffit, la page n'est pas rouverte.
    let again = week(&uni, &mut session).await.unwrap();
    assert_eq!(again, courses);
    assert_eq!(hits(&state), (1, 2));
}

#[tokio::test]
async fn lien_envoye_par_une_navigation_vaadin() {
    let (state, base) = start_with(Feed::Normal, true).await;
    let uni = university(&base, None);
    let mut session = session(true);

    let courses = week(&uni, &mut session).await.unwrap();
    assert_eq!(courses.len(), 4);
    assert_eq!(hits(&state), (1, 1));
    // Aller-retour par une autre vue, avec la clé de la page.
    assert_eq!(
        state.lock().unwrap().navigations,
        ["protected/resume", "ical"]
    );
    assert!(session.tokens.contains_key(TOKEN_KEY));
}

#[tokio::test]
async fn lien_perime_relu_une_seule_fois() {
    let (state, base) = start(Feed::Normal).await;
    let uni = university(&base, None);
    let mut session = session(true);
    session.tokens.insert(
        TOKEN_KEY.to_owned(),
        format!("{base}{FEED_PATH}?data=deadbeef,1"),
    );

    let courses = week(&uni, &mut session).await.unwrap();
    assert_eq!(courses.len(), 4);
    // Ancien lien refusé (HTML), page relue une fois, nouveau lien accepté.
    assert_eq!(hits(&state), (1, 2));
    assert!(session.tokens[TOKEN_KEY].ends_with(&fresh_data()));
}

#[tokio::test]
async fn generateur_desactive_donne_upstream_changed_sans_fuite() {
    let (state, base) = start(Feed::Html).await;
    let uni = university(&base, None);
    let mut session = session(true);
    session.tokens.insert(
        TOKEN_KEY.to_owned(),
        format!("{base}{FEED_PATH}?data=deadbeef,1"),
    );

    let err = week(&uni, &mut session).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::UpstreamChanged);
    // Un seul nouvel essai : lien gardé, page, lien lu → 2 téléchargements, 1 lecture de la page.
    assert_eq!(hits(&state), (1, 2));
    let shown = format!("{err:?} {}", serde_json::to_string(&err).unwrap());
    assert!(!shown.contains("0123456789abcdef"), "{shown}");
    assert!(!shown.contains("deadbeef"), "{shown}");
    assert!(!shown.contains("ticket"), "{shown}");
}

#[tokio::test]
async fn cas_qui_redemande_le_mot_de_passe_donne_session_expired() {
    let (state, base) = start(Feed::Normal).await;
    let uni = university(&base, None);
    let mut session = session(false);

    let err = week(&uni, &mut session).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::SessionExpired);
    assert_eq!(hits(&state), (0, 0));
    assert!(!session.tokens.contains_key(TOKEN_KEY));
}

#[tokio::test]
async fn serveur_ade_en_panne_donne_service_unavailable() {
    let (_state, base) = start(Feed::Down).await;
    let uni = university(&base, None);
    let mut session = session(true);
    session.tokens.insert(
        TOKEN_KEY.to_owned(),
        format!("{base}{FEED_PATH}?data={}", fresh_data()),
    );

    let err = week(&uni, &mut session).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ServiceUnavailable);
    // Le lien gardé reste en place : une panne n'est pas un lien périmé.
    assert!(session.tokens.contains_key(TOKEN_KEY));
}

#[tokio::test]
async fn le_flux_n_est_jamais_demande_a_un_autre_hote() {
    let (state, base) = start(Feed::Normal).await;
    // La page annonce un flux sur le faux serveur ; la configuration attend un autre hôte.
    let uni = university(&base, Some("127.0.0.1:1"));
    let mut session = session(true);

    let err = week(&uni, &mut session).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::UpstreamChanged);
    assert_eq!(hits(&state), (1, 0));
    assert!(!session.tokens.contains_key(TOKEN_KEY));

    // Un lien déjà gardé qui vise un hôte non configuré est ignoré, pas suivi.
    session.tokens.insert(
        TOKEN_KEY.to_owned(),
        format!("{base}{FEED_PATH}?data={}", fresh_data()),
    );
    let _ = week(&uni, &mut session).await;
    assert_eq!(hits(&state).1, 0);
}

#[tokio::test]
async fn groupes_non_listables_avec_ce_fournisseur() {
    let (state, base) = start(Feed::Normal).await;
    let uni = university(&base, None);
    let mut session = session(true);
    let mut ctx = Context {
        university: &uni,
        session: &mut session,
        now: Utc::now(),
        net: NetSettings { https_only: false },
    };
    let err = timetable_groups(&mut ctx).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotImplemented);
    assert!(err.detail.unwrap().contains("groupes"));
    assert_eq!(hits(&state), (0, 0));
}
