//! Fournisseur `scodoc-gateway` contre un faux serveur ScoDoc + CAS local. Aucun accès réseau réel.

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::Utc;
use serde_json::{Value, json};
use tonapi_config::{ResolvedUniversity, UniversityFile};
use tonapi_core::ErrorCode;
use tonapi_providers::{Context, NetSettings, absences, grades};
use tonapi_session::{SessionData, StoredCookie};

use support::Server;

const PAYLOAD: &str = include_str!("fixtures/scodoc/premiere_connexion.json");

fn university(server: &Server, sign_in: &Value) -> ResolvedUniversity {
    ResolvedUniversity {
        dir: PathBuf::new(),
        university: serde_json::from_value::<UniversityFile>(server.university(sign_in)).unwrap(),
        establishments: BTreeMap::new(),
    }
}

fn session(cookies: Vec<StoredCookie>) -> SessionData {
    SessionData {
        university: "test".to_owned(),
        username: "e00000000000".to_owned(),
        establishment: None,
        cookies,
        tokens: BTreeMap::new(),
        pending: false,
        issued_at: 0,
        expires_at: i64::MAX,
    }
}

fn cookie(name: &str, value: &str) -> StoredCookie {
    StoredCookie {
        domain: "127.0.0.1".to_owned(),
        path: "/".to_owned(),
        name: name.to_owned(),
        value: value.to_owned(),
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

fn university_cas() -> Value {
    json!({ "kind": "university-cas" })
}

fn own_cas(server: &Server) -> Value {
    json!({
        "kind": "own-cas",
        "base-url": server.cas(),
        "flavor": "phpcas",
        "credentials": { "kind": "separate", "label": "Identifiant IUT (prenom.nom)" },
    })
}

#[tokio::test]
async fn notes_et_absences_par_le_cas_de_l_universite() {
    let server = support::start(PAYLOAD).await;
    let university = university(&server, &university_cas());
    let mut session = session(vec![cookie("TGC", support::TGC)]);
    let mut ctx = context(&university, &mut session);

    let notes = grades(&mut ctx).await.unwrap();
    assert_eq!(notes.provider, "scodoc-gateway");
    let semester = &notes.data.semesters[0];
    assert_eq!(semester.id, "901");
    assert!((semester.average.unwrap().value - 13.96).abs() < 1e-9);
    assert_eq!(semester.rank.as_deref(), Some("12/56"));
    assert_eq!(semester.modules.len(), 3);
    {
        let state = server.state();
        assert_eq!(state.ticket_validations, 1);
        assert_eq!(state.data_calls, 1, "pas d'appel avant d'avoir une session");
        assert_eq!(state.data_methods, ["POST"]);
        assert_eq!(
            state.password_posts, 0,
            "le mot de passe ne sert jamais ici"
        );
    }
    assert!(ctx.session.cookies.iter().any(|c| c.name == "PHPSESSID"));

    // Le PHPSESSID gardé dans la session évite de repasser par le CAS.
    let report = absences(&mut ctx).await.unwrap().data;
    assert_eq!(report.absences.len(), 3);
    assert_eq!(report.totals.justified, 1);
    assert_eq!(report.totals.unjustified, 2);
    let state = server.state();
    assert_eq!(state.ticket_validations, 1);
    assert_eq!(state.data_calls, 2);
}

#[tokio::test]
async fn session_perdue_la_session_est_rouverte_puis_un_seul_nouvel_essai() {
    let server = support::start(PAYLOAD).await;
    let university = university(&server, &university_cas());
    let mut session = session(vec![cookie("TGC", support::TGC)]);
    let mut ctx = context(&university, &mut session);
    grades(&mut ctx).await.unwrap();

    // `{"redirect"}` une fois : la session est rouverte, l'appel refait, et ça passe.
    server.state().expire_next = 1;
    grades(&mut ctx).await.unwrap();
    assert_eq!(server.state().data_calls, 3);

    // La session PHP a expiré côté passerelle : retour par le CAS avec le TGC, sans mot de passe.
    server.expire_sessions();
    grades(&mut ctx).await.unwrap();
    {
        let state = server.state();
        assert_eq!(state.ticket_validations, 2);
        assert_eq!(state.data_calls, 5);
        assert_eq!(state.password_posts, 0);
    }

    // `{"redirect"}` après la réouverture : session_expired, pas de troisième appel.
    server.state().always_expired = true;
    let before = server.state().data_calls;
    let error = grades(&mut ctx).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::SessionExpired);
    assert_eq!(server.state().data_calls - before, 2);
}

#[tokio::test]
async fn sans_tgc_session_expiree_et_le_mot_de_passe_n_est_pas_demande() {
    let server = support::start(PAYLOAD).await;
    let university = university(&server, &university_cas());
    let mut session = session(Vec::new());
    let mut ctx = context(&university, &mut session);

    let error = grades(&mut ctx).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::SessionExpired);
    let state = server.state();
    assert_eq!(state.password_posts, 0);
    assert_eq!(state.data_calls, 0);
}

#[tokio::test]
async fn erreur_applicative_sans_recopier_le_message() {
    let payload = r#"{"erreur":"Relevé non disponible pour Camille Demo e00000000000"}"#;
    let server = support::start(payload).await;
    let university = university(&server, &university_cas());
    let mut session = session(vec![cookie("TGC", support::TGC)]);
    let mut ctx = context(&university, &mut session);

    let error = grades(&mut ctx).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::ServiceUnavailable);
    let detail = error.detail.unwrap();
    assert!(!detail.contains("Camille") && !detail.contains("e00000000000"));
}

#[tokio::test]
async fn reponse_non_json_ou_inattendue() {
    for payload in ["<html>maintenance</html>", "[1, 2]"] {
        let server = support::start(payload).await;
        let university = university(&server, &university_cas());
        let mut session = session(vec![cookie("TGC", support::TGC)]);
        let mut ctx = context(&university, &mut session);
        let error = grades(&mut ctx).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamChanged, "{payload}");
    }
}

#[tokio::test]
async fn cas_de_la_composante_sans_session_de_service() {
    let server = support::start(PAYLOAD).await;
    let university = university(&server, &own_cas(&server));
    let mut session = session(vec![cookie("TGC", support::TGC)]);
    let mut ctx = context(&university, &mut session);

    for error in [
        grades(&mut ctx).await.unwrap_err(),
        absences(&mut ctx).await.unwrap_err(),
    ] {
        assert_eq!(error.code, ErrorCode::ServiceLoginRequired);
        assert!(error.detail.unwrap().contains("connexion au service"));
    }
    // Aucune requête : ni la passerelle ni le CAS n'ont été contactés.
    let state = server.state();
    assert_eq!(
        (state.data_calls, state.cas_forms, state.password_posts),
        (0, 0, 0)
    );
    assert!(state.cas_cookies.is_empty());
}

#[tokio::test]
async fn cas_de_la_composante_avec_jeton_de_service() {
    let server = support::start(PAYLOAD).await;
    let university = university(&server, &own_cas(&server));
    // Les cookies de l'université n'ont aucune place dans les échanges avec ce service.
    let mut session = session(vec![cookie("UNIV", "secret-universite")]);
    let key = format!("scodoc.{}", server.authority());
    // Jeton de service : le TGC du CAS de la composante, sans session PHP.
    let token = serde_json::to_string(&[cookie("CASTGC", support::TGC)]).unwrap();
    session.tokens.insert(key.clone(), token);
    let mut ctx = context(&university, &mut session);

    let notes = grades(&mut ctx).await.unwrap();
    assert_eq!(notes.data.semesters.len(), 1);
    {
        let state = server.state();
        assert_eq!(state.password_posts, 0);
        assert!(state.cas_cookies.iter().all(|c| !c.contains("UNIV")));
    }
    assert_eq!(ctx.session.cookies, [cookie("UNIV", "secret-universite")]);
    assert!(ctx.session.tokens[&key].contains("PHPSESSID"));

    // La session PHP du jeton suffit pour la suite : pas de nouveau passage par le CAS.
    absences(&mut ctx).await.unwrap();
    assert_eq!(server.state().ticket_validations, 1);
}

#[tokio::test]
async fn cas_de_la_composante_jeton_sans_tgc_demande_une_nouvelle_connexion() {
    let server = support::start(PAYLOAD).await;
    let university = university(&server, &own_cas(&server));
    let mut session = session(Vec::new());
    let key = format!("scodoc.{}", server.authority());
    session.tokens.insert(key, "[]".to_owned());
    let mut ctx = context(&university, &mut session);

    let error = grades(&mut ctx).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::ServiceLoginRequired);
    assert_eq!(server.state().password_posts, 0);
}

#[tokio::test]
async fn connexion_locale_non_prise_en_charge() {
    let server = support::start(PAYLOAD).await;
    let university = university(&server, &json!({ "kind": "local-password" }));
    let mut session = session(Vec::new());
    let mut ctx = context(&university, &mut session);
    let error = grades(&mut ctx).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::NotImplemented);
}
