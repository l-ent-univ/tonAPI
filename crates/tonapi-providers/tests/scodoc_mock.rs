//! Fournisseur `scodoc-gateway` contre un faux serveur ScoDoc + CAS local. Aucun accès réseau réel.

mod common;
mod support;

use chrono::Utc;
use serde_json::{Value, json};
use tonapi_config::{ResolvedUniversity, ServiceKind};
use tonapi_core::DistributionUnavailable;
use tonapi_core::ErrorCode;
use tonapi_providers::{
    Context, absences, grade_distribution, grade_notice, grade_transcript, grades,
    grades_and_absences, grades_and_absences_of, grades_of, service_login, student_picture,
};
use tonapi_session::{SessionData, StoredCookie};

use support::Server;

const PAYLOAD: &str = include_str!("fixtures/scodoc/premiere_connexion.json");
const FULL: &str = include_str!("fixtures/scodoc/releve_complet.json");

fn university(server: &Server, sign_in: &Value) -> ResolvedUniversity {
    common::university(server.university(sign_in))
}

fn session(cookies: Vec<StoredCookie>) -> SessionData {
    common::session("test", "e00000000000", cookies)
}

fn cookie(name: &str, value: &str) -> StoredCookie {
    common::stored_cookie(name, value, "/")
}

fn context<'a>(university: &'a ResolvedUniversity, session: &'a mut SessionData) -> Context<'a> {
    common::context(university, session, Utc::now())
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
async fn notes_et_absences_d_une_seule_lecture() {
    let server = support::start(PAYLOAD).await;
    let university = university(&server, &university_cas());
    let mut session = session(vec![cookie("TGC", support::TGC)]);
    let (notes, report) = grades_and_absences(&mut context(&university, &mut session)).await;
    assert_eq!(notes.unwrap().data.semesters[0].id, "901");
    assert_eq!(report.unwrap().data.absences.len(), 3);
    assert_eq!(server.state().data_calls, 1, "une seule requête à data.php");
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

#[tokio::test]
async fn connexion_au_service_puis_notes_et_absences() {
    let server = support::start(PAYLOAD).await;
    let university = university(&server, &own_cas(&server));
    let mut session = session(Vec::new());
    let mut ctx = context(&university, &mut session);

    service_login(
        &mut ctx,
        ServiceKind::Grades,
        support::USERNAME,
        support::PASSWORD,
    )
    .await
    .unwrap();
    let key = format!("scodoc.{}", server.authority());
    assert!(!ctx.session.tokens[&key].contains(support::PASSWORD));

    // La connexion faite pour les notes sert aussi aux absences.
    grades(&mut ctx).await.unwrap();
    absences(&mut ctx).await.unwrap();
    assert_eq!(server.state().password_posts, 1);
}

#[tokio::test]
async fn connexion_au_service_mauvais_mot_de_passe() {
    let server = support::start(PAYLOAD).await;
    let university = university(&server, &own_cas(&server));
    let mut session = session(Vec::new());
    let mut ctx = context(&university, &mut session);

    let error = service_login(&mut ctx, ServiceKind::Absences, support::USERNAME, "faux")
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidCredentials);
    assert!(ctx.session.tokens.is_empty());
    assert_eq!(server.state().password_posts, 1);
}

// autour du relevé : autre semestre, message, répartition, PDF

/// Réponse de `relevéEtudiant` pour le semestre du relevé complet : ni `config` ni `semestres`.
fn semester_payload() -> String {
    let full: Value = serde_json::from_str(FULL).unwrap();
    json!({ "relevé": full["relevé"], "absences": [] }).to_string()
}

async fn full_server() -> Server {
    let server = support::start(FULL).await;
    {
        let mut state = server.state();
        state.semesters.insert("901".to_owned(), semester_payload());
        state.message = String::from(r#"{"message":"<p>Jury le 12 f&eacute;vrier.</p>"}"#);
        state.distribution = String::from("[4.5,9,9.5,11,12.25,13,13.5,16,20]");
        state.pdf = true;
    }
    server
}

#[tokio::test]
async fn releve_d_un_autre_semestre() {
    let server = full_server().await;
    let university = university(&server, &university_cas());
    let mut session = session(vec![cookie("TGC", support::TGC)]);
    let mut ctx = context(&university, &mut session);

    let current = grades(&mut ctx).await.unwrap().data;
    assert_eq!(current.available.len(), 3);
    assert_eq!(current.semesters[0].transcript, Some(true));

    let asked = grades_of(&mut ctx, Some("901")).await.unwrap();
    assert_eq!(asked.provider, "scodoc-gateway");
    assert_eq!(asked.data.semesters[0].id, "901");
    assert_eq!(asked.data.semesters[0].units.len(), 3);
    // Les réglages de la passerelle ne viennent pas avec ce relevé.
    assert_eq!(asked.data.semesters[0].transcript, None);
    assert!(asked.data.available.is_empty());

    // Semestre que la passerelle ne connaît pas, puis identifiant qui n'en est pas un.
    let unknown = grades_of(&mut ctx, Some("123")).await.unwrap_err();
    assert_eq!(unknown.code, ErrorCode::ServiceUnavailable);
    let before = server.state().data_calls;
    let invalid = grades_of(&mut ctx, Some("12a")).await.unwrap_err();
    assert_eq!(invalid.code, ErrorCode::InvalidRequest);
    let state = server.state();
    assert_eq!(
        state.data_calls, before,
        "rien n'est demandé pour un identifiant invalide"
    );
    assert_eq!(
        state.data_actions,
        [
            "dataPremièreConnexion",
            "relevéEtudiant:901",
            "relevéEtudiant:123"
        ]
    );
    // Une seule connexion pour tout, et `GET` hors du premier appel.
    assert_eq!(state.ticket_validations, 1);
    assert_eq!(state.data_methods, ["POST", "GET", "GET"]);
}

#[tokio::test]
async fn message_du_departement() {
    let server = full_server().await;
    let university = university(&server, &university_cas());
    let mut session = session(vec![cookie("TGC", support::TGC)]);
    let mut ctx = context(&university, &mut session);

    let notice = grade_notice(&mut ctx, "DEMO").await.unwrap();
    assert_eq!(notice.data.message.as_deref(), Some("Jury le 12 février."));
    server.state().message = String::from(r#"{"message":""}"#);
    assert_eq!(
        grade_notice(&mut ctx, "DEMO").await.unwrap().data.message,
        None
    );
    assert_eq!(
        server.state().data_actions,
        ["getReportPageMessage:DEMO", "getReportPageMessage:DEMO"]
    );
}

#[tokio::test]
async fn repartition_d_une_evaluation_du_releve_seulement() {
    let server = full_server().await;
    let university = university(&server, &university_cas());
    let mut session = session(vec![cookie("TGC", support::TGC)]);
    let mut ctx = context(&university, &mut session);

    let distribution = grade_distribution(&mut ctx, "901", "7301")
        .await
        .unwrap()
        .data;
    assert_eq!(distribution.unavailable, None);
    assert_eq!(distribution.total, 9);
    assert_eq!(distribution.counts[9], 2);
    assert_eq!(distribution.counts[20], 1);

    // Une évaluation qui n'est pas dans le relevé de l'étudiant n'est jamais demandée.
    let foreign = grade_distribution(&mut ctx, "901", "9999")
        .await
        .unwrap_err();
    assert_eq!(foreign.code, ErrorCode::NotFound);
    assert_eq!(
        server.state().data_actions,
        [
            "relevéEtudiant:901",
            "listeNotes:7301",
            "relevéEtudiant:901"
        ]
    );

    // Ce que la passerelle refuse : trop peu de notes, histogramme coupé.
    for (answer, reason) in [
        (r#"["too low"]"#, DistributionUnavailable::TooFew),
        (
            r#"{"erreur":"L'histogramme n'est pas activé"}"#,
            DistributionUnavailable::Disabled,
        ),
    ] {
        server.state().distribution = String::from(answer);
        let refused = grade_distribution(&mut ctx, "901", "7301")
            .await
            .unwrap()
            .data;
        assert_eq!(refused.unavailable, Some(reason));
        assert!(refused.counts.is_empty());
    }

    // Relevé non publié : pas de répartition, et `listeNotes` n'est pas appelé.
    let mut hidden: Value = serde_json::from_str(&semester_payload()).unwrap();
    hidden["relevé"]["publie"] = json!(false);
    server
        .state()
        .semesters
        .insert("901".to_owned(), hidden.to_string());
    let calls = server.state().data_calls;
    let refused = grade_distribution(&mut ctx, "901", "7301")
        .await
        .unwrap()
        .data;
    assert_eq!(refused.unavailable, Some(DistributionUnavailable::Hidden));
    assert_eq!(server.state().data_calls, calls + 1);
}

#[tokio::test]
async fn releve_pdf_avec_l_identifiant_de_la_session() {
    let server = full_server().await;
    let university = university(&server, &university_cas());
    let mut session = session(vec![cookie("TGC", support::TGC)]);
    let mut ctx = context(&university, &mut session);

    let file = grade_transcript(&mut ctx, "890").await.unwrap().data;
    assert_eq!(file.content_type, "application/pdf");
    assert_eq!(file.file_name, "releve-890.pdf");
    assert!(file.bytes.starts_with(b"%PDF"));
    {
        let state = server.state();
        // L'étudiant est celui de la session ouverte, le type celui de son relevé.
        assert_eq!(
            state.pdf_queries,
            ["etudiant=e00000000000&sem_id=890&type=BUT"]
        );
        assert_eq!(state.ticket_validations, 1);
        assert_eq!(state.data_actions, ["dataPremièreConnexion"]);
    }

    // L'identifiant reste dans la session : le PDF suivant ne relit pas le relevé.
    let again = grade_transcript(&mut ctx, "880").await.unwrap().data;
    assert_eq!(again.file_name, "releve-880.pdf");
    {
        let state = server.state();
        assert_eq!(state.data_actions, ["dataPremièreConnexion"]);
        assert_eq!(state.pdf_queries.len(), 2);
        assert_eq!(
            state.pdf_queries[1],
            "etudiant=e00000000000&sem_id=880&type=BUT"
        );
    }

    // La passerelle ne rend pas de PDF : un seul nouvel essai après une reconnexion, puis l'erreur.
    server.state().pdf = false;
    let missing = grade_transcript(&mut ctx, "890").await.unwrap_err();
    assert_eq!(missing.code, ErrorCode::NotFound);
    assert_eq!(server.state().pdf_queries.len(), 4);

    let invalid = grade_transcript(&mut ctx, "../890").await.unwrap_err();
    assert_eq!(invalid.code, ErrorCode::InvalidRequest);
    assert_eq!(server.state().pdf_queries.len(), 4);
}

#[tokio::test]
async fn une_lecture_du_releve_suffit_au_pdf_qui_suit() {
    let server = full_server().await;
    let university = university(&server, &university_cas());
    let mut session = session(vec![cookie("TGC", support::TGC)]);
    {
        let mut ctx = context(&university, &mut session);
        grades(&mut ctx).await.unwrap();
    }
    // Ce que la session garde : l'identifiant pour la passerelle et le type du bulletin, rien du
    // relevé lui-même.
    let key = format!("scodoc-student.{}", server.authority());
    let kept: Value = serde_json::from_str(&session.tokens[&key]).unwrap();
    assert_eq!(kept, json!({ "id": "e00000000000", "kind": "BUT" }));

    let mut ctx = context(&university, &mut session);
    grade_transcript(&mut ctx, "901").await.unwrap();
    let state = server.state();
    assert_eq!(state.data_actions, ["dataPremièreConnexion"]);
    assert_eq!(
        state.pdf_queries,
        ["etudiant=e00000000000&sem_id=901&type=BUT"]
    );
}

#[tokio::test]
async fn sans_tgc_rien_de_tout_cela_n_est_servi() {
    let server = full_server().await;
    let university = university(&server, &university_cas());
    let mut session = session(Vec::new());
    let mut ctx = context(&university, &mut session);

    assert_eq!(
        grades_of(&mut ctx, Some("901")).await.unwrap_err().code,
        ErrorCode::SessionExpired
    );
    assert_eq!(
        grade_notice(&mut ctx, "DEMO").await.unwrap_err().code,
        ErrorCode::SessionExpired
    );
    assert_eq!(
        grade_distribution(&mut ctx, "901", "7301")
            .await
            .unwrap_err()
            .code,
        ErrorCode::SessionExpired
    );
    assert_eq!(
        grade_transcript(&mut ctx, "901").await.unwrap_err().code,
        ErrorCode::SessionExpired
    );
    // Le CAS a montré son formulaire à chaque fois : aucune session ouverte, aucun PDF demandé.
    let state = server.state();
    assert_eq!(state.ticket_validations, 0);
    assert_eq!(state.password_posts, 0);
    assert!(state.pdf_queries.is_empty());
}

#[tokio::test]
async fn notes_et_absences_d_un_autre_semestre_d_une_seule_lecture() {
    let server = full_server().await;
    let university = university(&server, &university_cas());
    let mut session = session(vec![cookie("TGC", support::TGC)]);
    let mut ctx = context(&university, &mut session);

    let (notes, report) = grades_and_absences_of(&mut ctx, Some("901")).await;
    assert_eq!(notes.unwrap().data.semesters[0].id, "901");
    let report = report.unwrap().data;
    // Les réglages de la passerelle ne viennent pas avec ce relevé.
    assert_eq!((report.visible, report.unit), (None, None));
    assert!(report.absences.is_empty());
    assert_eq!(server.state().data_actions, ["relevéEtudiant:901"]);

    // Sans semestre : la première connexion, comme `grades_and_absences`.
    let (notes, report) = grades_and_absences_of(&mut ctx, None).await;
    assert_eq!(notes.unwrap().data.available.len(), 3);
    assert_eq!(report.unwrap().data.visible, Some(true));
    let state = server.state();
    assert_eq!(state.data_actions.len(), 2);
    assert_eq!(state.data_actions[1], "dataPremièreConnexion");
}

/// Début d'un fichier JPEG, inventé.
const PHOTO: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'];

#[tokio::test]
async fn photo_de_l_etudiant_puis_session_perdue_rouverte_une_fois() {
    let server = full_server().await;
    server.state().picture = Some(("image/jpeg; charset=binary".to_owned(), PHOTO.to_vec()));
    let university = university(&server, &university_cas());
    let mut session = session(vec![cookie("TGC", support::TGC)]);
    let mut ctx = context(&university, &mut session);

    let photo = student_picture(&mut ctx).await.unwrap();
    assert_eq!(photo.provider, "scodoc-gateway");
    assert_eq!(photo.data.content_type, "image/jpeg");
    assert_eq!(photo.data.file_name, "photo.jpeg");
    assert_eq!(photo.data.bytes, PHOTO);
    assert_eq!(server.state().ticket_validations, 1);
    assert_eq!(server.state().data_actions, ["getStudentPic"]);
    assert_eq!(server.state().data_methods, ["GET"]);

    // La session PHP posée sert à la demande suivante, sans repasser par le CAS.
    student_picture(&mut ctx).await.unwrap();
    assert_eq!(server.state().ticket_validations, 1);

    // Perdue côté passerelle : rouverte, et la photo redemandée une seule fois.
    server.expire_sessions();
    let again = student_picture(&mut ctx).await.unwrap();
    assert_eq!(again.data.bytes, PHOTO);
    assert_eq!(server.state().ticket_validations, 2);
    assert_eq!(server.state().data_calls, 4);
}

#[tokio::test]
async fn sans_image_pas_de_photo() {
    let server = full_server().await;
    let university = university(&server, &university_cas());
    let mut session = session(vec![cookie("TGC", support::TGC)]);
    let mut ctx = context(&university, &mut session);

    // Une page vide à la place de l'image, même après une nouvelle connexion.
    let none = student_picture(&mut ctx).await.unwrap_err();
    assert_eq!(none.code, ErrorCode::NotFound);

    // Un SVG est un document : jamais rendu comme photo.
    server.state().picture = Some(("image/svg+xml".to_owned(), b"<svg/>".to_vec()));
    let svg = student_picture(&mut ctx).await.unwrap_err();
    assert_eq!(svg.code, ErrorCode::NotFound);

    // Une image annoncée mais vide n'en est pas une.
    server.state().picture = Some(("image/png".to_owned(), Vec::new()));
    let empty = student_picture(&mut ctx).await.unwrap_err();
    assert_eq!(empty.code, ErrorCode::NotFound);

    // La passerelle refuse la session même rouverte : la session de l'université est perdue.
    server.state().always_expired = true;
    let refused = student_picture(&mut ctx).await.unwrap_err();
    assert_eq!(refused.code, ErrorCode::SessionExpired);
}

#[tokio::test]
async fn photo_sans_tgc_ou_sans_passerelle() {
    let server = full_server().await;
    server.state().picture = Some(("image/png".to_owned(), PHOTO.to_vec()));
    let university = university(&server, &university_cas());
    let mut signed_out = session(Vec::new());
    let expired = student_picture(&mut context(&university, &mut signed_out))
        .await
        .unwrap_err();
    assert_eq!(expired.code, ErrorCode::SessionExpired);
    assert_eq!(server.state().password_posts, 0);

    // Sans passerelle ScoDoc pour la composante : le service n'existe pas.
    let without = common::university(json!({
        "id": "test",
        "name": "Université de test",
        "sso": { "kind": "cas", "base-url": server.cas() },
    }));
    let mut session = session(vec![cookie("TGC", support::TGC)]);
    let absent = student_picture(&mut context(&without, &mut session))
        .await
        .unwrap_err();
    assert_eq!(absent.code, ErrorCode::ServiceAbsent);
}
