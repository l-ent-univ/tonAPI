//! Tests du fournisseur `scodoc-gateway` : analyses pures sur les fixtures synthétiques, puis
//! connexion au CAS de la composante contre un faux serveur local.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::Utc;
use chrono_tz::Europe::Paris;
use serde_json::json;
use tonapi_config::{GradesService, ResolvedUniversity, ServiceOptions, UniversityFile};
use tonapi_session::SessionData;

use super::*;
use crate::NetSettings;

const PAYLOAD: &str = include_str!("../../tests/fixtures/scodoc/premiere_connexion.json");
const NO_SEMESTER: &str = include_str!("../../tests/fixtures/scodoc/sans_semestre.json");
const UNPUBLISHED: &str = include_str!("../../tests/fixtures/scodoc/non_publie.json");

fn fixture(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

#[allow(clippy::unnecessary_wraps)] // lisible dans les comparaisons avec `Option<Grade>`
fn grade(value: f64) -> Option<Grade> {
    Some(Grade {
        value,
        out_of: 20.0,
    })
}

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

// ───────────────────────────── notes ─────────────────────────────

#[test]
fn releve_du_semestre_courant() {
    let report = parse_grades(&fixture(PAYLOAD), Paris).unwrap();
    let [semester] = report.semesters.as_slice() else {
        panic!("un seul semestre attendu");
    };
    assert_eq!(semester.id, "901");
    assert_eq!(semester.name, "BUT Démo - Semestre 1");
    assert_eq!(semester.average, grade(13.96));
    assert_eq!(semester.rank.as_deref(), Some("12/56"));

    // UE : tri naturel, `~` sans moyenne, crédits ECTS du semestre.
    let units: Vec<_> = semester
        .units
        .iter()
        .map(|u| (u.code.as_str(), u.name.as_str(), u.average, u.ects))
        .collect();
    assert_eq!(
        units,
        [
            ("UE1.1", "Comprendre", grade(12.5), Some(5.0)),
            ("UE1.2", "Réaliser", None, Some(10.0)),
        ]
    );

    // Ressources puis SAÉ, `R1.2` avant `R1.10`, clés accentuées conservées.
    let codes: Vec<_> = semester.modules.iter().map(|m| m.code.as_str()).collect();
    assert_eq!(codes, ["R1.01", "R1.10", "SAÉ 1.01"]);
    assert_eq!(semester.modules[0].average, None);
    assert_eq!(semester.modules[2].average, grade(15.0));
    assert_eq!(semester.modules[2].name, "Réaliser un site web");
}

#[test]
fn evaluations_notes_mentions_et_dates() {
    let report = parse_grades(&fixture(PAYLOAD), Paris).unwrap();
    let evaluations = &report.semesters[0].modules[0].evaluations;
    let summary: Vec<_> = evaluations
        .iter()
        .map(|e| (e.id.as_str(), e.grade, e.status.as_deref(), e.coefficient))
        .collect();
    assert_eq!(
        summary,
        [
            ("7001", grade(12.5), None, Some(1.0)),
            ("7002", grade(14.25), None, Some(2.0)),
            ("7003", None, Some("ATT"), Some(1.0)),
            ("7004", None, Some("ABS"), Some(1.0)),
            ("7005", None, Some("EXC"), Some(1.0)),
            ("7006", None, None, Some(1.0)),
        ]
    );
    assert_eq!(evaluations[0].name, "Note TD");
    assert_eq!(evaluations[0].date, Some(day("2026-09-11")));
    assert_eq!(evaluations[5].name, "Évaluation 7006");
}

#[test]
fn aucune_donnee_personnelle_dans_le_releve() {
    let report = parse_grades(&fixture(PAYLOAD), Paris).unwrap();
    let debug = format!("{report:?}");
    for secret in ["Camille", "e00000000000", "0000000000A", "exemple.invalid"] {
        assert!(!debug.contains(secret), "{secret}");
    }
}

#[test]
fn valeurs_qui_ne_sont_pas_des_nombres() {
    let status = |s: &str| Cell::Status(s.to_owned());
    let cases = [
        (json!("12.50"), Cell::Value(12.5)),
        (json!("12,50"), Cell::Value(12.5)),
        (json!(" 07.00 "), Cell::Value(7.0)),
        (json!(14), Cell::Value(14.0)),
        (json!("0.00"), Cell::Value(0.0)),
        (json!("~"), status("ATT")),
        (json!("ABS"), status("ABS")),
        (json!("abs"), status("ABS")),
        (json!("EXC"), status("EXC")),
        (json!("ATT"), status("ATT")),
        (json!("NaN"), status("NAN")),
        (json!("inf"), status("INF")),
        (json!("-"), Cell::Empty),
        (json!(""), Cell::Empty),
        (json!("  "), Cell::Empty),
        (json!(null), Cell::Empty),
        (json!(true), Cell::Empty),
    ];
    for (value, expected) in cases {
        assert_eq!(cell(Some(&value)), expected, "{value}");
    }
    assert_eq!(cell(None), Cell::Empty);
}

#[test]
fn champs_manquants() {
    for data in [
        json!({}),
        json!({"relevé": null}),
        json!({"relevé": "texte"}),
        json!({"relevé": {"formsemestre_id": 1, "ues": "texte"}}),
        json!({"relevé": {"formsemestre_id": 1, "ressources": {"R1": "texte"}}}),
        json!({"relevé": {"formsemestre_id": 1, "ressources": {"R1": {"evaluations": [{"description": "sans id"}]}}}}),
        json!({"relevé": {"formsemestre_id": 1, "ressources": {"R1": {"evaluations": {"a": 1}}}}}),
        json!({"semestres": "texte", "relevé": {}}),
    ] {
        let error = parse_grades(&data, Paris).unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamChanged, "{data}");
    }

    // Le minimum utile : un identifiant de semestre.
    let report = parse_grades(&json!({"relevé": {"formsemestre_id": 5}}), Paris).unwrap();
    let semester = &report.semesters[0];
    assert_eq!(semester.name, "Semestre 5");
    assert_eq!(semester.average, None);
    assert_eq!(semester.rank, None);
    assert!(semester.units.is_empty() && semester.modules.is_empty());

    let data = json!({"relevé": {"formsemestre_id": "7", "semestre": {"numero": 3}}});
    assert_eq!(
        parse_grades(&data, Paris).unwrap().semesters[0].name,
        "Semestre 3"
    );
}

#[test]
fn rang_masque_ou_sans_total() {
    let rank = |value: Value| rank(Some(&value));
    assert_eq!(rank(json!({"value": "-", "total": 56})), None);
    assert_eq!(rank(json!({"value": "", "total": 56})), None);
    assert_eq!(
        rank(json!({"value": 3, "total": 56})).as_deref(),
        Some("3/56")
    );
    assert_eq!(
        rank(json!({"value": "3", "total": 0})).as_deref(),
        Some("3")
    );
    assert_eq!(rank(json!({"total": 56})), None);
}

#[test]
fn aucun_semestre_ou_releve_non_publie() {
    let report = parse_grades(&fixture(NO_SEMESTER), Paris).unwrap();
    assert!(report.semesters.is_empty());
    let report = parse_absences(&fixture(NO_SEMESTER), Paris).unwrap();
    assert!(report.absences.is_empty());
    assert_eq!((report.totals.justified, report.totals.unjustified), (0, 0));

    // Non publié : pas de `formsemestre_id`, le dernier semestre de la liste est le courant.
    let report = parse_grades(&fixture(UNPUBLISHED), Paris).unwrap();
    let semester = &report.semesters[0];
    assert_eq!(semester.id, "901");
    assert_eq!(semester.name, "BUT Démo - Semestre 2");
    assert!(semester.modules.is_empty() && semester.average.is_none());
}

#[test]
fn dates_d_evaluation_dans_le_fuseau_de_l_universite() {
    let date = |text: &str| evaluation_date(Some(&json!(text)), Paris);
    assert_eq!(date("2026-09-11T00:00:00+02:00"), Some(day("2026-09-11")));
    // 23 h 30 UTC la veille du passage à l'heure d'été : il est déjà minuit passé à Paris.
    assert_eq!(date("2026-03-28T23:30:00+00:00"), Some(day("2026-03-29")));
    // Après le passage à l'heure d'hiver (le 25 octobre), Paris est à UTC+1.
    assert_eq!(date("2026-10-25T22:30:00+00:00"), Some(day("2026-10-25")));
    assert_eq!(date("2026-10-24T22:30:00+00:00"), Some(day("2026-10-25")));
    // Sans fuseau : la date telle quelle.
    assert_eq!(date("2026-09-11"), Some(day("2026-09-11")));
    assert_eq!(date("2026-09-11T00:00:00"), Some(day("2026-09-11")));
    assert_eq!(date("n'importe quoi"), None);
    assert_eq!(evaluation_date(Some(&json!(12)), Paris), None);
    assert_eq!(evaluation_date(None, Paris), None);
}

// ──────────────────────────── absences ────────────────────────────

#[test]
fn absences_du_semestre() {
    let report = parse_absences(&fixture(PAYLOAD), Paris).unwrap();
    // Les retards et les présences ne sont pas des absences ; tri chronologique.
    let summary: Vec<_> = report
        .absences
        .iter()
        .map(|a| {
            (
                a.id.as_str(),
                a.start.to_rfc3339(),
                a.end.to_rfc3339(),
                a.justified,
                a.module.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (
                "79015",
                "2026-10-05T08:00:00+02:00".to_owned(),
                "2026-10-05T10:00:00+02:00".to_owned(),
                false,
                Some("Initiation au développement")
            ),
            // Après le passage à l'heure d'hiver ; module inconnu de ce relevé : pas de nom.
            (
                "79020",
                "2026-10-25T08:00:00+01:00".to_owned(),
                "2026-10-25T09:30:00+01:00".to_owned(),
                true,
                None
            ),
            (
                "79022",
                "2026-10-26T15:30:00+01:00".to_owned(),
                "2026-10-26T17:00:00+01:00".to_owned(),
                false,
                Some("Réaliser un site web")
            ),
        ]
    );
    assert_eq!(report.totals.justified, 1);
    assert_eq!(report.totals.unjustified, 2);
}

#[test]
fn heures_decimales_et_changement_d_heure() {
    let iso = |d: &str, h: f64| at(day(d), h, Paris).unwrap().to_rfc3339();
    assert_eq!(iso("2026-03-28", 23.0), "2026-03-28T23:00:00+01:00");
    assert_eq!(iso("2026-03-29", 3.0), "2026-03-29T03:00:00+02:00");
    // 2 h 30 n'existe pas le 29 mars 2026 : on avance d'une heure.
    assert_eq!(iso("2026-03-29", 2.5), "2026-03-29T03:30:00+02:00");
    // 2 h 30 existe deux fois le 25 octobre 2026 : la première.
    assert_eq!(iso("2026-10-25", 2.5), "2026-10-25T02:30:00+02:00");
    assert_eq!(iso("2026-10-25", 3.0), "2026-10-25T03:00:00+01:00");
    assert_eq!(iso("2026-10-05", 15.5), "2026-10-05T15:30:00+02:00");
    assert_eq!(iso("2026-10-05", 24.0), "2026-10-06T00:00:00+02:00");
    for hours in [-1.0, 24.5, f64::NAN, f64::INFINITY] {
        assert!(at(day("2026-10-05"), hours, Paris).is_err(), "{hours}");
    }
}

#[test]
fn absences_formes_et_champs_manquants() {
    let parse = |absences: Value| parse_absences(&json!({"absences": absences}), Paris);
    let entry = |extra: Value| {
        let mut base = json!({"debut": 8, "fin": 10, "statut": "absent"});
        base.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        base
    };

    // PHP envoie `[]` pour « aucun jour ».
    assert!(parse(json!([])).unwrap().absences.is_empty());
    assert!(
        parse(json!({"2026-10-05": []}))
            .unwrap()
            .absences
            .is_empty()
    );

    // Sans `idAbs` (passerelle sans ScoDoc assiduités) : identifiant déduit du créneau.
    let report =
        parse(json!({"2026-10-05": [entry(json!({"debut": "15,5", "fin": "17"}))]})).unwrap();
    assert_eq!(report.absences[0].id, "2026-10-05-15.5-17");
    assert_eq!(
        report.absences[0].start.to_rfc3339(),
        "2026-10-05T15:30:00+02:00"
    );

    // Statut en majuscules accepté, statut inconnu ignoré.
    let report = parse(json!({"2026-10-05": [
        entry(json!({"idAbs": 1, "statut": "ABSENT"})),
        entry(json!({"idAbs": 2, "statut": "excusé"})),
    ]}))
    .unwrap();
    assert_eq!(report.absences.len(), 1);

    // Justification : booléen, 0/1, chaîne ; absente = non justifiée.
    let report = parse(json!({"2026-10-05": [
        entry(json!({"idAbs": 1, "justifie": true})),
        entry(json!({"idAbs": 2, "justifie": 1})),
        entry(json!({"idAbs": 3, "justifie": "true"})),
        entry(json!({"idAbs": 4, "justifie": false})),
        entry(json!({"idAbs": 5})),
    ]}))
    .unwrap();
    assert_eq!(report.totals.justified, 3);
    assert_eq!(report.totals.unjustified, 2);

    // Absence sur plusieurs jours : une entrée, `dateFin` donne le dernier jour.
    let report = parse(json!({"2026-10-05": [
        entry(json!({"idAbs": 1, "debut": 22, "fin": 10, "dateFin": "2026-10-06", "matiereComplet": null}))
    ]}))
    .unwrap();
    assert_eq!(
        report.absences[0].end.to_rfc3339(),
        "2026-10-06T10:00:00+02:00"
    );
    assert_eq!(report.absences[0].module, None);

    // Champs obligatoires manquants ou illisibles.
    let without = |key: &str| {
        let mut e = entry(json!({"idAbs": 1}));
        e.as_object_mut().unwrap().remove(key);
        e
    };
    for key in ["statut", "debut", "fin"] {
        let error = parse(json!({"2026-10-05": [without(key)]})).unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamChanged, "{key}");
    }
    assert!(parse(json!({"hier": [entry(json!({}))]})).is_err());
    assert!(parse(json!({"2026-10-05": "texte"})).is_err());
    assert!(parse_absences(&json!({}), Paris).is_err());
    assert!(parse_absences(&json!({"absences": null}), Paris).is_err());
    assert!(parse(json!("texte")).is_err());
}

// ───────────────────── réponse et adresses du service ─────────────────────

#[test]
fn reponses_de_data_php() {
    assert!(matches!(
        classify_reply(json!({"redirect": "/services/doAuth.php"})),
        Ok(Reply::SessionLost)
    ));
    assert!(matches!(
        classify_reply(fixture(PAYLOAD)),
        Ok(Reply::Data(_))
    ));
    let error =
        classify_reply(json!({"erreur": "Relevé de Camille Demo indisponible"})).unwrap_err();
    assert_eq!(error.code, ErrorCode::ServiceUnavailable);
    assert!(!error.detail.unwrap().contains("Camille"));
    for other in [json!([1]), json!("texte"), json!(null)] {
        assert_eq!(
            classify_reply(other).unwrap_err().code,
            ErrorCode::UpstreamChanged
        );
    }
}

#[test]
fn tri_naturel() {
    let mut codes = ["R1.10", "R1.2", "R1.01", "SAÉ 1.2", "SAÉ 1.01"];
    codes.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(codes, ["R1.01", "R1.2", "R1.10", "SAÉ 1.01", "SAÉ 1.2"]);
    assert_eq!(natural_cmp("UE1.1", "UE1.1"), Ordering::Equal);
}

#[test]
fn adresses_du_service() {
    let config = |url: &str| ScoDocGateway {
        base_url: Url::parse(url).unwrap(),
        sign_in: SignIn::default(),
        options: ServiceOptions::default(),
    };
    let base = base_of(&config("https://notes.exemple.fr/scodoc?x=1#y"));
    assert_eq!(base.as_str(), "https://notes.exemple.fr/scodoc/");
    assert_eq!(
        data_url(&base).as_str(),
        "https://notes.exemple.fr/scodoc/services/data.php?q=dataPremi%C3%A8reConnexion"
    );
    assert_eq!(
        auth_url(&base).as_str(),
        "https://notes.exemple.fr/scodoc/services/doAuth.php?href=https%3A%2F%2Fnotes.exemple.fr%2Fscodoc%2F"
    );
    assert_eq!(token_key(&base), "scodoc.notes.exemple.fr");
    let local = Url::parse("http://127.0.0.1:8080/").unwrap();
    assert_eq!(token_key(&local), "scodoc.127.0.0.1:8080");
}

// ──────────── connexion au CAS de la composante (faux serveur) ────────────

use super::support;

fn university(server: &support::Server, sign_in: &Value) -> ResolvedUniversity {
    ResolvedUniversity {
        dir: PathBuf::new(),
        university: serde_json::from_value::<UniversityFile>(server.university(sign_in)).unwrap(),
        establishments: BTreeMap::new(),
    }
}

fn gateway(university: &ResolvedUniversity) -> ScoDocGateway {
    match &university.university.services.grades {
        Some(GradesService::ScodocGateway(gateway)) => gateway.clone(),
        other => panic!("fournisseur inattendu : {other:?}"),
    }
}

fn session_with(cookies: Vec<StoredCookie>) -> SessionData {
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

fn own_cas(server: &support::Server) -> Value {
    json!({
        "kind": "own-cas",
        "base-url": server.cas(),
        "flavor": "phpcas",
        "credentials": { "kind": "separate", "label": "Identifiant IUT (prenom.nom)" },
    })
}

fn univ_cookie() -> StoredCookie {
    StoredCookie {
        domain: "127.0.0.1".to_owned(),
        path: "/".to_owned(),
        name: "UNIV".to_owned(),
        value: "secret-universite".to_owned(),
        host_only: true,
        expires: None,
    }
}

macro_rules! context {
    ($ctx:ident, $university:expr, $session:expr) => {
        let mut $ctx = Context {
            university: $university,
            session: $session,
            now: Utc::now(),
            net: NetSettings { https_only: false },
        };
    };
}

#[tokio::test]
async fn connexion_puis_notes_sans_mot_de_passe_ni_cookies_de_l_universite() {
    let server = support::start(PAYLOAD).await;
    let university = university(&server, &own_cas(&server));
    let config = gateway(&university);
    let mut session = session_with(vec![univ_cookie()]);
    context!(ctx, &university, &mut session);

    sign_in(&mut ctx, &config, support::USERNAME, support::PASSWORD)
        .await
        .unwrap();
    let key = format!("scodoc.{}", server.authority());
    let token = ctx.session.tokens[&key].clone();
    assert_eq!(ctx.session.tokens.len(), 1);
    assert!(!token.contains(support::PASSWORD) && !token.contains(support::USERNAME));
    assert!(token.contains("PHPSESSID") && token.contains("CASTGC"));
    assert!(
        !token.contains("UNIV"),
        "cookies de l'université dans le jeton"
    );
    assert_eq!(ctx.session.cookies, [univ_cookie()]);
    {
        let state = server.state();
        assert_eq!(state.password_posts, 1);
        assert!(state.cas_cookies.iter().all(|c| !c.contains("UNIV")));
    }

    // Les notes s'obtiennent avec le seul jeton : pas de nouvelle connexion.
    let notes = fetch_grades(&mut ctx, &config).await.unwrap();
    assert_eq!(notes.semesters[0].average, grade(13.96));
    {
        let state = server.state();
        assert_eq!(state.ticket_validations, 1);
        assert_eq!(state.data_calls, 1);
    }

    // Session PHP expirée : retour silencieux par le CAS grâce au TGC du jeton.
    server.expire_sessions();
    let report = fetch_absences(&mut ctx, &config).await.unwrap();
    assert_eq!(report.absences.len(), 3);
    let state = server.state();
    assert_eq!(state.ticket_validations, 2);
    assert_eq!(
        state.password_posts, 1,
        "le mot de passe n'est posté qu'une fois"
    );
}

#[tokio::test]
async fn mauvais_mot_de_passe_un_seul_envoi_et_aucun_jeton() {
    let server = support::start(PAYLOAD).await;
    let university = university(&server, &own_cas(&server));
    let config = gateway(&university);
    let mut session = session_with(Vec::new());
    context!(ctx, &university, &mut session);

    let error = sign_in(&mut ctx, &config, support::USERNAME, "autre-mot-de-passe")
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidCredentials);
    assert!(ctx.session.tokens.is_empty());
    assert_eq!(server.state().password_posts, 1);
    assert_eq!(server.state().data_calls, 0);
}

#[tokio::test]
async fn connexion_separee_refusee_pour_un_service_sans_compte_propre() {
    let server = support::start(PAYLOAD).await;
    let university = university(&server, &json!({"kind": "university-cas"}));
    let config = gateway(&university);
    let mut session = session_with(Vec::new());
    context!(ctx, &university, &mut session);

    let error = sign_in(&mut ctx, &config, support::USERNAME, support::PASSWORD)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    let state = server.state();
    assert_eq!((state.password_posts, state.cas_forms), (0, 0));
}
