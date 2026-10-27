use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::services::Services;

/// Fichier `universities/<id>/university.toml`.
///
/// Seuls `id`, `name` et `sso` sont obligatoires ; un service absent n'est pas proposé.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct UniversityFile {
    /// Version du format. Vaut 1.
    #[serde(default = "crate::default_schema_version")]
    pub schema_version: u32,
    /// Identifiant stable, identique au nom du dossier (`a-z`, `0-9`, `-`).
    pub id: String,
    /// Nom complet affiché à l'étudiant.
    pub name: String,
    /// Nom court (sigle).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
    /// Avancement de la prise en charge.
    #[serde(default)]
    pub status: Status,
    /// Fuseau horaire IANA des données (emploi du temps, échéances).
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// Liens utiles affichés tels quels.
    #[serde(default)]
    pub links: Links,
    /// Connexion unique de l'université.
    pub sso: Sso,
    /// Portail (ENT) : liste des applications et profil.
    #[serde(default)]
    pub portal: PortalConfig,
    /// Services proposés à tous les étudiants, sauf remplacement par une composante.
    #[serde(default)]
    pub services: Services,
    /// Réglages TLS pour les serveurs mal configurés.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<Tls>,
}

fn default_timezone() -> String {
    "Europe/Paris".to_owned()
}

/// Avancement de la prise en charge d'une université ou d'une composante.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Testée avec un vrai compte.
    #[default]
    Active,
    /// Configurée d'après la documentation, pas encore testée de bout en bout.
    Beta,
    /// Annoncée, non utilisable.
    Planned,
}

/// Liens utiles. Tous facultatifs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Links {
    /// Site de l'établissement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<Url>,
    /// Page d'aide sur le compte numérique.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_help: Option<Url>,
    /// Page « mot de passe oublié ».
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_reset: Option<Url>,
    /// Page d'activation du compte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_activation: Option<Url>,
}

/// Connexion unique.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Sso {
    /// Serveur CAS.
    Cas(Box<CasSso>),
    /// Connexion factice de l'université de démonstration (identifiants `demo` / `demo`).
    Demo,
}

/// Serveur CAS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct CasSso {
    /// Racine du serveur, sans `/login` (ex. `https://cas.exemple.fr/cas/`).
    pub base_url: Url,
    /// Logiciel CAS. `apereo` convient à la plupart des universités.
    #[serde(default)]
    pub flavor: CasFlavor,
    /// Second facteur, quand l'université en demande un.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mfa: Option<MfaConfig>,
    /// Fédération SAML (Shibboleth) branchée sur ce CAS, pour Moodle ou le mail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saml: Option<SamlConfig>,
}

/// Logiciel CAS, qui détermine la forme du formulaire et des redirections.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CasFlavor {
    /// Apereo CAS 5 à 7 : formulaire avec `execution`.
    #[default]
    Apereo,
    /// esup-otp-cas-server devant Apereo CAS : `/cas` puis `/pre-otp`, rappel `login-<md5(service)>`.
    EsupOtpGateway,
    /// phpCAS / CAS 1-2 : formulaire avec jeton `lt`.
    Phpcas,
}

/// Second facteur esup-otp.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct MfaConfig {
    /// Racine de l'API esup-otp-api.
    pub api_url: Url,
}

/// Passage par une fédération SAML : fournisseur de service → WAYF → IdP → CAS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct SamlConfig {
    /// Service de découverte (WAYF).
    pub wayf_url: Url,
    /// entityID de l'IdP de l'université, choisi dans le WAYF.
    pub idp_entity_id: String,
    /// Suffixes de nom d'hôte autorisés pendant les redirections (ex. `renater.fr`).
    pub allowed_hosts: Vec<String>,
}

/// Portail (ENT).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum PortalConfig {
    /// uPortal 4 avec ESUP ProlongationENT (`layout.jsp` en JSONP).
    Uportal4Prolongation {
        /// Racine uPortal (ex. `https://ent.exemple.fr/uPortal/`).
        #[serde(rename = "base-url")]
        base_url: Url,
    },
    /// uPortal 5 (`/api/v4-3/dlm/layout.json`, `/api/v5-1/userinfo`).
    Uportal5 {
        /// Racine du portail.
        #[serde(rename = "base-url")]
        base_url: Url,
    },
    /// Portail à tuiles avec client CAS intégré : entrée `cas/login?next=/`, profil
    /// `api/cas-attributes`, applications `api/tiles`. Logiciel non identifié (vu à Montpellier,
    /// octobre 2026).
    TilesPortal {
        /// Racine du portail (ex. `https://ent.exemple.fr/`).
        #[serde(rename = "base-url")]
        base_url: Url,
    },
    /// Pas de portail exploitable.
    #[default]
    None,
}

/// Réglages TLS.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Tls {
    /// Certificats PEM à ajouter aux autorités reconnues, relatifs au dossier de l'université.
    /// Sert aux serveurs qui n'envoient pas leur certificat intermédiaire.
    #[serde(default)]
    pub extra_ca: Vec<PathBuf>,
}
