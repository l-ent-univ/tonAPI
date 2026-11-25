//! Tests du fournisseur `scodoc-gateway` : analyses pures sur les fixtures synthétiques, puis
//! connexion au CAS de la composante contre un faux serveur local.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{NaiveDate, Utc};
use chrono_tz::Europe::Paris;
use serde_json::json;
use tonapi_config::{GradesService, ResolvedUniversity, ServiceOptions, UniversityFile};
use tonapi_core::{DistributionUnavailable, EvaluationKind, Grade, GradeStats};
use tonapi_session::SessionData;

use super::parse::{Cell, at, cell, evaluation_date, natural_cmp, parse_report, rank};
use super::*;
use crate::NetSettings;

const PAYLOAD: &str = include_str!("../../tests/fixtures/scodoc/premiere_connexion.json");
const NO_SEMESTER: &str = include_str!("../../tests/fixtures/scodoc/sans_semestre.json");
const UNPUBLISHED: &str = include_str!("../../tests/fixtures/scodoc/non_publie.json");
const FULL: &str = include_str!("../../tests/fixtures/scodoc/releve_complet.json");

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

// notes

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
    assert_eq!(semester.published, Some(false));
    assert_eq!(semester.notice.as_deref(), Some("Relevé non publié."));
    assert_eq!(semester.department.as_deref(), Some("DEMO"));
    assert_eq!(semester.distributions, Some(false));
    assert_eq!(report.available.len(), 2);
}

// relevé complet : ce que lit une page de notes

#[allow(clippy::unnecessary_wraps)] // lisible dans les comparaisons avec `Option<GradeStats>`
fn stats(min: f64, mean: f64, max: f64) -> Option<GradeStats> {
    Some(GradeStats {
        min: Some(min),
        mean: Some(mean),
        max: Some(max),
    })
}

#[test]
fn semestre_complet_identite_promotion_rangs_et_jury() {
    let report = parse_grades(&fixture(FULL), Paris).unwrap();

    // Tous les semestres, du plus ancien au plus récent ; celui sans identifiant est écarté.
    let listed: Vec<_> = report
        .available
        .iter()
        .map(|s| (s.id.as_str(), s.name.as_str(), s.number, s.year.as_deref()))
        .collect();
    assert_eq!(
        listed,
        [
            ("880", "BUT Démo", Some(1), Some("2025/2026")),
            ("890", "BUT Démo", Some(2), Some("2025/2026")),
            ("901", "Semestre 3", Some(3), Some("2026/2027")),
        ]
    );

    let semester = &report.semesters[0];
    assert_eq!(
        (semester.id.as_str(), semester.name.as_str()),
        ("901", "Semestre 3")
    );
    assert_eq!(semester.number, Some(3));
    assert_eq!(semester.year.as_deref(), Some("2026 - 2027"));
    assert_eq!(semester.start, Some(day("2026-09-01")));
    assert_eq!(semester.end, Some(day("2027-01-31")));
    assert_eq!(semester.kind.as_deref(), Some("BUT"));
    assert_eq!(semester.program.as_deref(), Some("BUT DEMO"));
    // Sans `dept_acronym` : le département de l'adresse de la photo, et rien d'autre de celle-ci.
    assert_eq!(semester.department.as_deref(), Some("DEMO"));
    assert_eq!(semester.published, Some(true));
    assert_eq!(semester.notice, None);
    assert_eq!(
        semester.updated_at.map(|at| at.to_rfc3339()).as_deref(),
        Some("2026-11-12T10:58:26+01:00")
    );
    assert_eq!(semester.average, grade(12.1));
    assert_eq!(semester.stats, stats(6.0, 11.5, 16.8));
    assert_eq!(semester.rank.as_deref(), Some("12/84"));
    assert_eq!(
        (semester.ects, semester.ects_earned),
        (Some(30.0), Some(5.0))
    );
    assert_eq!(semester.transcript, Some(true));
    assert_eq!(semester.distributions, Some(true));

    // Groupes : objets ou simples noms, sans doublon ni groupe sans nom.
    let groups: Vec<_> = semester
        .groups
        .iter()
        .map(|g| (g.id.as_deref(), g.name.as_str(), g.partition.as_deref()))
        .collect();
    assert_eq!(
        groups,
        [
            (Some("61"), "TD B", Some("TD")),
            (Some("62"), "TP B1", Some("TP")),
            (None, "Parcours A", None),
        ]
    );
    // Rangs de groupe : nommés par le groupe, `[place, effectif]` accepté, rang masqué écarté.
    let ranks: Vec<_> = semester
        .group_ranks
        .iter()
        .map(|r| (r.group.as_str(), r.rank.as_str()))
        .collect();
    assert_eq!(
        ranks,
        [("TD B", "3/28"), ("TP B1", "2 ex/14"), ("Apprentis", "5")]
    );

    let decision = semester.decision.as_ref().unwrap();
    assert_eq!(
        (decision.code.as_str(), decision.date),
        ("ADM", Some(day("2027-02-04")))
    );
    let year = semester.year_decision.as_ref().unwrap();
    assert_eq!((year.code.as_str(), year.date), ("RED", None));

    let debug = format!("{report:?}");
    for secret in [
        "Camille",
        "e00000000000",
        "exemple.invalid",
        "4242",
        "Inscrit",
    ] {
        assert!(!debug.contains(secret), "{secret}");
    }
}

#[test]
fn unites_couleur_bonus_capitalisation_jury_et_modules() {
    let report = parse_grades(&fixture(FULL), Paris).unwrap();
    let [first, second, third] = report.semesters[0].units.as_slice() else {
        panic!("trois UE attendues");
    };

    assert_eq!(first.code, "UE3.1");
    assert_eq!(first.average, grade(13.5));
    assert_eq!(first.stats, stats(5.0, 11.0, 17.0));
    assert_eq!(first.rank.as_deref(), Some("4 ex/84"));
    assert_eq!(first.color.as_deref(), Some("#B80004"));
    assert_eq!((first.ects, first.ects_earned), (Some(5.0), Some(5.0)));
    assert_eq!((first.bonus, first.malus), (Some(0.25), None));
    assert!(!first.capitalized);
    assert_eq!(first.decision.as_deref(), Some("ADM"));
    // Modules de l'UE : coefficient et moyenne du module dans cette UE (nombre ou `{value}`).
    let links: Vec<_> = first
        .modules
        .iter()
        .map(|m| (m.code.as_str(), m.coefficient, m.average))
        .collect();
    assert_eq!(
        links,
        [
            ("R3.01", Some(4.0), grade(13.4)),
            ("SAÉ 3.01", Some(8.0), grade(14.0)),
        ]
    );

    // Couleur illisible écartée ; `~` sans statistique ; décision retrouvée par l'acronyme.
    assert_eq!(second.color, None);
    assert_eq!(second.stats, None);
    assert_eq!(second.rank, None);
    assert_eq!((second.bonus, second.malus), (None, Some(0.5)));
    assert_eq!((second.ects, second.ects_earned), (Some(10.0), Some(0.0)));
    assert_eq!(second.decision.as_deref(), Some("AJ"));
    assert_eq!(second.modules[0].coefficient, Some(0.0));
    assert_eq!(second.modules[0].average, None);

    // Capitalisée d'après `ues_capitalisees`, sans moyenne ni module ce semestre.
    assert!(third.capitalized);
    assert_eq!((third.average, third.ects), (None, Some(0.0)));
    assert!(third.modules.is_empty() && third.decision.is_none());
}

#[test]
fn evaluations_type_poids_promotion_et_horaires() {
    let report = parse_grades(&fixture(FULL), Paris).unwrap();
    let modules = &report.semesters[0].modules;
    let kinds: Vec<_> = modules
        .iter()
        .map(|m| (m.code.as_str(), m.id.as_deref(), m.kind.as_deref()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("R3.01", Some("5301"), Some("ressource")),
            ("SAÉ 3.01", Some("5401"), Some("sae")),
        ]
    );

    let [test, resit, second_session, bonus] = modules[0].evaluations.as_slice() else {
        panic!("quatre évaluations attendues");
    };
    // `date_debut` prime sur `date` ; l'heure connue donne le début et la fin de l'épreuve.
    assert_eq!(test.kind, EvaluationKind::Normal);
    assert_eq!(test.date, Some(day("2026-10-02")));
    assert_eq!(
        test.start.map(|at| at.to_rfc3339()).as_deref(),
        Some("2026-10-02T08:15:00+02:00")
    );
    assert_eq!(
        test.end.map(|at| at.to_rfc3339()).as_deref(),
        Some("2026-10-02T10:15:00+02:00")
    );
    assert_eq!(test.stats, stats(2.5, 11.4, 19.0));
    // Poids par UE, nombre ou chaîne ; un poids vide est ignoré.
    let weights: Vec<_> = test.weights.iter().map(|(u, w)| (u.as_str(), *w)).collect();
    assert_eq!(weights, [("UE3.1", 1.0), ("UE3.2", 0.5)]);

    // Rattrapage sans coefficient ; minuit n'est pas une heure d'épreuve.
    assert_eq!(resit.kind, EvaluationKind::Resit);
    assert_eq!((resit.coefficient, resit.stats), (None, None));
    assert_eq!(resit.date, Some(day("2026-10-20")));
    assert_eq!((resit.start, resit.end), (None, None));

    assert_eq!(second_session.kind, EvaluationKind::SecondSession);
    assert_eq!(second_session.status.as_deref(), Some("NP"));
    assert_eq!(second_session.date, None);
    assert!(second_session.weights.is_empty());

    assert_eq!(bonus.kind, EvaluationKind::Bonus);
    assert_eq!(bonus.grade, grade(0.5));
}

#[test]
fn ce_que_l_etablissement_masque_n_est_pas_rendu() {
    let mut data = fixture(FULL);
    data["relevé"]["options"] =
        json!({ "show_rangs": false, "show_ue_rangs": 0, "show_ects": false });
    let report = parse_grades(&data, Paris).unwrap();
    let semester = &report.semesters[0];
    assert_eq!(semester.rank, None);
    assert!(semester.group_ranks.is_empty());
    assert_eq!((semester.ects, semester.ects_earned), (None, None));
    let unit = &semester.units[0];
    assert_eq!(unit.rank, None);
    // Le poids de l'UE (ses crédits) reste : il sert au calcul de la moyenne générale.
    assert_eq!((unit.ects, unit.ects_earned), (Some(5.0), None));
}

#[test]
fn releve_pdf_et_repartitions_selon_la_passerelle() {
    let read = |change: &dyn Fn(&mut Value)| {
        let mut data = fixture(FULL);
        change(&mut data);
        let semester = parse_grades(&data, Paris).unwrap().semesters.remove(0);
        (semester.transcript, semester.distributions)
    };
    assert_eq!(read(&|_| {}), (Some(true), Some(true)));
    // PDF : coupé, réservé à d'autres départements, ou ouvert à tous par une liste vide.
    assert_eq!(
        read(&|d| d["config"]["releve_PDF"] = json!(false)).0,
        Some(false)
    );
    assert_eq!(
        read(&|d| d["config"]["liste_dep_publi_PDF"] = json!("AUTRE")).0,
        Some(false)
    );
    assert_eq!(
        read(&|d| d["config"]["liste_dep_publi_PDF"] = json!(" , ")).0,
        Some(true)
    );
    // Répartitions : coupées par la passerelle, ou relevé non publié.
    assert_eq!(
        read(&|d| d["config"]["histogramme"] = json!(false)).1,
        Some(false)
    );
    assert_eq!(
        read(&|d| d["relevé"]["publie"] = json!(false)).1,
        Some(false)
    );
    // Sans `config` (relevé d'un autre semestre) : inconnu.
    assert_eq!(read(&|d| d["config"] = Value::Null), (None, None));
}

#[test]
fn releve_d_un_semestre_demande() {
    // `relevéEtudiant` : ni `config`, ni `semestres`.
    let mut report = fixture(FULL)["relevé"].clone();
    let data = json!({ "relevé": report.clone(), "absences": [] });
    let parsed = parse_report(&data, Paris, Some("890")).unwrap();
    assert!(parsed.available.is_empty());
    // Le relevé dit son semestre : il prime sur celui de la demande.
    assert_eq!(parsed.semesters[0].id, "901");
    assert_eq!(parsed.semesters[0].name, "Semestre 3");
    assert_eq!(parsed.semesters[0].year.as_deref(), Some("2026 - 2027"));

    // Relevé qui ne dit pas son semestre (non publié) : celui de la demande.
    report.as_object_mut().unwrap().remove("formsemestre_id");
    report["publie"] = json!(false);
    report["message"] = json!("<p>Relevé <b>non publié</b>.</p>");
    let parsed = parse_report(&json!({ "relevé": report }), Paris, Some("890")).unwrap();
    let semester = &parsed.semesters[0];
    assert_eq!(semester.id, "890");
    assert_eq!(semester.published, Some(false));
    assert_eq!(semester.notice.as_deref(), Some("Relevé non publié."));
}

// autour du relevé : répartition, message, PDF

#[test]
fn repartition_des_notes_par_point_entier() {
    // Notes sur 20, dans le désordre : 20 dans la dernière case, 12,999999999 compté comme 13.
    let grades = json!([
        0,
        0.99,
        7.5,
        12.999_999_999_9,
        13.0,
        19.99,
        20,
        20.0,
        25,
        -1
    ]);
    let distribution = parse_distribution(&grades).unwrap();
    assert_eq!(distribution.unavailable, None);
    assert_eq!((distribution.total, distribution.out_of), (10, 20.0));
    assert_eq!(distribution.counts.len(), 21);
    let filled: Vec<_> = distribution
        .counts
        .iter()
        .enumerate()
        .filter(|(_, count)| **count > 0)
        .map(|(bucket, count)| (bucket, *count))
        .collect();
    assert_eq!(filled, [(0, 3), (7, 1), (13, 2), (19, 1), (20, 3)]);
}

#[test]
fn repartition_refusee_ou_illisible() {
    let reason = |value: Value| parse_distribution(&value).unwrap().unavailable;
    assert_eq!(
        reason(json!(["too low"])),
        Some(DistributionUnavailable::TooFew)
    );
    assert_eq!(reason(json!([])), Some(DistributionUnavailable::TooFew));
    assert_eq!(
        reason(json!([10.0, 11.0, 12.0, 13.0, 14.0])),
        Some(DistributionUnavailable::TooFew)
    );
    assert_eq!(
        reason(json!({ "erreur": "L'histogramme n'est pas activé" })),
        Some(DistributionUnavailable::Disabled)
    );
    let hidden = hidden_distribution();
    assert_eq!(hidden.unavailable, Some(DistributionUnavailable::Hidden));
    assert!(hidden.counts.is_empty() && hidden.total == 0);

    // Autre erreur, valeur non numérique, tableau démesuré : format inattendu, sans son contenu.
    for value in [
        json!({ "erreur": "SECRET de l'établissement" }),
        json!([10.0, "12", 13.0, 14.0, 15.0, 16.0]),
        json!([10.0, null, 13.0, 14.0, 15.0, 16.0]),
        Value::Array(vec![json!(10.0); 5001]),
        json!("texte"),
    ] {
        let problem = parse_distribution(&value).unwrap_err();
        assert_eq!(problem.code, ErrorCode::UpstreamChanged);
        assert!(!problem.detail.unwrap_or_default().contains("SECRET"));
    }
}

#[test]
fn evaluation_du_releve_et_publication() {
    let data = fixture(FULL);
    assert!(owns_evaluation(&data, "7301"));
    assert!(owns_evaluation(&data, "7304"));
    // Une évaluation d'ailleurs, l'identifiant d'un module, un relevé sans évaluations.
    assert!(!owns_evaluation(&data, "9999"));
    assert!(!owns_evaluation(&data, "5301"));
    assert!(!owns_evaluation(&fixture(UNPUBLISHED), "7301"));
    assert!(!owns_evaluation(&json!({}), "7301"));

    assert!(is_published(&data));
    assert!(!is_published(&fixture(UNPUBLISHED)));
    // Département masqué : la clé `publie` manque.
    assert!(!is_published(&fixture(NO_SEMESTER)));
}

#[test]
fn message_du_departement_en_texte() {
    let html = "<p>Jury le <b>12&nbsp;f&eacute;vrier</b>.</p>\n<ul><li>Salle A</li><li> Apportez\n votre carte</li></ul>Bon courage&nbsp;!<br><br>";
    let notice = parse_notice(&json!({ "message": html }));
    // Une ligne par paragraphe, les éléments de liste marqués, les entités françaises décodées.
    assert_eq!(
        notice.message.as_deref(),
        Some("Jury le 12 février.\n• Salle A\n• Apportez votre carte\nBon courage !")
    );
    for empty in [
        json!({ "message": "" }),
        json!({ "message": "<p> </p>" }),
        json!({}),
        json!({ "message": 3 }),
    ] {
        assert_eq!(parse_notice(&empty).message, None);
    }
}

#[test]
fn identifiant_de_l_etudiant_et_type_du_releve_pdf() {
    assert_eq!(student_id(&fixture(FULL)).as_deref(), Some("e00000000000"));
    for bad in [
        json!({}),
        json!({ "auth": {} }),
        json!({ "auth": { "session": "" } }),
        json!({ "auth": { "session": "a/../b" } }),
        json!({ "auth": { "session": "x".repeat(65) } }),
        json!({ "auth": { "session": 12 } }),
    ] {
        assert_eq!(student_id(&bad), None, "{bad}");
    }
    assert_eq!(transcript_type(&fixture(FULL)), "BUT");
    assert_eq!(
        transcript_type(&json!({ "relevé": { "type": "DUT" } })),
        "DUT"
    );
    assert_eq!(
        transcript_type(&json!({ "relevé": { "type": "LP" } })),
        "BUT"
    );
    assert_eq!(transcript_type(&json!({})), "BUT");

    let base = Url::parse("https://notes.exemple.fr/scodoc/").unwrap();
    assert_eq!(
        transcript_url(&base, "BUT", "901", "e0@x").as_str(),
        "https://notes.exemple.fr/scodoc/services/bulletin_PDF.php?type=BUT&sem_id=901&etudiant=e0%40x"
    );
    let query = Query {
        action: "relevéEtudiant",
        params: &[("semestre", "890")],
    };
    assert_eq!(
        data_url(&base, &query).as_str(),
        "https://notes.exemple.fr/scodoc/services/data.php?q=relev%C3%A9Etudiant&semestre=890"
    );
    // Un identifiant ScoDoc est un entier : rien d'autre ne part dans une adresse.
    assert_eq!(scodoc_id(" 901 ", "de semestre").unwrap(), "901");
    for bad in ["", "9a", "-1", "1/2", "12345678901", "1&x=2"] {
        assert_eq!(
            scodoc_id(bad, "de semestre").unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
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

// absences

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

// réponse et adresses du service

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
        data_url(&base, &FIRST_CONNECTION).as_str(),
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

// connexion au CAS de la composante (faux serveur)

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
    let notes = fetch_grades(&mut ctx, &config, None).await.unwrap();
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
