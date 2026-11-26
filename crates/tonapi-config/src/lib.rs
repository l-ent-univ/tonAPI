//! Configuration des universités servies par tonAPI.
//!
//! Une université est un dossier `universities/<id>/` qui contient `university.toml` et, si elle a des
//! composantes servies différemment (IUT, UFR), un fichier par composante dans `establishments/`.
//! [`Registry::load`] lit tout le dossier, valide chaque fichier et refuse un fichier invalide en entier.
//!
//! Les fournisseurs portent le nom d'un logiciel (`scodoc-gateway`, `ade-portal-rest`…), jamais celui d'une
//! université : ajouter une composante qui utilise un logiciel déjà géré ne demande qu'un fichier TOML.

mod affiliation;
mod catalog;
mod diagnostics;
mod establishment;
mod load;
mod schema;
mod services;
mod templates;
mod university;
mod validate;

pub use affiliation::{Affiliation, MatchRule, Signal, SignalSource, normalize};
pub use catalog::{ProviderInfo, providers};
pub use establishment::EstablishmentFile;
pub use load::{LoadReport, Registry, ResolvedUniversity};
pub use schema::{establishment_schema, university_schema};
pub use services::{
    AbsencesService, AppsService, Credentials, DocumentsService, ExamsService, GradesService,
    HomeworkService, LinkService, MailService, MoodleEntry, ProfileService, ScoDocGateway,
    ServiceAvailability, ServiceKind, ServiceOptions, Services, SignIn, StudentRecord,
    TimetableService,
};
pub use templates::{PLACEHOLDER, PLACEHOLDER_HOST, establishment_template, university_template};
pub use university::{
    CasFlavor, CasSso, DrupalCatalogue, Links, MfaConfig, PortalConfig, SamlConfig, Sso, Status,
    Tls, UniversityFile,
};
pub use validate::{Issue, is_valid_id};

/// Version du format de fichier comprise par cette version de tonAPI.
pub const SCHEMA_VERSION: u32 = 1;

pub(crate) fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}
