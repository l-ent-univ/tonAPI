//! Appel d'un fournisseur et mise en forme de la réponse `{ data, meta }`.

use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use serde::Serialize;
use tonapi_core::{ErrorCode, Meta, Problem};
use tonapi_providers::{Context, Fetched};
use tonapi_session::SessionData;
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::{AppState, SESSION_HEADER};

/// Réponse de données.
#[derive(Serialize, ToSchema)]
pub struct DataResponse<T: ToSchema> {
    pub data: T,
    pub meta: Meta,
}

/// Exécute `fetch` avec la session de l'étudiant. Si le fournisseur a modifié la session (cookies,
/// jetons), le nouveau jeton part dans l'en-tête `TonAPI-Session`.
pub async fn respond<T, F>(
    state: &AppState,
    original: SessionData,
    fetch: F,
) -> Result<Response, ApiError>
where
    T: Serialize + ToSchema,
    F: AsyncFnOnce(&mut Context<'_>) -> Result<Fetched<T>, Problem>,
{
    let registry = state.registry.load_full();
    let university = registry
        .get(&original.university)
        .ok_or(ErrorCode::UnknownUniversity)?;
    let mut session = original.clone();
    let mut ctx = Context {
        university,
        session: &mut session,
        now: Utc::now(),
        net: tonapi_providers::NetSettings::default(),
    };
    let fetched = fetch(&mut ctx).await?;
    let meta = Meta {
        university: session.university.clone(),
        establishment: session.establishment.as_ref().map(|e| e.id.clone()),
        provider: fetched.provider.to_owned(),
        fetched_at: Utc::now().fixed_offset(),
    };
    let mut response = axum::Json(DataResponse {
        data: fetched.data,
        meta,
    })
    .into_response();
    no_store(&mut response);
    if session != original {
        attach_session(state, &session, &mut response)?;
    }
    Ok(response)
}

/// Réponse personnelle : jamais mise en cache.
pub fn no_store(response: &mut Response) {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
}

/// Ajoute le jeton de `session` dans l'en-tête `TonAPI-Session`.
pub fn attach_session(
    state: &AppState,
    session: &SessionData,
    response: &mut Response,
) -> Result<(), ApiError> {
    let token = state.keys.seal(session).map_err(|_| ErrorCode::Internal)?;
    let value = HeaderValue::from_str(&token).map_err(|_| ErrorCode::Internal)?;
    response.headers_mut().insert(SESSION_HEADER, value);
    Ok(())
}
