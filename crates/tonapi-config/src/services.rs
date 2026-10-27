//! Services proposés à l'étudiant et fournisseurs qui les rendent.
//!
//! Chaque service est une enum étiquetée par `provider` : un fournisseur inconnu, ou utilisé pour un
//! service qu'il ne rend pas, est refusé à la lecture du fichier et par le JSON Schema.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::university::CasFlavor;

/// Services, tous facultatifs. Un service absent n'est pas proposé ; dans un fichier de composante, un
/// service absent reprend celui de l'université.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Services {
    /// Emploi du temps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timetable: Option<TimetableService>,
    /// Notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grades: Option<GradesService>,
    /// Absences.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absences: Option<AbsencesService>,
    /// Calendrier des examens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exams: Option<ExamsService>,
    /// Devoirs et échéances.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homework: Option<HomeworkService>,
    /// Messagerie.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mail: Option<MailService>,
    /// Applications du portail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apps: Option<AppsService>,
    /// Profil (nom, adresse, composante).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileService>,
    /// Documents (certificat de scolarité, attestations).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documents: Option<DocumentsService>,
}

impl Services {
    /// Services de `self` remplacés, un par un, par ceux que `overrides` définit.
    #[must_use]
    pub fn merged_with(&self, overrides: &Services) -> Services {
        fn pick<T: Clone>(base: Option<&T>, over: Option<&T>) -> Option<T> {
            over.or(base).cloned()
        }
        Services {
            timetable: pick(self.timetable.as_ref(), overrides.timetable.as_ref()),
            grades: pick(self.grades.as_ref(), overrides.grades.as_ref()),
            absences: pick(self.absences.as_ref(), overrides.absences.as_ref()),
            exams: pick(self.exams.as_ref(), overrides.exams.as_ref()),
            homework: pick(self.homework.as_ref(), overrides.homework.as_ref()),
            mail: pick(self.mail.as_ref(), overrides.mail.as_ref()),
            apps: pick(self.apps.as_ref(), overrides.apps.as_ref()),
            profile: pick(self.profile.as_ref(), overrides.profile.as_ref()),
            documents: pick(self.documents.as_ref(), overrides.documents.as_ref()),
        }
    }

    /// Disponibilité de chaque service, dans l'ordre de [`ServiceKind::ALL`].
    #[must_use]
    pub fn availability(&self) -> Vec<(ServiceKind, ServiceAvailability)> {
        fn of<T: Provider>(s: Option<&T>) -> ServiceAvailability {
            s.map_or(ServiceAvailability::Absent, Provider::availability)
        }
        vec![
            (ServiceKind::Timetable, of(self.timetable.as_ref())),
            (ServiceKind::Grades, of(self.grades.as_ref())),
            (ServiceKind::Absences, of(self.absences.as_ref())),
            (ServiceKind::Exams, of(self.exams.as_ref())),
            (ServiceKind::Homework, of(self.homework.as_ref())),
            (ServiceKind::Mail, of(self.mail.as_ref())),
            (ServiceKind::Apps, of(self.apps.as_ref())),
            (ServiceKind::Profile, of(self.profile.as_ref())),
            (ServiceKind::Documents, of(self.documents.as_ref())),
        ]
    }
}

/// Nom d'un service.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceKind {
    Timetable,
    Grades,
    Absences,
    Exams,
    Homework,
    Mail,
    Apps,
    Profile,
    Documents,
}

impl ServiceKind {
    pub const ALL: [ServiceKind; 9] = [
        ServiceKind::Timetable,
        ServiceKind::Grades,
        ServiceKind::Absences,
        ServiceKind::Exams,
        ServiceKind::Homework,
        ServiceKind::Mail,
        ServiceKind::Apps,
        ServiceKind::Profile,
        ServiceKind::Documents,
    ];

    /// Nom de la clé dans `[services]`.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            ServiceKind::Timetable => "timetable",
            ServiceKind::Grades => "grades",
            ServiceKind::Absences => "absences",
            ServiceKind::Exams => "exams",
            ServiceKind::Homework => "homework",
            ServiceKind::Mail => "mail",
            ServiceKind::Apps => "apps",
            ServiceKind::Profile => "profile",
            ServiceKind::Documents => "documents",
        }
    }
}

/// Ce que l'étudiant obtient pour un service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ServiceAvailability {
    /// Données servies par l'API.
    Api { provider: String },
    /// Simple lien vers le site de l'établissement.
    Link { url: Url },
    /// Service non proposé.
    Absent,
}

/// Comportement commun des enums de service.
pub(crate) trait Provider {
    fn provider_id(&self) -> &'static str;
    fn link(&self) -> Option<&Url>;

    fn availability(&self) -> ServiceAvailability {
        match (self.provider_id(), self.link()) {
            ("none", _) => ServiceAvailability::Absent,
            (_, Some(url)) => ServiceAvailability::Link { url: url.clone() },
            (id, None) => ServiceAvailability::Api {
                provider: id.to_owned(),
            },
        }
    }
}

macro_rules! provider_ids {
    ($ty:ident { $($pat:pat => $id:literal),* $(,)? }) => {
        impl Provider for $ty {
            fn provider_id(&self) -> &'static str {
                match self {
                    $( $pat => $id, )*
                }
            }

            fn link(&self) -> Option<&Url> {
                match self {
                    Self::Link(link) => Some(&link.url),
                    _ => None,
                }
            }
        }
    };
}

/// Simple lien, quand le service n'a pas d'API exploitable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct LinkService {
    /// Page à ouvrir.
    pub url: Url,
    /// Libellé du lien.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Réglages communs à tous les fournisseurs distants. Tous facultatifs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ServiceOptions {
    /// Durée de cache des réponses, en secondes. Défaut propre au fournisseur.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_ttl_secs: Option<u32>,
    /// Délai maximal d'une requête, en secondes. Défaut : 20.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u32>,
}

impl ServiceOptions {
    fn is_default(&self) -> bool {
        *self == ServiceOptions::default()
    }
}

/// Manière d'ouvrir une session sur le service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum SignIn {
    /// CAS de l'université. Le service CAS se déduit de l'URL du fournisseur ; `service` le remplace.
    UniversityCas {
        /// URL de service CAS, si elle diffère de celle déduite.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        service: Option<Url>,
    },
    /// CAS propre à la composante.
    OwnCas {
        /// Racine du CAS de la composante.
        #[serde(rename = "base-url")]
        base_url: Url,
        /// Logiciel CAS.
        #[serde(default)]
        flavor: CasFlavor,
        /// Identifiants demandés.
        #[serde(default)]
        credentials: Credentials,
    },
    /// Identifiant et mot de passe envoyés directement au service (ex. `login/token.php` de Moodle).
    LocalPassword {
        /// Identifiants demandés.
        #[serde(default)]
        credentials: Credentials,
    },
}

impl Default for SignIn {
    fn default() -> Self {
        SignIn::UniversityCas { service: None }
    }
}

impl SignIn {
    fn is_default(&self) -> bool {
        *self == SignIn::default()
    }
}

/// Identifiants demandés à l'étudiant.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Credentials {
    /// Ceux du compte de l'université.
    #[default]
    University,
    /// Un autre compte, propre à la composante.
    Separate {
        /// Ce qu'on demande à l'étudiant (ex. « Identifiant IUT (prenom.nom) »).
        label: String,
    },
}

/// Passerelle ScoDoc (Scodoc_Notes) : `/services/data.php`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ScoDocGateway {
    /// Racine de la passerelle (ex. `https://notes.iut.exemple.fr/`).
    pub base_url: Url,
    /// Connexion. Défaut : CAS de l'université.
    #[serde(default, skip_serializing_if = "SignIn::is_default")]
    pub sign_in: SignIn,
    #[serde(default, skip_serializing_if = "ServiceOptions::is_default")]
    pub options: ServiceOptions,
}

/// Emploi du temps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "provider", rename_all = "kebab-case", deny_unknown_fields)]
pub enum TimetableService {
    /// Flux iCal ADE (`direct_cal.jsp?data=…`) dont le lien est lu sur une page authentifiée.
    AdeIcalLink {
        /// Page, derrière le CAS, qui contient le lien du flux.
        #[serde(rename = "link-page")]
        link_page: Url,
        /// Hôte du serveur ADE qui sert le flux.
        #[serde(rename = "feed-host")]
        feed_host: String,
        #[serde(default, skip_serializing_if = "ServiceOptions::is_default")]
        options: ServiceOptions,
    },
    /// API REST du portail ADE (`/portal/RestApi/`), après connexion CAS.
    AdePortalRest {
        /// Racine du portail ADE (ex. `https://planning.exemple.fr/portal/`).
        #[serde(rename = "base-url")]
        base_url: Url,
        #[serde(default, skip_serializing_if = "ServiceOptions::is_default")]
        options: ServiceOptions,
    },
    /// ADE « MyPlanning » (`/direct/myplanning.jsp`, interface GWT) : les ressources de
    /// l'étudiant sont lues après connexion CAS, puis le flux vient de `anonymous_cal.jsp`.
    AdeMyplanning {
        /// Racine du serveur ADE (ex. `https://planning.exemple.fr/`).
        #[serde(rename = "base-url")]
        base_url: Url,
        /// Projet ADE. Défaut : 1.
        #[serde(rename = "project-id", default = "default_project")]
        project_id: u32,
        #[serde(default, skip_serializing_if = "ServiceOptions::is_default")]
        options: ServiceOptions,
    },
    /// Simple lien vers le site de l'établissement.
    Link(LinkService),
    /// Service non proposé.
    None,
    /// Données fictives de l'université de démonstration.
    Demo,
}

fn default_project() -> u32 {
    1
}

provider_ids!(TimetableService {
    Self::AdeIcalLink { .. } => "ade-ical-link",
    Self::AdePortalRest { .. } => "ade-portal-rest",
    Self::AdeMyplanning { .. } => "ade-myplanning",
    Self::Link(_) => "link",
    Self::None => "none",
    Self::Demo => "demo",
});

/// Calendrier des examens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "provider", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ExamsService {
    /// Application de scolarité avec `api/etudiants/calendrier` (voir [`StudentRecord::EtudiantsApi`]).
    EtudiantsApi {
        /// Racine de l'application (ex. `https://scolarite.exemple.fr/`).
        #[serde(rename = "base-url")]
        base_url: Url,
    },
    /// Simple lien vers le site de l'établissement.
    Link(LinkService),
    /// Service non proposé.
    None,
    /// Données fictives de l'université de démonstration.
    Demo,
}

provider_ids!(ExamsService {
    Self::EtudiantsApi { .. } => "etudiants-api",
    Self::Link(_) => "link",
    Self::None => "none",
    Self::Demo => "demo",
});

/// Notes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "provider", rename_all = "kebab-case", deny_unknown_fields)]
pub enum GradesService {
    /// Passerelle ScoDoc des IUT.
    ScodocGateway(ScoDocGateway),
    /// Application de scolarité avec `api/etudiants/notes` (voir [`StudentRecord::EtudiantsApi`]).
    EtudiantsApi {
        /// Racine de l'application (ex. `https://scolarite.exemple.fr/`).
        #[serde(rename = "base-url")]
        base_url: Url,
    },
    /// Simple lien vers le site de l'établissement.
    Link(LinkService),
    /// Service non proposé.
    None,
    /// Données fictives de l'université de démonstration.
    Demo,
}

provider_ids!(GradesService {
    Self::ScodocGateway(_) => "scodoc-gateway",
    Self::EtudiantsApi { .. } => "etudiants-api",
    Self::Link(_) => "link",
    Self::None => "none",
    Self::Demo => "demo",
});

/// Absences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "provider", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AbsencesService {
    /// Passerelle ScoDoc des IUT.
    ScodocGateway(ScoDocGateway),
    /// Simple lien vers le site de l'établissement.
    Link(LinkService),
    /// Service non proposé.
    None,
    /// Données fictives de l'université de démonstration.
    Demo,
}

provider_ids!(AbsencesService {
    Self::ScodocGateway(_) => "scodoc-gateway",
    Self::Link(_) => "link",
    Self::None => "none",
    Self::Demo => "demo",
});

/// Entrée de connexion d'un Moodle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MoodleEntry {
    /// `/login/index.php?authCAS=CAS`.
    #[default]
    Cas,
    /// `/auth/shibboleth/index.php`, via la fédération SAML de l'université.
    Shibboleth,
}

/// Devoirs et échéances.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "provider", rename_all = "kebab-case", deny_unknown_fields)]
pub enum HomeworkService {
    /// Web service Moodle (`/webservice/rest/server.php`) avec un jeton.
    MoodleWebservice {
        /// Racine du Moodle (ex. `https://moodle.exemple.fr/`).
        #[serde(rename = "base-url")]
        base_url: Url,
        /// Obtention du jeton. Défaut : CAS de l'université.
        #[serde(
            rename = "sign-in",
            default,
            skip_serializing_if = "SignIn::is_default"
        )]
        sign_in: SignIn,
        #[serde(default, skip_serializing_if = "ServiceOptions::is_default")]
        options: ServiceOptions,
    },
    /// Appels AJAX de l'interface Moodle (`/lib/ajax/service.php`) avec la session web.
    MoodleAjax {
        /// Racine du Moodle.
        #[serde(rename = "base-url")]
        base_url: Url,
        /// Entrée de connexion.
        #[serde(default)]
        entry: MoodleEntry,
        #[serde(default, skip_serializing_if = "ServiceOptions::is_default")]
        options: ServiceOptions,
    },
    /// Simple lien vers le site de l'établissement.
    Link(LinkService),
    /// Service non proposé.
    None,
    /// Données fictives de l'université de démonstration.
    Demo,
}

provider_ids!(HomeworkService {
    Self::MoodleWebservice { .. } => "moodle-webservice",
    Self::MoodleAjax { .. } => "moodle-ajax",
    Self::Link(_) => "link",
    Self::None => "none",
    Self::Demo => "demo",
});

/// Messagerie.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "provider", rename_all = "kebab-case", deny_unknown_fields)]
pub enum MailService {
    /// Zimbra (`/service/soap`, `/home/~/inbox`), session `ZM_AUTH_TOKEN` obtenue par préauthentification.
    ZimbraRest {
        /// Racine du webmail.
        #[serde(rename = "base-url")]
        base_url: Url,
        /// Page qui lance la connexion. Défaut : `base-url`.
        #[serde(rename = "entry-url", default, skip_serializing_if = "Option::is_none")]
        entry_url: Option<Url>,
        #[serde(default, skip_serializing_if = "ServiceOptions::is_default")]
        options: ServiceOptions,
    },
    /// BlueMind : connexion par le CAS sur le webmail, puis API REST (`/api/…`) avec la clé de
    /// session de `webapp/session-infos.js`.
    Bluemind {
        /// Racine du webmail (ex. `https://mail.exemple.fr/`).
        #[serde(rename = "base-url")]
        base_url: Url,
        #[serde(default, skip_serializing_if = "ServiceOptions::is_default")]
        options: ServiceOptions,
    },
    /// Simple lien vers le site de l'établissement.
    Link(LinkService),
    /// Service non proposé.
    None,
    /// Données fictives de l'université de démonstration.
    Demo,
}

provider_ids!(MailService {
    Self::ZimbraRest { .. } => "zimbra-rest",
    Self::Bluemind { .. } => "bluemind",
    Self::Link(_) => "link",
    Self::None => "none",
    Self::Demo => "demo",
});

/// Applications du portail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "provider", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AppsService {
    /// Lues dans le portail déclaré dans `[portal]`.
    Portal,
    /// Service non proposé.
    None,
    /// Données fictives de l'université de démonstration.
    Demo,
}

impl Provider for AppsService {
    fn provider_id(&self) -> &'static str {
        match self {
            AppsService::Portal => "portal",
            AppsService::None => "none",
            AppsService::Demo => "demo",
        }
    }

    fn link(&self) -> Option<&Url> {
        None
    }
}

/// Profil.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "provider", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ProfileService {
    /// Lu dans le portail déclaré dans `[portal]`, éventuellement complété par un dossier
    /// étudiant (numéro, formation).
    Portal {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        record: Option<StudentRecord>,
    },
    /// Service non proposé.
    None,
    /// Données fictives de l'université de démonstration.
    Demo,
}

/// Source qui complète le profil du portail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum StudentRecord {
    /// Application de scolarité avec `api/etudiants/notes` (JSON : `numero`, `etapes[].libelle`),
    /// derrière le CAS de l'université. Logiciel non identifié (vu à Paul-Valéry, octobre 2026).
    EtudiantsApi {
        /// Racine de l'application (ex. `https://scolarite.exemple.fr/`).
        #[serde(rename = "base-url")]
        base_url: Url,
    },
}

impl Provider for ProfileService {
    fn provider_id(&self) -> &'static str {
        match self {
            ProfileService::Portal { .. } => "portal",
            ProfileService::None => "none",
            ProfileService::Demo => "demo",
        }
    }

    fn link(&self) -> Option<&Url> {
        None
    }
}

/// Documents administratifs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "provider", rename_all = "kebab-case", deny_unknown_fields)]
pub enum DocumentsService {
    /// Mon Dossier Web (Vaadin), onglet Inscriptions.
    MonDossierWeb {
        /// Racine de Mon Dossier Web.
        #[serde(rename = "base-url")]
        base_url: Url,
        #[serde(default, skip_serializing_if = "ServiceOptions::is_default")]
        options: ServiceOptions,
    },
    /// Application de scolarité avec `api/etudiants/attestations` : attestations semestrielles,
    /// annuelles et de réussite, en PDF.
    EtudiantsApi {
        /// Racine de l'application (ex. `https://scolarite.exemple.fr/`).
        #[serde(rename = "base-url")]
        base_url: Url,
        /// Page du certificat de scolarité, ajoutée à la liste comme lien.
        #[serde(
            default,
            rename = "certificate-url",
            skip_serializing_if = "Option::is_none"
        )]
        certificate_url: Option<Url>,
    },
    /// API ardemat (attestations dématérialisées).
    Ardemat {
        /// Racine de l'API (ex. `https://app.exemple.fr/ardemat-api/`).
        #[serde(rename = "base-url")]
        base_url: Url,
        #[serde(default, skip_serializing_if = "ServiceOptions::is_default")]
        options: ServiceOptions,
    },
    /// Simple lien vers le site de l'établissement.
    Link(LinkService),
    /// Service non proposé.
    None,
    /// Données fictives de l'université de démonstration.
    Demo,
}

provider_ids!(DocumentsService {
    Self::MonDossierWeb { .. } => "mon-dossier-web",
    Self::EtudiantsApi { .. } => "etudiants-api",
    Self::Ardemat { .. } => "ardemat",
    Self::Link(_) => "link",
    Self::None => "none",
    Self::Demo => "demo",
});
