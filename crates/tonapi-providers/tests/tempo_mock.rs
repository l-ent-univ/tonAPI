//! Fournisseur `tempo` contre un faux TEMPO local et un faux CAS : `api/events` répond 401 en JSON
//! tant que `auth` n'a pas reçu de ticket. Aucune requête ne quitte cet ordinateur.

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Response, StatusCode, Uri, header};
use chrono::{NaiveDate, Utc};
use serde_json::json;
use tonapi_config::ResolvedUniversity;
use tonapi_core::ErrorCode;
use tonapi_providers::{Context, timetable, timetable_groups, timetable_of};
use tonapi_session::{SessionData, StoredCookie};

use common::{CAS_TICKET, Cas, has_cookie, query};

const EVENTS: &str = include_str!("fixtures/tempo/evenements.json");
const SESSION: &str = "PHPSESSID=tempo-ok";

// Faux TEMPO

#[derive(Default)]
struct Tempo {
    /// `auth` sans ticket répond 403 au lieu de renvoyer à l'accueil.
    auth_loop: bool,
    /// Réponse forcée de `api/events` pour un étudiant connecté.
    events: Option<(StatusCode, &'static str)>,
    /// Requêtes reçues : chemin et paramètres, dans l'ordre.
    seen: Vec<String>,
}

type Shared = Arc<Mutex<Tempo>>;

fn reply(status: StatusCode, content_type: &'static str, body: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body.to_owned()))
        .unwrap()
}

async fn tempo_handler(
    State(state): State<Shared>,
    uri: Uri,
    headers: HeaderMap,
) -> Response<Body> {
    let mut tempo = state.lock().unwrap();
    tempo.seen.push(uri.to_string());
    let signed_in = has_cookie(&headers, SESSION);
    match uri.path() {
        "/auth" if query(&uri, "ticket").as_deref() == Some(CAS_TICKET) => {
            let mut response = common::redirect("/auth");
            response.headers_mut().insert(
                header::SET_COOKIE,
                format!("{SESSION}; Path=/").parse().unwrap(),
            );
            response
        }
        "/auth" if tempo.auth_loop => reply(StatusCode::FORBIDDEN, "text/html", "Forbidden"),
        "/auth" => common::redirect("/"),
        "/api/events" if !signed_in => reply(
            StatusCode::UNAUTHORIZED,
            "application/json",
            r#"{"message":"Authentication required"}"#,
        ),
        "/api/events" => match tempo.events {
            Some((status, body)) => reply(status, "text/html", body),
            None => reply(StatusCode::OK, "application/json", EVENTS),
        },
        _ => reply(
            StatusCode::OK,
            "text/html",
            "<html>Tempo - Mon emploi du temps</html>",
        ),
    }
}

/// Faux TEMPO et faux CAS, et l'université qui les désigne.
struct Campus {
    tempo: Shared,
    cas: Cas,
    university: ResolvedUniversity,
}

async fn campus() -> Campus {
    let (cas_listener, cas_listener_base) = common::listen().await;
    let cas = Cas::default();
    common::spawn_cas(cas_listener, &cas);
    let tempo = Shared::default();
    let base = common::serve(
        Router::new()
            .fallback(tempo_handler)
            .with_state(tempo.clone()),
    )
    .await;
    let university = common::university(json!({
        "id": "test",
        "name": "Université de test",
        "sso": { "kind": "cas", "base-url": common::cas_base(&cas_listener_base) },
        "services": { "timetable": { "provider": "tempo", "base-url": format!("{base}/") } },
    }));
    Campus {
        tempo,
        cas,
        university,
    }
}

impl Campus {
    fn seen(&self) -> Vec<String> {
        self.tempo.lock().unwrap().seen.clone()
    }
}

fn session(cookies: Vec<StoredCookie>) -> SessionData {
    common::session("test", "e00000000", cookies)
}

fn signed_in() -> StoredCookie {
    common::stored_cookie("PHPSESSID", "tempo-ok", "/")
}

fn context<'a>(university: &'a ResolvedUniversity, session: &'a mut SessionData) -> Context<'a> {
    common::context(university, session, Utc::now())
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}

// Tests

#[tokio::test]
async fn sans_session_le_cas_ouvre_tempo_une_fois_puis_la_semaine_est_lue() {
    let campus = campus().await;
    let mut session = session(vec![common::tgc()]);

    let fetched = timetable(
        &mut context(&campus.university, &mut session),
        day(2),
        day(8),
    )
    .await
    .unwrap();
    assert_eq!(fetched.provider, "tempo");
    let read: Vec<_> = fetched
        .data
        .iter()
        .map(|course| (course.id.as_str(), course.start.to_rfc3339()))
        .collect();
    assert_eq!(
        read,
        [
            ("9909", "2026-11-02T10:45:00+01:00".to_owned()),
            ("9910", "2026-11-04T14:00:00+01:00".to_owned())
        ]
    );
    // 401, puis le CAS, `auth` avec le ticket, `auth` : la session est acquise sans charger
    // l'accueil, et la lecture est rejouée.
    let events = "/api/events?start=2026-11-02&end=2026-11-08";
    assert_eq!(
        campus.seen(),
        [
            events.to_owned(),
            format!("/auth?ticket={}", common::CAS_TICKET),
            "/auth".to_owned(),
            events.to_owned()
        ]
    );
    {
        let log = campus.cas.log.lock().unwrap();
        assert_eq!(log.services.len(), 1);
        assert!(log.services.first().unwrap().ends_with("/auth"));
    }
    assert!(session.cookies.iter().any(|c| c.name == "PHPSESSID"));

    // La session TEMPO est dans celle de l'étudiant : la lecture suivante ne repasse pas par le
    // CAS.
    let next = timetable(
        &mut context(&campus.university, &mut session),
        day(9),
        day(15),
    )
    .await
    .unwrap();
    assert_eq!(next.data.len(), 2);
    assert_eq!(campus.seen().len(), 5);
    assert_eq!(campus.cas.log.lock().unwrap().services.len(), 1);
}

#[tokio::test]
async fn session_cas_expiree() {
    let campus = campus().await;
    let mut session = session(Vec::new());

    let error = timetable(
        &mut context(&campus.university, &mut session),
        day(2),
        day(8),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::SessionExpired);
    // Une lecture, le formulaire du CAS, et rien d'autre : pas de nouvel essai.
    assert_eq!(campus.seen().len(), 1);
    let log = campus.cas.log.lock().unwrap();
    assert_eq!((log.forms, log.password_posts), (1, 0));
}

#[tokio::test]
async fn auth_rejoue_jusqu_au_403_la_session_est_ouverte_quand_meme() {
    let campus = campus().await;
    campus.tempo.lock().unwrap().auth_loop = true;
    let mut session = session(vec![common::tgc()]);

    let fetched = timetable(
        &mut context(&campus.university, &mut session),
        day(2),
        day(8),
    )
    .await
    .unwrap();
    assert_eq!(fetched.data.len(), 2);
}

#[tokio::test]
async fn erreur_de_tempo_sans_ouverture_de_session_inutile() {
    let campus = campus().await;
    campus.tempo.lock().unwrap().events = Some((StatusCode::INTERNAL_SERVER_ERROR, "{}"));
    let mut session = session(vec![signed_in(), common::tgc()]);

    let error = timetable(
        &mut context(&campus.university, &mut session),
        day(2),
        day(8),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::ServiceUnavailable);
    assert_eq!(campus.seen().len(), 1);
    assert!(campus.cas.log.lock().unwrap().services.is_empty());
}

#[tokio::test]
async fn objet_json_a_la_place_de_la_liste() {
    let campus = campus().await;
    campus.tempo.lock().unwrap().events = Some((StatusCode::OK, r#"{"message":"non"}"#));
    let mut session = session(vec![signed_in()]);

    let error = timetable(
        &mut context(&campus.university, &mut session),
        day(2),
        day(8),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamChanged);
    // La réponse ne passe jamais dans l'erreur.
    assert!(!error.detail.unwrap_or_default().contains("non"));
}

#[tokio::test]
async fn page_html_meme_apres_le_cas_une_erreur_pas_une_semaine_vide() {
    let campus = campus().await;
    campus.tempo.lock().unwrap().events = Some((StatusCode::OK, "<html>maintenance</html>"));
    let mut session = session(vec![signed_in(), common::tgc()]);

    let error = timetable(
        &mut context(&campus.university, &mut session),
        day(2),
        day(8),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamChanged);
    // Une ouverture de session, une seconde lecture, pas davantage.
    let events = campus
        .seen()
        .iter()
        .filter(|seen| seen.starts_with("/api/events"))
        .count();
    assert_eq!(events, 2);
    assert_eq!(campus.cas.log.lock().unwrap().services.len(), 1);
}

#[tokio::test]
async fn ni_groupes_ni_ressource_choisie() {
    let campus = campus().await;
    let mut session = session(vec![signed_in()]);

    let groups = timetable_groups(&mut context(&campus.university, &mut session))
        .await
        .unwrap_err();
    assert_eq!(groups.code, ErrorCode::NotImplemented);
    let chosen = timetable_of(
        &mut context(&campus.university, &mut session),
        day(2),
        day(8),
        &["1234".to_owned()],
    )
    .await
    .unwrap_err();
    assert_eq!(chosen.code, ErrorCode::NotImplemented);
    assert!(campus.seen().is_empty());
}
