use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::affiliation::Affiliation;
use crate::services::Services;
use crate::university::{Links, Status, Tls};

/// Fichier `universities/<université>/establishments/<id>.toml` : une composante (IUT, UFR, faculté)
/// dont les étudiants n'ont pas tout à fait les mêmes services que le reste de l'université.
///
/// Seuls `id` et `name` sont obligatoires.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct EstablishmentFile {
    /// Version du format. Vaut 1.
    #[serde(default = "crate::default_schema_version")]
    pub schema_version: u32,
    /// Identifiant stable, identique au nom du fichier sans `.toml` (`a-z`, `0-9`, `-`).
    pub id: String,
    /// Nom complet affiché à l'étudiant.
    pub name: String,
    /// Nom court.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
    /// Avancement de la prise en charge.
    #[serde(default)]
    pub status: Status,
    /// Composante choisie quand aucune règle de rattachement ne correspond. Une seule par université.
    #[serde(default)]
    pub default: bool,
    /// Règles de rattachement automatique. Sans règle, l'étudiant choisit lui-même.
    #[serde(default)]
    pub affiliation: Affiliation,
    /// Services qui remplacent ceux de l'université pour les étudiants de la composante.
    #[serde(default)]
    pub services: Services,
    /// Liens propres à la composante.
    #[serde(default)]
    pub links: Links,
    /// Réglages TLS propres aux serveurs de la composante.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<Tls>,
}
