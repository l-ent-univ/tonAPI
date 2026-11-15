//! Rattachement automatique à une composante, sur les fichiers réels de `universities/`.

use std::path::Path;

use tonapi_config::{Registry, Signal, SignalSource};
use tonapi_core::EstablishmentSource;
use tonapi_providers::detect_establishment;

fn signal(source: SignalSource, value: &str) -> Signal {
    Signal {
        source,
        value: value.to_owned(),
    }
}

#[test]
fn un_etudiant_de_l_iut_de_beziers_est_reconnu() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../universities");
    let (registry, _) = Registry::load(&dir);
    let um = registry.get("umontpellier").unwrap();

    let pick = detect_establishment(
        um,
        &[
            signal(SignalSource::EnrollmentComponent, "I.U.T. de Béziers"),
            signal(SignalSource::PostalAddress, "3 place du Test$34500$BEZIERS"),
        ],
    )
    .unwrap();
    assert_eq!(pick.id, "iut-beziers");
    assert_eq!(pick.source, EstablishmentSource::Detected);
    assert!(pick.confidence > 0.0 && pick.confidence <= 1.0);

    // Aucun signal reconnu : pas de détection, la composante par défaut reste.
    assert!(detect_establishment(um, &[]).is_none());
    assert!(
        detect_establishment(
            um,
            &[signal(SignalSource::EnrollmentComponent, "Faculté X")]
        )
        .is_none()
    );
}

#[test]
fn un_etudiant_de_l_iut_de_lannion_est_reconnu_par_son_groupe_ade() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../universities");
    let (registry, _) = Registry::load(&dir);
    let rennes = registry.get("univ-rennes").unwrap();

    // Chemin du groupe dans l'arbre du portail ADE, tel que `ade-portal-rest` le fournit.
    let pick = detect_establishment(
        rennes,
        &[signal(
            SignalSource::AdeGroupPath,
            "IUT Lannion › BUT Démo › Démo 2 › TD 1",
        )],
    )
    .unwrap();
    assert_eq!(pick.id, "iut-lannion");
    assert_eq!(pick.source, EstablishmentSource::Detected);

    // Un groupe d'une autre composante laisse la composante par défaut.
    assert!(
        detect_establishment(
            rennes,
            &[signal(
                SignalSource::AdeGroupPath,
                "Droit › Licence 1 › TD 3"
            )]
        )
        .is_none()
    );
}
