//! Erreurs `application/problem+json` (RFC 9457) avec des codes stables.

use std::fmt;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Code d'erreur stable. Un client se fie à `code`, jamais au texte de `title` ou `detail`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Paramètres de requête invalides.
    InvalidRequest,
    /// Route inconnue.
    NotFound,
    /// Identifiant ou mot de passe refusé par l'université.
    InvalidCredentials,
    /// Jeton absent, illisible ou modifié.
    InvalidToken,
    /// Session expirée : se reconnecter.
    SessionExpired,
    /// L'université demande une action sur le compte (mot de passe expiré…), à faire sur son site.
    AccountActionRequired,
    /// Université inconnue.
    UnknownUniversity,
    /// Composante inconnue pour cette université.
    UnknownEstablishment,
    /// Service non proposé à cet étudiant (éventuellement remplacé par un lien).
    ServiceAbsent,
    /// Fournisseur prévu mais pas encore pris en charge par cette version.
    NotImplemented,
    /// Le serveur de l'université ne répond pas ou renvoie une erreur.
    ServiceUnavailable,
    /// Le serveur de l'université a répondu dans un format inattendu.
    UpstreamChanged,
    /// Trop de requêtes.
    RateLimited,
    /// Erreur interne.
    Internal,
}

impl ErrorCode {
    /// Statut HTTP associé.
    #[must_use]
    pub fn status(self) -> u16 {
        match self {
            ErrorCode::InvalidRequest => 400,
            ErrorCode::InvalidCredentials | ErrorCode::InvalidToken | ErrorCode::SessionExpired => {
                401
            }
            ErrorCode::NotFound
            | ErrorCode::UnknownUniversity
            | ErrorCode::UnknownEstablishment
            | ErrorCode::ServiceAbsent => 404,
            ErrorCode::AccountActionRequired => 403,
            ErrorCode::RateLimited => 429,
            ErrorCode::Internal => 500,
            ErrorCode::NotImplemented => 501,
            ErrorCode::ServiceUnavailable | ErrorCode::UpstreamChanged => 502,
        }
    }

    /// Titre court, en français.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            ErrorCode::InvalidRequest => "Requête invalide",
            ErrorCode::NotFound => "Introuvable",
            ErrorCode::InvalidCredentials => "Identifiant ou mot de passe incorrect",
            ErrorCode::InvalidToken => "Jeton de session invalide",
            ErrorCode::SessionExpired => "Session expirée",
            ErrorCode::AccountActionRequired => "Action requise sur le compte universitaire",
            ErrorCode::UnknownUniversity => "Université inconnue",
            ErrorCode::UnknownEstablishment => "Composante inconnue",
            ErrorCode::ServiceAbsent => "Service non proposé",
            ErrorCode::NotImplemented => "Pas encore pris en charge",
            ErrorCode::ServiceUnavailable => "Service de l'université indisponible",
            ErrorCode::UpstreamChanged => "Réponse inattendue du service de l'université",
            ErrorCode::RateLimited => "Trop de requêtes",
            ErrorCode::Internal => "Erreur interne",
        }
    }

    /// Valeur sérialisée (`invalid_credentials`…).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidRequest => "invalid_request",
            ErrorCode::NotFound => "not_found",
            ErrorCode::InvalidCredentials => "invalid_credentials",
            ErrorCode::InvalidToken => "invalid_token",
            ErrorCode::SessionExpired => "session_expired",
            ErrorCode::AccountActionRequired => "account_action_required",
            ErrorCode::UnknownUniversity => "unknown_university",
            ErrorCode::UnknownEstablishment => "unknown_establishment",
            ErrorCode::ServiceAbsent => "service_absent",
            ErrorCode::NotImplemented => "not_implemented",
            ErrorCode::ServiceUnavailable => "service_unavailable",
            ErrorCode::UpstreamChanged => "upstream_changed",
            ErrorCode::RateLimited => "rate_limited",
            ErrorCode::Internal => "internal",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Erreur RFC 9457.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// `urn:tonapi:problem:<code>`.
    #[serde(rename = "type")]
    #[schema(example = "urn:tonapi:problem:session_expired")]
    pub kind: String,
    pub title: String,
    pub status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub code: ErrorCode,
    /// Lien vers la page de l'établissement, quand le service n'existe que sous forme de lien.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

impl Problem {
    #[must_use]
    pub fn new(code: ErrorCode) -> Self {
        Problem {
            kind: format!("urn:tonapi:problem:{code}"),
            title: code.title().to_owned(),
            status: code.status(),
            detail: None,
            code,
            link: None,
        }
    }

    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    #[must_use]
    pub fn link(mut self, link: impl Into<String>) -> Self {
        self.link = Some(link.into());
        self
    }
}

impl From<ErrorCode> for Problem {
    fn from(code: ErrorCode) -> Self {
        Problem::new(code)
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.title, self.code)?;
        if let Some(detail) = &self.detail {
            write!(f, " : {detail}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Problem {}
