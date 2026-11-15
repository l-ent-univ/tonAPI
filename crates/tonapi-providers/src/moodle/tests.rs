//! Tests de l'analyse (fixtures synthétiques de `tests/fixtures/moodle/`) et de `sign_in` contre un
//! faux `login/token.php` local. Le parcours complet est dans `tests/moodle_mock.rs`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::routing::post;
use chrono_tz::Europe::Paris;
use tonapi_config::{
    CasFlavor, CasSso, Credentials, ResolvedUniversity, SamlConfig, ServiceOptions, Services,
    Status, UniversityFile,
};
use tonapi_session::SessionData;

use super::parse::*;
use super::saml::*;
use super::*;
use crate::NetSettings;

const EVENTS: &str = include_str!("../../tests/fixtures/moodle/events.json");
const AJAX_EVENTS: &str = include_str!("../../tests/fixtures/moodle/ajax_events.json");
const WS_EVENTS: &str = include_str!("../../tests/fixtures/moodle/ws_events.json");
const MISSING_FIELDS: &str = include_str!("../../tests/fixtures/moodle/events_missing_fields.json");
const MISSING_ID: &str = include_str!("../../tests/fixtures/moodle/events_missing_id.json");
const MISSING_TITLE: &str = include_str!("../../tests/fixtures/moodle/events_missing_title.json");
const INVALID_SESSKEY: &str = include_str!("../../tests/fixtures/moodle/ajax_invalidsesskey.json");
const SERVICE_NOT_AVAILABLE: &str =
    include_str!("../../tests/fixtures/moodle/ajax_servicenotavailable.json");
const WHOLE_REFUSED: &str =
    include_str!("../../tests/fixtures/moodle/ajax_whole_request_refused.json");
const INVALID_TOKEN: &str = include_str!("../../tests/fixtures/moodle/ws_invalidtoken.json");
const ACCESS_EXCEPTION: &str = include_str!("../../tests/fixtures/moodle/ws_accessexception.json");
const TOKEN_OK: &str = include_str!("../../tests/fixtures/moodle/token_ok.json");
const TOKEN_INVALID_LOGIN: &str =
    include_str!("../../tests/fixtures/moodle/token_invalidlogin.json");
const MY_PAGE: &str = include_str!("../../tests/fixtures/moodle/my_page.html");
const MY_PAGE_GUEST: &str = include_str!("../../tests/fixtures/moodle/my_page_guest.html");
const MY_PAGE_WITHOUT_SESSKEY: &str =
    include_str!("../../tests/fixtures/moodle/my_page_without_sesskey.html");

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn events_of(text: &str) -> Vec<Value> {
    json(text)["events"].as_array().unwrap().clone()
}

fn detail(problem: &Problem) -> String {
    problem.detail.clone().unwrap_or_default()
}

// événements → devoirs

#[test]
fn les_evenements_deviennent_des_devoirs_tries_par_echeance() {
    let homework = parse_homework(&events_of(EVENTS), Paris, AJAX).unwrap();
    // 1002 est le même devoir que 1001 dans un autre espace de cours ; 1005 n'a pas d'échéance.
    let ids: Vec<&str> = homework.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(ids, ["1001", "1003", "1004"]);

    let tp = &homework[0];
    assert_eq!(tp.title, "Rendu TP1");
    assert_eq!(tp.course.as_deref(), Some("Algorithmique & Données"));
    assert_eq!(tp.due.to_rfc3339(), "2026-10-20T23:59:00+02:00");
    assert_eq!(
        tp.description.as_deref(),
        Some("Déposer l'archive tp1.zip. Voir « consignes » & barème.")
    );
    assert_eq!(
        tp.url.as_deref(),
        Some("https://moodle.exemple.test/mod/assign/view.php?id=501")
    );
    assert_eq!(tp.submitted, Some(false));

    // Identifiant et date en chaîne, titre nettoyé, cours désigné par son nom court, adresse de
    // l'action faute d'adresse de l'événement.
    let quiz = &homework[1];
    assert_eq!(quiz.title, "Quiz de synthèse");
    assert_eq!(quiz.course.as_deref(), Some("RESEAU"));
    assert_eq!(quiz.due.to_rfc3339(), "2026-10-30T23:59:00+01:00");
    assert_eq!(quiz.description, None);
    assert_eq!(
        quiz.url.as_deref(),
        Some("https://moodle.exemple.test/mod/quiz/view.php?id=601")
    );
    assert_eq!(quiz.submitted, Some(false));

    // Un forum n'a pas de notion de rendu ; une adresse qui n'est pas http(s) est écartée.
    let forum = &homework[2];
    assert_eq!(forum.title, "Discussion ouverte");
    assert_eq!(forum.course, None);
    assert_eq!(forum.url, None);
    assert_eq!(forum.submitted, None);
}

#[test]
fn les_donnees_ajax_et_webservice_donnent_les_memes_devoirs() {
    let mut data = ajax_data(json(AJAX_EVENTS)).unwrap();
    let ajax = parse_homework(&events_of(&data.to_string()), Paris, AJAX).unwrap();
    assert_eq!(ajax.len(), 2);
    assert!(data.get_mut("events").is_some());

    let ws = webservice_data(json(WS_EVENTS)).unwrap();
    let homework = parse_homework(&events_of(&ws.to_string()), Paris, WEBSERVICE).unwrap();
    assert_eq!(homework.len(), 1);
    assert_eq!(homework[0].title, "Projet BUT");
    assert_eq!(homework[0].due.to_rfc3339(), "2026-10-22T12:00:00+02:00");
}

#[test]
fn echeance_dans_le_fuseau_de_l_universite_au_changement_d_heure() {
    // Dimanche 25 octobre 2026 : à 01:00 UTC, Paris repasse de +02:00 à +01:00.
    let event = |id: u32, timesort: i64| json!({ "id": id, "name": "Rendu", "timesort": timesort });
    let events = [event(1, 1_792_888_200), event(2, 1_792_891_800)];
    let homework = parse_homework(&events, Paris, AJAX).unwrap();
    assert_eq!(homework[0].due.to_rfc3339(), "2026-10-25T02:30:00+02:00");
    assert_eq!(homework[1].due.to_rfc3339(), "2026-10-25T02:30:00+01:00");
    // Le même instant dans un autre fuseau garde son instant.
    let reunion = parse_homework(&events[..1], chrono_tz::Indian::Reunion, AJAX).unwrap();
    assert_eq!(reunion[0].due.to_rfc3339(), "2026-10-25T04:30:00+04:00");
    assert_eq!(reunion[0].due, homework[0].due);
}

#[test]
fn champs_obligatoires_manquants_donnent_upstream_changed() {
    for fixture in [MISSING_FIELDS, MISSING_ID, MISSING_TITLE] {
        let problem = parse_homework(&events_of(fixture), Paris, AJAX).unwrap_err();
        assert_eq!(problem.code, ErrorCode::UpstreamChanged, "{fixture}");
        // La structure seulement : jamais le contenu de l'événement.
        let detail = detail(&problem);
        assert!(!detail.contains("Devoir"), "{detail}");
    }
    let problem = parse_homework(&[json!("texte")], Paris, AJAX).unwrap_err();
    assert_eq!(problem.code, ErrorCode::UpstreamChanged);
}

#[test]
fn liste_vide_et_champs_facultatifs_absents() {
    assert!(parse_homework(&[], Paris, AJAX).unwrap().is_empty());
    let event = json!({ "id": 5, "name": "Devoir seul", "timestart": 1_792_533_540 });
    let homework = parse_homework(&[event], Paris, AJAX).unwrap();
    assert_eq!(homework[0].title, "Devoir seul");
    assert_eq!(homework[0].course, None);
    assert_eq!(homework[0].description, None);
    assert_eq!(homework[0].url, None);
    assert_eq!(homework[0].submitted, None);
}

#[test]
fn un_devoir_sans_action_n_est_pas_declare_non_rendu() {
    let event = json!({ "id": 6, "name": "TP", "modulename": "assign", "timesort": 1_792_533_540 });
    let homework = parse_homework(&[event], Paris, AJAX).unwrap();
    assert_eq!(homework[0].submitted, None);
}

// réponses d'erreur de Moodle, en HTTP 200

#[test]
fn erreurs_ajax_en_http_200() {
    assert!(matches!(
        ajax_data(json(INVALID_SESSKEY)),
        Err(CallError::SessionLost)
    ));
    // Toute la requête refusée : un objet, pas un tableau.
    assert!(matches!(
        ajax_data(json(WHOLE_REFUSED)),
        Err(CallError::SessionLost)
    ));
    let Err(CallError::Failed(problem)) = ajax_data(json(SERVICE_NOT_AVAILABLE)) else {
        panic!("erreur attendue");
    };
    assert_eq!(problem.code, ErrorCode::ServiceUnavailable);
    assert!(!detail(&problem).contains("SECRET"), "{problem:?}");

    let unknown = json!([{ "error": true, "exception": { "message": "SECRET", "errorcode": "invalidparameter" } }]);
    let Err(CallError::Failed(problem)) = ajax_data(unknown) else {
        panic!("erreur attendue");
    };
    assert_eq!(problem.code, ErrorCode::UpstreamChanged);
    assert!(detail(&problem).contains("invalidparameter"));
    assert!(!detail(&problem).contains("SECRET"));

    for shape in [
        json!([]),
        json!({ "autre": 1 }),
        json!([{ "error": false }]),
    ] {
        let Err(CallError::Failed(problem)) = ajax_data(shape) else {
            panic!("réponse sans données acceptée");
        };
        assert_eq!(problem.code, ErrorCode::UpstreamChanged);
    }
}

#[test]
fn erreurs_du_webservice_en_http_200() {
    for fixture in [INVALID_TOKEN, ACCESS_EXCEPTION] {
        assert!(matches!(
            webservice_data(json(fixture)),
            Err(CallError::SessionLost)
        ));
    }
    let sans_code = json!({ "exception": "moodle_exception", "message": "SECRET" });
    let Err(CallError::Failed(problem)) = webservice_data(sans_code) else {
        panic!("erreur attendue");
    };
    assert_eq!(problem.code, ErrorCode::UpstreamChanged);
    assert!(!detail(&problem).contains("SECRET"));
    // Un code d'erreur piégé n'entre dans le message que nettoyé.
    assert_eq!(safe_code("bad code/../<script>"), "badcodescript");
    assert!(safe_code(&"a".repeat(200)).len() <= 48);
}

#[test]
fn jeton_de_login_token_php() {
    assert_eq!(
        parse_token(&json(TOKEN_OK)).unwrap(),
        "0123456789abcdef0123456789abcdef"
    );
    let refused = parse_token(&json(TOKEN_INVALID_LOGIN)).unwrap_err();
    assert_eq!(refused.code, ErrorCode::InvalidCredentials);
    assert_eq!(refused.detail, None);

    let expired = parse_token(&json!({ "errorcode": "passwordisexpired", "error": "SECRET" }));
    assert_eq!(expired.unwrap_err().code, ErrorCode::AccountActionRequired);
    for code in [
        "sitemaintenance",
        "servicenotavailable",
        "enablewsdescription",
    ] {
        let problem = parse_token(&json!({ "errorcode": code, "error": "SECRET" })).unwrap_err();
        assert_eq!(problem.code, ErrorCode::ServiceUnavailable);
        assert!(!detail(&problem).contains("SECRET"));
    }
    for other in [
        json!({}),
        json!({ "errorcode": "missingparam", "error": "SECRET" }),
    ] {
        let problem = parse_token(&other).unwrap_err();
        assert_eq!(problem.code, ErrorCode::UpstreamChanged);
        assert!(!detail(&problem).contains("SECRET"));
    }
}

#[test]
fn arguments_au_format_de_server_php() {
    let args = json!({ "timesortfrom": 12, "limittononsuspendedevents": true, "x": false, "s": "a b", "ignoré": [1] });
    let mut pairs = form_pairs(&args);
    pairs.sort();
    assert_eq!(
        pairs,
        [
            ("limittononsuspendedevents".to_owned(), "1".to_owned()),
            ("s".to_owned(), "a b".to_owned()),
            ("timesortfrom".to_owned(), "12".to_owned()),
            ("x".to_owned(), "0".to_owned()),
        ]
    );
    assert!(form_pairs(&json!(null)).is_empty());
}

// page Moodle

#[test]
fn sesskey_lu_dans_le_m_cfg() {
    assert_eq!(
        page_state(MY_PAGE),
        PageState::LoggedIn("AbCdEf1234".to_owned())
    );
    // Page d'invité : la clé existe mais la session n'est pas celle de l'étudiant.
    assert_eq!(page_state(MY_PAGE_GUEST), PageState::NotLoggedIn);
    assert_eq!(page_state(MY_PAGE_WITHOUT_SESSKEY), PageState::NoSesskey);
    assert_eq!(page_state(""), PageState::NoSesskey);
}

#[test]
fn sesskey_avec_espaces_et_fausses_pistes() {
    let page = r#"<script>var a = {"sesskey" : "Zz9900aa"}</script>"#;
    assert_eq!(sesskey_of(page).as_deref(), Some("Zz9900aa"));
    // Un `"sesskey"` sans valeur ou trop court est sauté, le suivant est lu.
    let page = r#"x "sesskey" y "sesskey":"abc" "sesskey":"Valid12345""#;
    assert_eq!(sesskey_of(page).as_deref(), Some("Valid12345"));
    assert_eq!(sesskey_of(r#""sesskey":"Abcdef"#), None);
    assert_eq!(sesskey_of(r#""sesskey":null"#), None);
}

// texte

#[test]
fn nettoyage_des_titres() {
    assert_eq!(clean_name("TP 3 doit être rendu"), "TP 3");
    assert_eq!(clean_name("Quiz 1 FERME"), "Quiz 1");
    assert_eq!(clean_name("Compte rendu est dû"), "Compte rendu");
    assert_eq!(clean_name("Atelier se termine"), "Atelier");
    assert_eq!(clean_name("Projet"), "Projet");
    // Un titre réduit au suffixe est conservé tel quel.
    assert_eq!(clean_name("ferme"), "ferme");
}

#[test]
fn entites_html() {
    assert_eq!(
        decode_entities("A &amp; B &lt;C&gt; &quot;d&quot; l&#039;e &#x27;f&#X27; &eacute; & &amp"),
        "A & B <C> \"d\" l'e 'f' &eacute; & &amp"
    );
    assert_eq!(decode_entities("sans entité"), "sans entité");
    assert_eq!(decode_entities("&#99999999999;"), "&#99999999999;");
}

#[test]
fn texte_d_un_fragment_html() {
    assert_eq!(
        html_to_text("<div>Un<br/>deux <b>gras</b>é</div><ul><li>a</li><li>b</li></ul>"),
        "Un deux grasé a b"
    );
    assert_eq!(
        html_to_text("&lt;b&gt; reste du texte"),
        "<b> reste du texte"
    );
    assert_eq!(html_to_text("<p> </p>"), "");
}

// redirections écrites dans une page

#[test]
fn redirections_html() {
    let from = Url::parse("https://idp.exemple.test/a/b").unwrap();
    let meta =
        r#"<html><head><META HTTP-EQUIV="Refresh" CONTENT="0; URL='/suite?x=1&amp;y=2'"></head>"#;
    assert_eq!(
        html_redirect(meta, &from).unwrap().as_str(),
        "https://idp.exemple.test/suite?x=1&y=2"
    );
    let script = r#"<script>window.location = "https://moodle.exemple.test/my/";</script>"#;
    assert_eq!(
        html_redirect(script, &from).unwrap().as_str(),
        "https://moodle.exemple.test/my/"
    );
    let replace = "<script>location.replace('relative')</script>";
    assert_eq!(
        html_redirect(replace, &from).unwrap().as_str(),
        "https://idp.exemple.test/a/relative"
    );
    assert!(html_redirect("<p>rien</p>", &from).is_none());
    assert!(html_redirect("<meta charset=\"utf-8\">", &from).is_none());
}

fn saml(hosts: &[&str]) -> SamlConfig {
    SamlConfig {
        wayf_url: Url::parse("https://wayf.exemple.test/").unwrap(),
        idp_entity_id: "urn:test:idp".to_owned(),
        allowed_hosts: hosts.iter().map(|h| (*h).to_owned()).collect(),
    }
}

#[test]
fn hotes_autorises_pendant_la_federation() {
    let base = Url::parse("https://foad.exemple.test/").unwrap();
    let saml = saml(&["exemple.test", "renater.fr"]);
    let url = |s: &str| Url::parse(s).unwrap();
    let allowed = |u: &str| host_allowed(&url(u), &base, Some("cas.univ.test"), &saml);
    assert!(allowed("https://foad.exemple.test/my/"));
    assert!(allowed("https://wayf.exemple.test/x"));
    assert!(allowed("https://exemple.test/"));
    assert!(allowed("https://discovery.renater.fr/"));
    assert!(allowed("https://cas.univ.test/login"));
    assert!(!allowed("https://pirate.test/"));
    assert!(!allowed("https://exemple.test.pirate.test/"));
    assert!(!allowed("https://evilrenater.fr/"));
}

// sign_in contre un faux login/token.php

#[derive(Default)]
struct TokenMock {
    bodies: Vec<String>,
    reply: &'static str,
}

type Shared = Arc<Mutex<TokenMock>>;

async fn token_php(State(state): State<Shared>, body: String) -> String {
    let mut state = state.lock().unwrap();
    state.bodies.push(body);
    state.reply.to_owned()
}

async fn start(reply: &'static str) -> (Shared, Url) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!(
        "http://{}/moodle/",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let state: Shared = Arc::new(Mutex::new(TokenMock {
        reply,
        ..TokenMock::default()
    }));
    let app = Router::new()
        .route("/moodle/login/token.php", post(token_php))
        .with_state(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (state, base)
}

fn university() -> ResolvedUniversity {
    ResolvedUniversity {
        dir: PathBuf::new(),
        university: UniversityFile {
            schema_version: 1,
            id: "demo-moodle".to_owned(),
            name: "Université de test".to_owned(),
            short_name: None,
            status: Status::default(),
            timezone: "Europe/Paris".to_owned(),
            links: tonapi_config::Links::default(),
            sso: Sso::Cas(Box::new(CasSso {
                base_url: Url::parse("http://127.0.0.1:9/cas/").unwrap(),
                flavor: CasFlavor::Apereo,
                mfa: None,
                saml: None,
            })),
            portal: tonapi_config::PortalConfig::None,
            services: Services::default(),
            tls: None,
        },
        establishments: BTreeMap::new(),
    }
}

fn session() -> SessionData {
    SessionData {
        university: "demo-moodle".to_owned(),
        username: "e00000000000".to_owned(),
        establishment: None,
        cookies: Vec::new(),
        tokens: BTreeMap::new(),
        pending: false,
        issued_at: 0,
        expires_at: 3600,
    }
}

fn webservice(base: &Url, sign_in: SignIn) -> HomeworkService {
    HomeworkService::MoodleWebservice {
        base_url: base.clone(),
        sign_in,
        options: ServiceOptions::default(),
    }
}

const LOCAL: SignIn = SignIn::LocalPassword {
    credentials: Credentials::University,
};

#[tokio::test]
async fn sign_in_ne_garde_que_le_jeton() {
    let (state, base) = start(TOKEN_OK).await;
    let university = university();
    let mut session = session();
    let mut ctx = Context {
        university: &university,
        session: &mut session,
        now: Utc::now(),
        net: NetSettings { https_only: false },
    };
    sign_in(
        &mut ctx,
        &webservice(&base, LOCAL),
        "e00000000000",
        "mot de passe & secret",
    )
    .await
    .unwrap();

    // Une seule requête, avec le service mobile officiel.
    let bodies = state.lock().unwrap().bodies.clone();
    assert_eq!(bodies.len(), 1);
    assert!(bodies[0].contains("service=moodle_mobile_app"));
    assert!(bodies[0].contains("username=e00000000000"));

    // Dans la session : le jeton, ni le `privatetoken` ni le mot de passe.
    let slot = token_slot(&base);
    assert_eq!(
        session.tokens.get(&slot).map(String::as_str),
        Some("0123456789abcdef0123456789abcdef")
    );
    assert_eq!(session.tokens.len(), 1);
    let stored = format!("{:?}{:?}", session.tokens, session.cookies);
    assert!(!stored.contains("PRIVATE-NEVER-KEPT"));
    assert!(!stored.contains("secret"));
}

#[tokio::test]
async fn sign_in_n_insiste_pas_apres_invalidlogin() {
    let (state, base) = start(TOKEN_INVALID_LOGIN).await;
    let university = university();
    let mut session = session();
    let mut ctx = Context {
        university: &university,
        session: &mut session,
        now: Utc::now(),
        net: NetSettings { https_only: false },
    };
    let problem = sign_in(&mut ctx, &webservice(&base, LOCAL), "e00000000000", "faux")
        .await
        .unwrap_err();
    assert_eq!(problem.code, ErrorCode::InvalidCredentials);
    assert!(!format!("{problem:?}").contains("SECRET"));
    // Un mot de passe refusé n'est jamais rejoué : le compte Moodle pourrait se verrouiller.
    assert_eq!(state.lock().unwrap().bodies.len(), 1);
    assert!(session.tokens.is_empty());
}

#[tokio::test]
async fn sign_in_refuse_les_connexions_par_cas_sans_rien_envoyer() {
    let (state, base) = start(TOKEN_OK).await;
    let university = university();
    let mut session = session();
    let mut ctx = Context {
        university: &university,
        session: &mut session,
        now: Utc::now(),
        net: NetSettings { https_only: false },
    };
    let methods = [
        SignIn::UniversityCas { service: None },
        SignIn::OwnCas {
            base_url: Url::parse("https://cas.composante.test/").unwrap(),
            flavor: CasFlavor::Phpcas,
            credentials: Credentials::University,
        },
    ];
    for method in methods {
        let problem = sign_in(&mut ctx, &webservice(&base, method), "u", "p")
            .await
            .unwrap_err();
        assert_eq!(problem.code, ErrorCode::NotImplemented);
    }
    let ajax = HomeworkService::MoodleAjax {
        base_url: base.clone(),
        entry: MoodleEntry::Cas,
        options: ServiceOptions::default(),
    };
    let problem = sign_in(&mut ctx, &ajax, "u", "p").await.unwrap_err();
    assert_eq!(problem.code, ErrorCode::InvalidRequest);
    assert!(state.lock().unwrap().bodies.is_empty());
}

#[test]
fn sans_jeton_la_reponse_depend_de_la_configuration() {
    assert_eq!(login_required(&LOCAL).code, ErrorCode::ServiceLoginRequired);
    assert_eq!(
        login_required(&SignIn::UniversityCas { service: None }).code,
        ErrorCode::NotImplemented
    );
}

/// Le serveur axum exige des futurs `Send` : cela se vérifie à la compilation.
fn assert_send<T: Send>(_: &T) {}

#[allow(dead_code)]
fn les_futurs_sont_send(ctx: &mut Context<'_>, base: &Url, config: &HomeworkService) {
    assert_send(&fetch_ajax(ctx, base, MoodleEntry::Shibboleth));
    assert_send(&fetch_webservice(ctx, base, &LOCAL));
    assert_send(&sign_in(ctx, config, "u", "p"));
}

/// Un `&` sans `;` proche relisait toute la fin du texte, et chaque `<meta`
/// d'une même balise rescannait la fin de la page.
#[test]
fn entites_et_meta_refresh_en_temps_lineaire() {
    assert_eq!(decode_entities("&amp;aéééé"), "&aéééé");
    assert_eq!(decode_entities("&éééééééééé;"), "&éééééééééé;");
    let from = Url::parse("https://idp.exemple.test/").unwrap();
    let started = std::time::Instant::now();
    let ampersands = format!("{};", "&".repeat(200_000));
    assert_eq!(decode_entities(&ampersands), ampersands);
    let metas = format!("{}>", "<meta".repeat(200_000));
    assert!(html_redirect(&metas, &from).is_none());
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}
