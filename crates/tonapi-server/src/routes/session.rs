//! Connexion, second facteur, renouvellement et déconnexion.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use tonapi_config::{ResolvedUniversity, ServiceAvailability, ServiceKind};
use tonapi_core::{ErrorCode, EstablishmentChoice, Problem};
use tonapi_providers::{Context, LoginInput, LoginOutcome, MfaAction};
use tonapi_session::SessionData;
use utoipa::ToSchema;

use super::establishment_choice;
use crate::auth::AuthSession;
use crate::error::{ApiError, invalid};
use crate::respond::no_store;
use crate::{AppState, access, limit};

/// Un ancien jeton reste utilisable pour reprendre ses cookies pendant ce délai après expiration :
/// l'appareil mémorisé par le CAS vaut 30 jours.
const PREVIOUS_TOKEN_GRACE_SECS: i64 = 35 * 24 * 3600;
/// Attente maximale d'une validation sur le téléphone.
const PUSH_WAIT: Duration = Duration::from_secs(100);

/// Identifiants de l'étudiant. Le mot de passe sert à la connexion puis est oublié.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginRequest {
    #[schema(example = "demo")]
    university: String,
    #[schema(example = "demo")]
    username: String,
    #[schema(example = "demo", format = Password)]
    password: String,
    /// Composante choisie par l'étudiant. Par défaut : détectée, sinon celle par défaut.
    establishment: Option<String>,
    /// Jeton d'une session précédente, même expirée (35 jours au plus) : ses cookies évitent que
    /// l'université redemande le second facteur.
    previous_token: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    /// Jeton à envoyer dans `Authorization: Bearer`.
    token: String,
    expires_at: DateTime<FixedOffset>,
    university: String,
    establishment: Option<EstablishmentChoice>,
}

/// Méthodes de second facteur proposées.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MfaMethods {
    /// Appareil qui reçoit la notification, si la validation par notification est possible.
    push: Option<String>,
    /// Un code (application d'authentification, code de secours) est accepté.
    code: bool,
}

/// L'université demande un second facteur.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MfaRequired {
    /// Toujours `mfaRequired`.
    status: &'static str,
    /// Jeton provisoire à renvoyer à `POST /v1/auth/mfa`. Ne donne accès à aucune donnée.
    pending: String,
    expires_at: DateTime<FixedOffset>,
    methods: MfaMethods,
    /// Code refusé, notification non validée…
    message: Option<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum MfaMethod {
    /// Valider sur le téléphone ; la requête attend la validation (100 s au plus).
    Push,
    /// Saisir un code.
    Code,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MfaRequest {
    /// Jeton `pending` reçu avec la réponse 202.
    pending: String,
    method: MfaMethod,
    /// Code à 6 chiffres, si `method` vaut `code`.
    #[schema(example = "123456")]
    code: Option<String>,
    /// Faire mémoriser l'appareil par l'université (30 jours sans second facteur). Défaut : oui.
    remember_device: Option<bool>,
}

/// Service dont le compte est distinct du compte universitaire.
#[derive(Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum SeparateService {
    /// Notes ; la même connexion sert aux absences quand elles viennent du même ScoDoc.
    Grades,
    Absences,
    /// Devoirs (Moodle en connexion locale).
    Homework,
}

impl SeparateService {
    fn kind(self) -> ServiceKind {
        match self {
            SeparateService::Grades => ServiceKind::Grades,
            SeparateService::Absences => ServiceKind::Absences,
            SeparateService::Homework => ServiceKind::Homework,
        }
    }
}

/// Identifiants propres à un service. Le mot de passe sert une fois puis est oublié : seuls les
/// cookies ou le jeton du service restent dans la session.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceLoginRequest {
    service: SeparateService,
    #[schema(example = "camille.demo")]
    username: String,
    #[schema(format = Password)]
    password: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenResponse {
    token: String,
    expires_at: DateTime<FixedOffset>,
}

fn timestamp(secs: i64) -> DateTime<FixedOffset> {
    DateTime::from_timestamp(secs, 0)
        .unwrap_or_default()
        .fixed_offset()
}

fn university<'a>(
    registry: &'a tonapi_config::Registry,
    id: &str,
) -> Result<&'a ResolvedUniversity, ApiError> {
    registry.get(id).ok_or_else(|| {
        Problem::new(ErrorCode::UnknownUniversity)
            .detail(format!("`{id}` n'est pas servie"))
            .into()
    })
}

/// 200 avec le jeton, ou 202 si un second facteur est attendu.
fn outcome_response(
    state: &AppState,
    university: &ResolvedUniversity,
    outcome: LoginOutcome,
) -> Result<Response, ApiError> {
    let mut response = match outcome {
        LoginOutcome::SignedIn(session) => {
            let token = seal(state, &session)?;
            Json(LoginResponse {
                token,
                expires_at: timestamp(session.expires_at),
                university: session.university.clone(),
                establishment: establishment_choice(university, session.establishment.as_ref()),
            })
            .into_response()
        }
        LoginOutcome::SecondFactor(session, prompt) => {
            let pending = seal(state, &session)?;
            (
                StatusCode::ACCEPTED,
                Json(MfaRequired {
                    status: "mfaRequired",
                    pending,
                    expires_at: timestamp(session.expires_at),
                    methods: MfaMethods {
                        push: prompt.methods.push,
                        code: prompt.methods.code,
                    },
                    message: prompt.message,
                }),
            )
                .into_response()
        }
    };
    no_store(&mut response);
    Ok(response)
}

fn seal(state: &AppState, session: &SessionData) -> Result<String, ApiError> {
    let token = state
        .keys
        .seal(session)
        .map_err(|_| ApiError::from(ErrorCode::Internal))?;
    access::token_issued(&token);
    Ok(token)
}

/// Réponse ou erreur marquée de l'université (identifiant de configuration, jamais la valeur
/// reçue) pour le journal d'accès et les métriques.
fn tagged(result: Result<Response, ApiError>, university: &ResolvedUniversity) -> Response {
    let mut response = result.unwrap_or_else(IntoResponse::into_response);
    response
        .extensions_mut()
        .insert(access::University(university.university.id.clone()));
    response
}

/// Connexion avec le compte universitaire.
#[utoipa::path(
    post,
    path = "/v1/auth/login",
    tag = "session",
    security(()),
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Connecté", body = LoginResponse),
        (status = 202, description = "Second facteur demandé : appeler `POST /v1/auth/mfa`", body = MfaRequired),
        (status = 400, description = "`invalid_request`", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "`invalid_credentials`", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = "`account_action_required` : mot de passe expiré…", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`unknown_university`, `unknown_establishment`", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "`rate_limited` : trop d'échecs pour ce compte", body = Problem, content_type = "application/problem+json"),
        (status = 502, description = "`service_unavailable`, `upstream_changed`", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn login(
    State(state): State<AppState>,
    payload: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(request) = payload.map_err(|e| invalid(e.body_text()))?;
    check_lengths(&request.username, &request.password)?;
    let registry = state.registry.load_full();
    let university = university(&registry, &request.university)?;
    let limit_key = limit::key("login", &request.university, &request.username);
    let attempt = state
        .login_limiter
        .begin(&limit_key)
        .inspect_err(|problem| {
            record_login(&university.university.id, problem.code.as_str());
        })?;

    let now = Utc::now();
    let previous_cookies = request
        .previous_token
        .as_deref()
        .and_then(|t| {
            state
                .keys
                .open_with_grace(t, now.timestamp(), PREVIOUS_TOKEN_GRACE_SECS)
                .ok()
        })
        // Même université et même compte seulement : les cookies d'un autre étudiant ne doivent
        // jamais entrer dans cette session.
        .filter(|previous| {
            previous.university == request.university
                && !previous.pending
                && previous
                    .username
                    .trim()
                    .eq_ignore_ascii_case(request.username.trim())
        })
        .map(|previous| previous.cookies)
        .unwrap_or_default();

    let result = tonapi_providers::login(
        university,
        LoginInput {
            username: request.username.trim(),
            password: &request.password,
            establishment: request.establishment.as_deref(),
            previous_cookies,
            now,
            ttl_secs: state.settings.session_ttl_secs,
            net: tonapi_providers::NetSettings::default(),
        },
    )
    .await;
    let response = match result {
        Ok(outcome) => outcome_response(&state, university, outcome),
        Err(problem) => {
            // Une page de refus non reconnue (`upstream_changed`) compte aussi : sinon un CAS qui
            // verrouille le compte avec une page inconnue laisserait les essais sans limite.
            if matches!(
                problem.code,
                ErrorCode::InvalidCredentials
                    | ErrorCode::UpstreamChanged
                    | ErrorCode::AccountActionRequired
            ) {
                attempt.failed();
            }
            Err(problem.into())
        }
    };
    record_login(&university.university.id, login_outcome(&response));
    Ok(tagged(response, university))
}

/// Résultat d'une connexion pour la métrique : `success`, `second_factor` ou le code d'erreur
/// stable. Valeurs en nombre fini, jamais une donnée reçue.
fn login_outcome(response: &Result<Response, ApiError>) -> &'static str {
    match response {
        Ok(response) if response.status() == StatusCode::ACCEPTED => "second_factor",
        Ok(_) => "success",
        Err(error) => error.0.code.as_str(),
    }
}

/// `tonapi_login_total` : `university` est l'identifiant de configuration.
fn record_login(university: &str, outcome: &str) {
    tonapi_metrics::inc(
        "tonapi_login_total",
        &[("university", university), ("outcome", outcome)],
    );
}

/// Second facteur : notification sur le téléphone ou code.
#[utoipa::path(
    post,
    path = "/v1/auth/mfa",
    tag = "session",
    security(()),
    request_body = MfaRequest,
    responses(
        (status = 200, description = "Connecté", body = LoginResponse),
        (status = 202, description = "Code refusé ou notification non validée : nouveau jeton `pending`", body = MfaRequired),
        (status = 400, description = "`invalid_request`", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "`invalid_token`, `session_expired` : recommencer la connexion", body = Problem, content_type = "application/problem+json"),
        (status = 502, description = "`service_unavailable`", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn mfa(
    State(state): State<AppState>,
    payload: Result<Json<MfaRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(request) = payload.map_err(|e| invalid(e.body_text()))?;
    let now = Utc::now();
    let pending = state
        .keys
        .open(&request.pending, now.timestamp())
        .map_err(|e| match e {
            tonapi_session::SessionError::Expired => ApiError::from(ErrorCode::SessionExpired),
            _ => ErrorCode::InvalidToken.into(),
        })?;
    if !pending.pending {
        return Err(invalid("ce jeton n'attend pas de second facteur"));
    }
    let action = match (request.method, request.code) {
        (MfaMethod::Code, Some(code)) if code.len() > 64 => {
            return Err(invalid("code de validation trop long"));
        }
        (MfaMethod::Code, Some(code)) if !code.trim().is_empty() => MfaAction::Code(code),
        (MfaMethod::Code, _) => return Err(invalid("`code` est obligatoire avec `method: code`")),
        (MfaMethod::Push, _) => MfaAction::Push { wait: PUSH_WAIT },
    };
    let registry = state.registry.load_full();
    let university = university(&registry, &pending.university)?;
    // Codes refusés et notifications non validées comptent : ni essais sans fin, ni
    // notifications en boucle sur le téléphone de l'étudiant.
    let limit_key = limit::key("mfa", &pending.university, &pending.username);
    let attempt = state.login_limiter.begin(&limit_key)?;
    let result = tonapi_providers::complete_mfa(
        university,
        pending,
        action,
        request.remember_device.unwrap_or(true),
        now,
        state.settings.session_ttl_secs,
    )
    .await;
    // Toute issue autre qu'une connexion compte, erreurs comprises.
    let response = match result {
        Ok(outcome @ LoginOutcome::SignedIn(_)) => outcome_response(&state, university, outcome),
        Ok(outcome) => {
            attempt.failed();
            outcome_response(&state, university, outcome)
        }
        Err(problem) => {
            attempt.failed();
            Err(problem.into())
        }
    };
    Ok(tagged(response, university))
}

/// Identifiant et mot de passe de taille raisonnable : un identifiant démesuré gonflerait les
/// tables du limiteur et partirait tel quel au CAS.
fn check_lengths(username: &str, password: &str) -> Result<(), ApiError> {
    if username.trim().is_empty() || username.len() > 256 || password.len() > 1024 {
        return Err(invalid(
            "identifiant (256 caractères au plus) ou mot de passe (1024 au plus) invalide",
        ));
    }
    Ok(())
}

/// Clé du limiteur pour un compte de service : distincte de celle du compte universitaire, et
/// commune aux services servis par le même fournisseur (notes et absences ScoDoc).
fn service_limit_key(university: &str, provider: &str, username: &str) -> String {
    limit::key("service", &format!("{university}:{provider}"), username)
}

/// Connexion à un service qui a son propre compte, après une réponse `service_login_required`
/// (ScoDoc derrière le CAS d'une composante, Moodle en connexion locale).
#[utoipa::path(
    post,
    path = "/v1/auth/service-login",
    tag = "session",
    security(("session" = [])),
    request_body = ServiceLoginRequest,
    responses(
        (status = 200, description = "Connecté au service : remplacer le jeton de session par celui-ci", body = TokenResponse),
        (status = 400, description = "`invalid_request` : ce service n'a pas de connexion séparée", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "`invalid_credentials` ; `invalid_token`, `session_expired` pour le jeton de session", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = "`account_action_required` : mot de passe expiré…", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`service_absent`", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "`rate_limited` : trop d'échecs pour ce compte", body = Problem, content_type = "application/problem+json"),
        (status = 501, description = "`not_implemented` : connexion de ce type pas encore prise en charge", body = Problem, content_type = "application/problem+json"),
        (status = 502, description = "`service_unavailable`, `upstream_changed`", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn service_login(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
    payload: Result<Json<ServiceLoginRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(request) = payload.map_err(|e| invalid(e.body_text()))?;
    check_lengths(&request.username, &request.password)?;
    let kind = request.service.kind();
    let registry = state.registry.load_full();
    let university = university(&registry, &session.university)?;
    let provider = university
        .services_for(session.establishment.as_ref().map(|e| e.id.as_str()))
        .availability()
        .into_iter()
        .find_map(|(k, availability)| match availability {
            ServiceAvailability::Api { provider } if k == kind => Some(provider),
            _ => None,
        })
        .unwrap_or_default();
    let limit_key = service_limit_key(&session.university, &provider, &request.username);
    let _slot = state.account_slots.acquire(&session)?;
    let attempt = state.login_limiter.begin(&limit_key)?;

    let mut updated = session;
    let mut ctx = Context {
        university,
        session: &mut updated,
        now: Utc::now(),
        net: tonapi_providers::NetSettings::default(),
    };
    let result =
        tonapi_providers::service_login(&mut ctx, kind, &request.username, &request.password).await;
    if let Err(problem) = result {
        if problem.code == ErrorCode::InvalidCredentials {
            attempt.failed();
        }
        return Err(problem.into());
    }
    let mut response = Json(TokenResponse {
        token: seal(&state, &updated)?,
        expires_at: timestamp(updated.expires_at),
    })
    .into_response();
    no_store(&mut response);
    Ok(response)
}

/// Prolonge la session (repasse par le SSO de l'université, sans mot de passe).
#[utoipa::path(
    post,
    path = "/v1/auth/refresh",
    tag = "session",
    security(("session" = [])),
    responses(
        (status = 200, description = "Nouveau jeton", body = TokenResponse),
        (status = 401, description = "`invalid_token`, `session_expired` : se reconnecter", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "`rate_limited` : trop de requêtes en cours pour ce compte", body = Problem, content_type = "application/problem+json"),
        (status = 502, description = "`service_unavailable`", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn refresh(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
) -> Result<Response, ApiError> {
    // Comme les données : un compte ne prend pas à lui seul les places de l'hôte du CAS.
    let _slot = state.account_slots.acquire(&session)?;
    let registry = state.registry.load_full();
    let university = university(&registry, &session.university)?;
    let session = tonapi_providers::refresh(
        university,
        session,
        Utc::now(),
        state.settings.session_ttl_secs,
    )
    .await?;
    let mut response = Json(TokenResponse {
        token: seal(&state, &session)?,
        expires_at: timestamp(session.expires_at),
    })
    .into_response();
    no_store(&mut response);
    Ok(response)
}

/// Déconnexion : la session CAS de l'étudiant est fermée, puis le client oublie son jeton. Le
/// jeton reste techniquement lisible jusqu'à son expiration (aucune liste de révocation côté
/// serveur), mais le CAS ne le reconnaît plus.
#[utoipa::path(
    post,
    path = "/v1/auth/logout",
    tag = "session",
    security(("session" = [])),
    responses(
        (status = 204, description = "Session terminée (la fermeture côté CAS est tentée, sans garantie)"),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "`rate_limited` : trop de requêtes en cours pour ce compte, réessayer", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn logout(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
) -> Result<StatusCode, ApiError> {
    let _slot = state.account_slots.acquire(&session)?;
    let registry = state.registry.load_full();
    if let Some(university) = registry.get(&session.university) {
        tonapi_providers::logout(
            university,
            &session,
            tonapi_providers::NetSettings::default(),
        )
        .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::service_limit_key;

    #[test]
    fn la_cle_du_limiteur_est_propre_au_service() {
        let key = service_limit_key("umontpellier", "scodoc-gateway", "Camille.Demo");
        assert_eq!(key, "service:umontpellier:scodoc-gateway:camille.demo");
        // Jamais la même clé que la connexion universitaire (`<université>:<identifiant>`).
        assert_ne!(key, "umontpellier:camille.demo");
    }
}
