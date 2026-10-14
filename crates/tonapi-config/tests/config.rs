//! Tests sur les vrais fichiers de `universities/` et sur les fixtures invalides. Aucun accès réseau.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tonapi_config::{
    GradesService, Registry, establishment_schema, establishment_template, university_schema,
};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn invalid() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/invalid")
}

#[test]
fn les_universites_du_depot_se_chargent_sans_refus() {
    let (registry, report) = Registry::load(&repo().join("universities"));
    let problems: Vec<String> = report.rejected.iter().map(ToString::to_string).collect();
    assert!(problems.is_empty(), "{problems:#?}");
    for id in ["demo", "umontpellier", "univ-rennes"] {
        assert!(registry.get(id).is_some(), "{id} absente");
    }
}

#[test]
fn chaque_composante_recoit_ses_services() {
    let (registry, _) = Registry::load(&repo().join("universities"));
    let um = registry.get("umontpellier").unwrap();

    let beziers = um.services_for(Some("iut-beziers"));
    assert!(matches!(
        beziers.grades,
        Some(GradesService::ScodocGateway(_))
    ));
    assert_eq!(beziers.timetable, um.university.services.timetable);

    let fac = um.services_for(Some("fac"));
    assert!(matches!(fac.grades, Some(GradesService::Link(_))));
    assert_eq!(
        um.default_establishment().map(|e| e.id.as_str()),
        Some("fac")
    );

    let rennes = registry.get("univ-rennes").unwrap();
    let lannion = rennes.services_for(Some("iut-lannion"));
    assert!(matches!(
        lannion.grades,
        Some(GradesService::ScodocGateway(_))
    ));
    assert_eq!(rennes.services_for(Some("autres")).grades, None);
}

#[test]
fn chaque_fixture_invalide_est_refusee() {
    let (registry, report) = Registry::load(&invalid());
    let refused = |path: &str| report.rejected.iter().any(|i| i.file.ends_with(path));
    for dir in [
        "fournisseur-inconnu",
        "http",
        "parametre-manquant",
        "id-different",
        "champ-inconnu",
        "a-remplir",
    ] {
        assert!(refused(&format!("{dir}/university.toml")), "{dir} accepté");
        assert!(registry.get(dir).is_none());
    }
    for file in ["b-defaut", "regle-vide", "portail-absent"] {
        assert!(
            refused(&format!("composantes/establishments/{file}.toml")),
            "{file} accepté"
        );
    }
    // Une composante refusée n'écarte qu'elle.
    let composantes = registry.get("composantes").expect("université valide");
    assert_eq!(
        composantes.establishments.keys().collect::<Vec<_>>(),
        ["a-defaut"]
    );
}

#[test]
fn une_faute_de_frappe_donne_la_ligne_et_la_suggestion() {
    let (_, report) = Registry::load(&invalid());
    let issue = report
        .rejected
        .iter()
        .find(|i| i.file.ends_with("fournisseur-inconnu/university.toml"))
        .unwrap();
    assert_eq!(issue.line, Some(9));
    assert_eq!(issue.suggestion.as_deref(), Some("scodoc-gateway"));
}

#[test]
fn le_schema_commite_est_a_jour() {
    for (name, schema) in [
        ("university.schema.json", university_schema()),
        ("establishment.schema.json", establishment_schema()),
    ] {
        let committed = fs::read_to_string(repo().join("schema").join(name)).unwrap();
        let current = serde_json::to_string_pretty(&schema).unwrap() + "\n";
        assert!(
            committed == current,
            "schema/{name} n'est pas à jour : lancez `cargo run -p tonapi-cli -- schema`"
        );
    }
}

#[test]
fn les_fichiers_du_depot_respectent_le_json_schema() {
    let university = jsonschema::validator_for(&university_schema()).unwrap();
    let establishment = jsonschema::validator_for(&establishment_schema()).unwrap();
    let mut checked = 0;
    for univ in fs::read_dir(repo().join("universities")).unwrap() {
        let dir = univ.unwrap().path();
        let mut files = vec![(dir.join("university.toml"), &university)];
        if let Ok(entries) = fs::read_dir(dir.join("establishments")) {
            files.extend(entries.map(|e| (e.unwrap().path(), &establishment)));
        }
        for (file, validator) in files {
            let value: Value = toml::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
            let errors: Vec<String> = validator
                .iter_errors(&value)
                .map(|e| format!("{}: {e}", e.instance_path()))
                .collect();
            assert!(errors.is_empty(), "{}: {errors:#?}", file.display());
            checked += 1;
        }
    }
    assert!(checked >= 9);
}

/// Parcours de quelqu'un qui ajoute un IUT : modèle → deux valeurs à remplir → accepté.
#[test]
fn ajouter_un_iut_tient_en_un_petit_fichier() {
    let tmp = tempfile::tempdir().unwrap();
    let univ = tmp.path().join("demo");
    fs::create_dir_all(univ.join("establishments")).unwrap();
    fs::copy(
        repo().join("universities/demo/university.toml"),
        univ.join("university.toml"),
    )
    .unwrap();

    let template = establishment_template("demo", "iut-test");
    let file = univ.join("establishments/iut-test.toml");

    // Le modèle brut est refusé tant qu'il reste des valeurs à remplir.
    fs::write(&file, &template).unwrap();
    let (_, report) = Registry::load(tmp.path());
    assert!(!report.is_ok());

    let filled = template
        .replace("name = \"<à remplir>\"", "name = \"IUT de Test\"")
        .replace("# [services.grades]", "[services.grades]")
        .replace(
            "# provider = \"scodoc-gateway\"\n# base-url = \"https://notes.iut.exemple.fr/\"\n#",
            "provider = \"scodoc-gateway\"\nbase-url = \"https://notes.iut-test.fr/\"\n",
        );
    fs::write(&file, &filled).unwrap();
    let (registry, report) = Registry::load(tmp.path());
    let problems: Vec<String> = report.rejected.iter().map(ToString::to_string).collect();
    assert!(problems.is_empty(), "{problems:#?}\n{filled}");

    let services = registry.get("demo").unwrap().services_for(Some("iut-test"));
    assert!(matches!(
        services.grades,
        Some(GradesService::ScodocGateway(_))
    ));

    let useful = filled
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .count();
    assert!(useful < 20, "{useful} lignes utiles");
}
