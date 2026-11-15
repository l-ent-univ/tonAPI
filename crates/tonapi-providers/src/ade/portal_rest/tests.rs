use super::parse::{MAX_DEPTH, parse_clock, parse_offset};
use super::*;
use chrono_tz::Europe::Paris;
use serde_json::json;

const EVENTS: &str = include_str!("../../../tests/fixtures/ade_portal_rest/events.json");
const EVENTS_DST: &str = include_str!("../../../tests/fixtures/ade_portal_rest/events_dst.json");
const TREE: &str = include_str!("../../../tests/fixtures/ade_portal_rest/resourcestree.json");
const SSOLOGIN: &str = include_str!("../../../tests/fixtures/ade_portal_rest/ssologin.json");
const SELECTION: &str = include_str!("../../../tests/fixtures/ade_portal_rest/selection.json");

fn json_of(raw: &str) -> Value {
    serde_json::from_str(raw).expect("fixture JSON")
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("date valide")
}

fn code<T>(result: Result<T, Problem>) -> ErrorCode {
    result.err().expect("une erreur était attendue").code
}

#[test]
fn evenements_cas_normal() {
    let courses = parse_events(&json_of(EVENTS), Paris, day(2026, 9, 1), day(2026, 9, 30)).unwrap();
    assert_eq!(courses.len(), 2, "doublon entre ressources retiré");
    let first = &courses[0];
    assert_eq!(first.id, "501");
    assert_eq!(first.title, "Développement web");
    assert_eq!(first.start.to_rfc3339(), "2026-09-02T08:00:00+02:00");
    assert_eq!(first.end.to_rfc3339(), "2026-09-02T10:00:00+02:00");
    assert_eq!(first.teachers, ["Mme Dupont"]);
    assert_eq!(first.rooms, ["Salle 12"]);
    assert_eq!(first.groups, ["TP 1"]);
    assert_eq!(first.kind, None);
    // Sans décalage : heure de l'université ; sans titre : « Cours » ; sans fin : durée nulle.
    let second = &courses[1];
    assert_eq!(second.id, "502");
    assert_eq!(second.title, "Cours");
    assert_eq!(second.start.to_rfc3339(), "2026-09-03T14:00:00+02:00");
    assert_eq!(second.end, second.start);
    assert!(second.rooms.is_empty() && second.teachers.is_empty());
}

#[test]
fn evenements_filtres_sur_la_plage() {
    let courses = parse_events(&json_of(EVENTS), Paris, day(2026, 9, 3), day(2026, 9, 3)).unwrap();
    assert_eq!(courses.len(), 1);
    assert_eq!(courses[0].id, "502");
}

#[test]
fn evenements_double_enveloppe_et_a_plat() {
    let nested = json!({"data": {"data": [{"id": 1, "events": [
        {"id": 7, "date": "02/09/2026", "startHour": "08:00+0200"}
    ]}]}});
    let flat = json!([{"id": 8, "date": "02/09/2026", "startHour": "09:00"}]);
    let a = parse_events(&nested, Paris, day(2026, 9, 2), day(2026, 9, 2)).unwrap();
    let b = parse_events(&flat, Paris, day(2026, 9, 2), day(2026, 9, 2)).unwrap();
    assert_eq!((a[0].id.as_str(), b[0].id.as_str()), ("7", "8"));
    assert!(
        parse_events(
            &json!({"data": []}),
            Paris,
            day(2026, 9, 2),
            day(2026, 9, 2)
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn evenement_champ_obligatoire_manquant() {
    let sans = |champ: &str| {
        let mut event = json!({"id": 1, "date": "02/09/2026", "startHour": "08:00+0200"});
        event.as_object_mut().unwrap().remove(champ);
        parse_events(&json!([event]), Paris, day(2026, 9, 1), day(2026, 9, 30))
    };
    for champ in ["id", "date", "startHour"] {
        assert_eq!(code(sans(champ)), ErrorCode::UpstreamChanged, "{champ}");
    }
    // `activityId` remplace `id`.
    let event = json!({"activityId": 42, "date": "02/09/2026", "startHour": "08:00"});
    let courses = parse_events(&json!([event]), Paris, day(2026, 9, 1), day(2026, 9, 30)).unwrap();
    assert_eq!(courses[0].id, "42");
}

#[test]
fn evenement_valeurs_illisibles() {
    for (date, hour) in [
        ("2026-09-02", "08:00"),
        ("02/09/2026", "8h"),
        ("02/09/2026", "25:00"),
        ("02/09/2026", "08:00+99"),
    ] {
        let event = json!({"id": 1, "date": date, "startHour": hour});
        assert_eq!(
            code(parse_events(
                &json!([event]),
                Paris,
                day(2026, 1, 1),
                day(2026, 12, 31)
            )),
            ErrorCode::UpstreamChanged,
            "{date} {hour}"
        );
    }
}

#[test]
fn passage_heure_d_ete_et_d_hiver() {
    let courses = parse_events(
        &json_of(EVENTS_DST),
        Paris,
        day(2026, 3, 1),
        day(2026, 11, 1),
    )
    .unwrap();
    let by_id = |id: &str| courses.iter().find(|c| c.id == id).expect(id);
    // Veille du passage à l'heure d'été : +01:00 ; le jour même, après 3 h : +02:00.
    assert_eq!(by_id("1").start.to_rfc3339(), "2026-03-28T10:00:00+01:00");
    assert_eq!(by_id("2").start.to_rfc3339(), "2026-03-29T10:00:00+02:00");
    assert_eq!(by_id("3").start.to_rfc3339(), "2026-03-29T03:00:00+02:00");
    // Passage à l'heure d'hiver (25 octobre) : 02:30 existe deux fois, la première est gardée.
    assert_eq!(by_id("4").start.to_rfc3339(), "2026-10-25T02:30:00+02:00");
    assert_eq!(by_id("5").start.to_rfc3339(), "2026-10-25T08:00:00+01:00");
    // Un décalage `Z` est converti à l'heure de Paris.
    assert_eq!(by_id("6").start.to_rfc3339(), "2026-10-25T08:00:00+01:00");
    // Une séance qui traverse le saut : 01:30+01:00 → 03:30+02:00, soit 60 minutes réelles.
    assert_eq!((by_id("7").end - by_id("7").start).num_minutes(), 60);
}

#[test]
fn heure_inexistante_sans_decalage() {
    let event = json!({"id": 1, "date": "29/03/2026", "startHour": "02:30"});
    let courses = parse_events(&json!([event]), Paris, day(2026, 3, 29), day(2026, 3, 29))
        .expect("heure sautée décalée d'une heure");
    let start = courses.first().expect("une séance").start;
    assert_eq!(start.to_rfc3339(), "2026-03-29T03:30:00+02:00");
}

#[test]
fn heure_sans_decalage_suit_le_fuseau_de_l_universite() {
    let event = json!({"id": 1, "date": "02/09/2026", "startHour": "08:00"});
    let tz: Tz = "America/Martinique".parse().unwrap();
    let courses = parse_events(&json!([event]), tz, day(2026, 9, 2), day(2026, 9, 2)).unwrap();
    assert_eq!(courses[0].start.to_rfc3339(), "2026-09-02T08:00:00-04:00");
}

#[test]
fn decalages_acceptes() {
    for (raw, secs) in [
        ("+0200", 7200),
        ("+02:00", 7200),
        ("-0330", -12600),
        ("+02", 7200),
    ] {
        assert_eq!(
            parse_offset(raw).map(|o| o.local_minus_utc()),
            Some(secs),
            "{raw}"
        );
    }
    assert_eq!(parse_offset("Z").map(|o| o.local_minus_utc()), Some(0));
    assert!(parse_offset("+2").is_none() && parse_offset("0200").is_none());
    assert!(parse_clock("08:00:30+0100").is_some());
}

#[test]
fn filtre_d_evenements_mois_d_abord() {
    assert_eq!(
        events_filter(14961, day(2026, 9, 2), day(2026, 9, 3)),
        "resources;14961/startdate;09022026/enddate;09032026/level;=8/field;Nom;CODEZ_RESOURCE;OWNER;"
    );
}

#[test]
fn ssologin_cas_normal_et_projet() {
    let tokens = parse_ssologin(&json_of(SSOLOGIN)).unwrap();
    // `data.projectId` vaut 0 : le projet est `authData.projectId`.
    assert_eq!(tokens.project, 1);
    assert_eq!(tokens.access, "jeton-api-synthetique");
    assert_eq!(
        tokens.refresh.as_deref(),
        Some("00000000-0000-4000-8000-000000000001")
    );
    // Forme `data.authData.authData`.
    let nested = json!({"data": {"authData": {"authData": {"projectId": 3, "accessToken": "a"}}}});
    let tokens = parse_ssologin(&nested).unwrap();
    assert_eq!((tokens.project, tokens.refresh), (3, None));
}

#[test]
fn ssologin_ressources_attribuees() {
    let attribuees = |value: Value| parse_ssologin(&value).unwrap().resources;
    let base = |extra: Value| {
        let mut auth = json!({"projectId": 1, "accessToken": "a"});
        if let (Some(auth), Some(extra)) = (auth.as_object_mut(), extra.as_object()) {
            auth.extend(extra.clone());
        }
        json!({"data": {"authData": auth}})
    };
    // Objets avec `id`, doublons et ids synthétiques négatifs écartés.
    assert_eq!(
        attribuees(base(
            json!({"resources": [{"id": 101, "name": "TD"}, {"id": 101}, {"id": -3}]})
        )),
        Some(vec![101])
    );
    // `resources` vide : l'entrée du projet courant dans `projectsResources`.
    assert_eq!(
        attribuees(base(json!({"resources": [], "projectsResources": [
            {"projectId": 2, "resources": [{"id": 7}]},
            {"projectId": 1, "resources": [{"id": 102}]},
        ]}))),
        Some(vec![102])
    );
    // Rien d'attribué : liste vide, pas une erreur.
    assert_eq!(attribuees(base(json!({}))), Some(vec![]));
}

#[test]
fn ssologin_incomplet() {
    for value in [
        json!({"data": {}}),
        json!({"data": {"authData": {"projectId": 1}}}),
        json!({"data": {"authData": {"accessToken": "a"}}}),
        json!({"data": {"authData": {"projectId": 1, "accessToken": " "}}}),
    ] {
        assert_eq!(code(parse_ssologin(&value)), ErrorCode::UpstreamChanged);
    }
}

#[test]
fn jeton_sso_vide_quand_pas_de_session() {
    let vide = json!({"token": "", "errMsg": "No session allowed!"});
    assert_eq!(parse_sso_token(&vide), None);
    assert_eq!(
        parse_sso_token(&json!({"token": "t"})).as_deref(),
        Some("t")
    );
    assert_eq!(
        parse_sso_token(&json!({"data": {"token": "t"}})).as_deref(),
        Some("t")
    );
}

#[test]
fn rafraichissement() {
    let (access, refresh) =
        parse_refresh(&json!({"data": {"accessToken": "a2", "refreshToken": "r2"}})).unwrap();
    assert_eq!((access.as_str(), refresh.as_deref()), ("a2", Some("r2")));
    assert!(parse_refresh(&json!({"data": {}})).is_none());
}

#[test]
fn selection_formats() {
    assert!(parse_selection(&json!({"data": ""})).unwrap().is_empty());
    assert!(parse_selection(&json!({"data": null})).unwrap().is_empty());
    assert_eq!(
        parse_selection(&json_of(SELECTION)).unwrap(),
        [14961, 14962]
    );
    assert_eq!(
        parse_selection(&json!({"data": "14961;14962 -1"})).unwrap(),
        [14961, 14962]
    );
    assert_eq!(
        parse_selection(&json!({"data": "[3,\"4\",3]"})).unwrap(),
        [3, 4]
    );
    assert_eq!(
        parse_selection(&json!({"data": [{"id": 5, "name": "2024"}]})).unwrap(),
        [5]
    );
    // Non vide mais sans id : un format inconnu, pas une sélection vide.
    assert_eq!(
        code(parse_selection(&json!({"data": "n/a"}))),
        ErrorCode::UpstreamChanged
    );
}

#[test]
fn groupes_de_l_arbre() {
    let groups = parse_groups(&json_of(TREE), &[14962]).unwrap();
    let names: Vec<_> = groups
        .iter()
        .map(|g| (g.name.as_str(), g.selected))
        .collect();
    // Enseignants et salles ignorés ; seules les feuilles d'étudiants sont proposées.
    assert_eq!(names, [("TP 1", false), ("TP 2", true), ("Amphi", false)]);
    assert_eq!(groups[0].id, "14961");
    assert_eq!(
        groups[0].path,
        [
            "Département Démo",
            "BUT Informatique 2",
            "Groupe TD 1",
            "TP 1"
        ]
    );
    // Une branche sélectionnée apparaît aussi.
    let groups = parse_groups(&json_of(TREE), &[111]).unwrap();
    assert!(groups.iter().any(|g| g.id == "111" && g.selected));
}

#[test]
fn noeud_au_nom_vide_ignore() {
    let tree = json!({"data": {"category": [{"category": "trainee", "branch": [
        {"id": 10, "name": "BUT Démo", "leaf": [
            {"id": 11, "name": ""},
            {"id": 12, "name": "  "},
            {"id": 13, "name": "TD 1"},
        ]},
        {"id": 20, "name": "", "leaf": [{"id": 21, "name": "TD 2"}]},
    ]}]}});
    let groups = parse_groups(&tree, &[]).unwrap();
    let found: Vec<_> = groups
        .iter()
        .map(|g| (g.id.as_str(), g.path.join(" / ")))
        .collect();
    assert_eq!(
        found,
        [
            ("13", "BUT Démo / TD 1".to_owned()),
            ("21", "TD 2".to_owned())
        ]
    );
}

#[test]
fn arbre_incomplet_ou_trop_profond() {
    let sans_nom = json!({"data": {"category": [{"category": "trainee", "leaf": [{"id": 1}]}]}});
    assert_eq!(
        code(parse_groups(&sans_nom, &[])),
        ErrorCode::UpstreamChanged
    );
    assert_eq!(
        code(parse_groups(&json!({"data": 3}), &[])),
        ErrorCode::UpstreamChanged
    );
    let mut node = json!({"id": 1, "name": "x"});
    for _ in 0..(MAX_DEPTH + 2) {
        node = json!({"id": 1, "name": "x", "branch": [node]});
    }
    let deep = json!({"data": {"category": [{"category": "trainee", "branch": [node]}]}});
    assert_eq!(code(parse_groups(&deep, &[])), ErrorCode::UpstreamChanged);
}

#[test]
fn erreurs_sans_donnees_de_l_etudiant() {
    let secret = json!({"id": 1, "date": "SECRET-123", "startHour": "08:00"});
    let error =
        parse_events(&json!([secret]), Paris, day(2026, 1, 1), day(2026, 12, 31)).unwrap_err();
    assert!(!error.to_string().contains("SECRET"));
}
