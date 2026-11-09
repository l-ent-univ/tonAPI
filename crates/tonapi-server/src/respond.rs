//! Appel d'un fournisseur et mise en forme de la réponse `{ data, meta }`.

use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use serde::Serialize;
use tonapi_core::{ErrorCode, Meta, Problem};
use tonapi_providers::{Context, Download, Fetched};
use tonapi_session::SessionData;
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::{AppState, SESSION_HEADER, access};

/// Réponse de données.
#[derive(Serialize, ToSchema)]
pub struct DataResponse<T: ToSchema> {
    pub data: T,
    pub meta: Meta,
}

/// Exécute `fetch` avec la session de l'étudiant. Si le fournisseur a modifié la session (cookies,
/// jetons), le nouveau jeton part dans l'en-tête `TonAPI-Session`, y compris avec une erreur.
pub async fn respond<T, F>(
    state: &AppState,
    original: SessionData,
    fetch: F,
) -> Result<Response, ApiError>
where
    T: Serialize + ToSchema,
    F: AsyncFnOnce(&mut Context<'_>) -> Result<Fetched<T>, Problem>,
{
    run(state, original, fetch, |fetched, session| {
        let meta = Meta {
            university: session.university.clone(),
            establishment: session.establishment.as_ref().map(|e| e.id.clone()),
            provider: fetched.provider.to_owned(),
            fetched_at: Utc::now().fixed_offset(),
        };
        axum::Json(DataResponse {
            data: fetched.data,
            meta,
        })
        .into_response()
    })
    .await
}

/// Comme [`respond`], pour un fichier servi tel quel, en pièce jointe.
pub async fn respond_file<F>(
    state: &AppState,
    original: SessionData,
    fetch: F,
) -> Result<Response, ApiError>
where
    F: AsyncFnOnce(&mut Context<'_>) -> Result<Fetched<Download>, Problem>,
{
    run(state, original, fetch, |fetched, _| {
        let file = fetched.data;
        // Nom réduit à des caractères sûrs pour l'en-tête.
        let name: String = file
            .file_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let mut response = file.bytes.into_response();
        let headers = response.headers_mut();
        if let Ok(value) = HeaderValue::from_str(&file.content_type) {
            headers.insert(header::CONTENT_TYPE, value);
        }
        if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
            headers.insert(header::CONTENT_DISPOSITION, value);
        }
        headers.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        response
    })
    .await
}

async fn run<T, F>(
    state: &AppState,
    original: SessionData,
    fetch: F,
    finish: impl FnOnce(Fetched<T>, &SessionData) -> Response,
) -> Result<Response, ApiError>
where
    F: AsyncFnOnce(&mut Context<'_>) -> Result<Fetched<T>, Problem>,
{
    let _slot = state.account_slots.acquire(&original)?;
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
    let fetched = match fetch(&mut ctx).await {
        Ok(fetched) => fetched,
        // Même en erreur, un fournisseur a pu renouveler des cookies ou retirer un jeton refusé.
        Err(problem) => {
            let mut response = ApiError::from(problem).into_response();
            response
                .extensions_mut()
                .insert(access::University(original.university.clone()));
            no_store(&mut response);
            if session != original {
                attach_session(state, &session, &mut response)?;
            }
            return Ok(response);
        }
    };
    let provider = fetched.provider;
    let mut response = finish(fetched, &session);
    let extensions = response.extensions_mut();
    extensions.insert(access::University(original.university.clone()));
    extensions.insert(access::Provider(provider));
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
    access::token_issued(&token);
    let value = HeaderValue::from_str(&token).map_err(|_| ErrorCode::Internal)?;
    response.headers_mut().insert(SESSION_HEADER, value);
    Ok(())
}
