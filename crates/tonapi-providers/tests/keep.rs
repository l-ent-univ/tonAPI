//! Durée pendant laquelle un client peut garder la réponse d'un service (`meta.cacheSecs`) :
//! réglage du TOML, défaut par service, et rien pour ce qui ne vient pas d'un service distant.

mod common;

use serde_json::json;
use tonapi_config::ServiceKind;
use tonapi_providers::keep_secs;

fn services(services: &serde_json::Value) -> tonapi_config::Services {
    common::university(json!({
        "id": "test",
        "name": "Université de test",
        "sso": { "kind": "cas", "base-url": "https://cas.exemple.test/" },
        "portal": { "kind": "uportal5", "base-url": "https://ent.exemple.test/" },
        "services": services,
    }))
    .services_for(None)
}

#[test]
fn defaut_par_service_pour_un_fournisseur_distant() {
    let services = services(&json!({
        "timetable": { "provider": "ade-portal-rest", "base-url": "https://ade.exemple.test/portal/" },
        "grades": { "provider": "scodoc-gateway", "base-url": "https://notes.exemple.test/" },
        "absences": { "provider": "scodoc-gateway", "base-url": "https://notes.exemple.test/" },
        "homework": { "provider": "moodle-ajax", "base-url": "https://moodle.exemple.test/" },
        "mail": { "provider": "zimbra-rest", "base-url": "https://mail.exemple.test/" },
        "exams": { "provider": "etudiants-api", "base-url": "https://scol.exemple.test/" },
        "apps": { "provider": "portal" },
        "profile": { "provider": "portal" },
    }));
    let kept: Vec<_> = ServiceKind::ALL
        .iter()
        .map(|kind| (kind.key(), keep_secs(&services, *kind)))
        .collect();
    assert_eq!(
        kept,
        [
            ("timetable", Some(300)),
            ("grades", Some(600)),
            ("absences", Some(600)),
            ("exams", Some(600)),
            ("homework", Some(300)),
            ("mail", Some(60)),
            ("apps", Some(3600)),
            ("profile", Some(3600)),
            // Aucun service de documents dans cette configuration.
            ("documents", None),
        ]
    );
}

#[test]
fn reglage_du_toml_zero_et_plafond() {
    let with = |ttl: u32| {
        services(&json!({
            "grades": {
                "provider": "scodoc-gateway",
                "base-url": "https://notes.exemple.test/",
                "options": { "cache-ttl-secs": ttl },
            },
        }))
    };
    assert_eq!(keep_secs(&with(90), ServiceKind::Grades), Some(90));
    // `0` : ne jamais garder. Une durée démesurée est ramenée à un jour.
    assert_eq!(keep_secs(&with(0), ServiceKind::Grades), None);
    assert_eq!(
        keep_secs(&with(4_000_000), ServiceKind::Grades),
        Some(86_400)
    );
}

#[test]
fn rien_a_garder_sans_service_distant() {
    let services = services(&json!({
        "timetable": { "provider": "demo" },
        "grades": { "provider": "link", "url": "https://notes.exemple.test/" },
        "homework": { "provider": "none" },
    }));
    for kind in [
        ServiceKind::Timetable,
        ServiceKind::Grades,
        ServiceKind::Homework,
        ServiceKind::Mail,
    ] {
        assert_eq!(keep_secs(&services, kind), None, "{}", kind.key());
    }
}
