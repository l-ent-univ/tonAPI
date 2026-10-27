//! Connexion, second facteur, renouvellement et déconnexion.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use tonapi_config::ResolvedUniversity;
use tonapi_core::{ErrorCode, EstablishmentChoice, Problem};
use tonapi_providers::{LoginInput, LoginOutcome, MfaAction};
use tonapi_session::SessionData;
use utoipa::ToSchema;

use super::establishment_choice;
use crate::AppState;
use crate::auth::AuthSession;
use crate::error::{ApiError, invalid};
use crate::respond::no_store;

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
    state
        .keys
        .seal(session)
        .map_err(|_| ErrorCode::Internal.into())
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
    let registry = state.registry.load_full();
    let university = university(&registry, &request.university)?;
    let limit_key = format!("{}:{}", request.university, request.username.to_lowercase());
    state.login_limiter.check(&limit_key)?;

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
        .filter(|previous| previous.university == request.university)
        .map(|previous| previous.cookies)
        .unwrap_or_default();

    let result = tonapi_providers::login(
        university,
        LoginInput {
            username: &request.username,
            password: &request.password,
            establishment: request.establishment.as_deref(),
            previous_cookies,
            now,
            ttl_secs: state.settings.session_ttl_secs,
            net: tonapi_providers::NetSettings::default(),
        },
    )
    .await;
    match result {
        Ok(outcome) => outcome_response(&state, university, outcome),
        Err(problem) => {
            if problem.code == ErrorCode::InvalidCredentials {
                state.login_limiter.record_failure(&limit_key);
            }
            Err(problem.into())
        }
    }
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
        (MfaMethod::Code, Some(code)) if !code.trim().is_empty() => MfaAction::Code(code),
        (MfaMethod::Code, _) => return Err(invalid("`code` est obligatoire avec `method: code`")),
        (MfaMethod::Push, _) => MfaAction::Push { wait: PUSH_WAIT },
    };
    let registry = state.registry.load_full();
    let university = university(&registry, &pending.university)?;
    let outcome = tonapi_providers::complete_mfa(
        university,
        pending,
        action,
        request.remember_device.unwrap_or(true),
        now,
        state.settings.session_ttl_secs,
    )
    .await?;
    outcome_response(&state, university, outcome)
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
        (status = 502, description = "`service_unavailable`", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn refresh(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
) -> Result<Response, ApiError> {
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

/// Déconnexion. Le client oublie son jeton ; rien n'est gardé côté serveur.
#[utoipa::path(
    post,
    path = "/v1/auth/logout",
    tag = "session",
    security(("session" = [])),
    responses(
        (status = 204, description = "Session terminée"),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn logout(AuthSession(_session): AuthSession) -> StatusCode {
    StatusCode::NO_CONTENT
}
