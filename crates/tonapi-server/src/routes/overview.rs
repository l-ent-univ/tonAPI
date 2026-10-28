//! `GET /v1/overview` : plusieurs services en un appel, interrogés en parallèle.
//!
//! Chaque service part sur sa propre copie de la session ; les cookies et jetons renouvelés sont
//! fusionnés ensuite (`tonapi_session::SessionData::merged`) et repartent dans un seul `TonAPI-Session`. Le
//! temps de réponse est celui du service le plus lent, pas la somme.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::task::JoinSet;
use tonapi_config::{Registry, ResolvedUniversity, ServiceAvailability, ServiceKind};
use tonapi_core::{ErrorCode, Meta, Problem};
use tonapi_providers::{Context, Fetched, NetSettings};
use tonapi_session::SessionData;
use utoipa::{IntoParams, ToSchema};

use super::data::{Range, resolve_range};
use crate::AppState;
use crate::auth::AuthSession;
use crate::error::{ApiError, invalid};
use crate::respond::{DataResponse, attach_session, no_store};

/// Services que l'appel groupé sait interroger, dans l'ordre de la réponse.
const KEYS: [&str; 10] = [
    "profile",
    "timetable",
    "timetable-groups",
    "grades",
    "absences",
    "exams",
    "homework",
    "mail",
    "apps",
    "documents",
];

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct OverviewQuery {
    /// Services séparés par des virgules (`profile`, `timetable`, `timetable-groups`, `grades`,
    /// `absences`, `exams`, `homework`, `mail`, `apps`, `documents`). Défaut : le profil et
    /// chaque service que l'université sert par l'API.
    services: Option<String>,
    /// Premier jour de l'emploi du temps (`AAAA-MM-JJ`). Défaut : lundi de la semaine en cours.
    from: Option<NaiveDate>,
    /// Dernier jour de l'emploi du temps. Défaut : dimanche qui suit `from`.
    to: Option<NaiveDate>,
}

/// Résultat d'un service : ses données (même forme que la route du service) ou son erreur.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OverviewItem {
    pub service: String,
    /// Fournisseur qui a répondu.
    pub provider: Option<String>,
    /// Données, au format de la route du service (`/v1/grades`…).
    #[schema(value_type = Object)]
    pub data: Option<Value>,
    pub problem: Option<Problem>,
    pub duration_ms: u64,
}

/// Plusieurs services en un appel, interrogés en parallèle.
#[utoipa::path(
    get,
    path = "/v1/overview",
    tag = "données",
    security(("session" = [])),
    params(OverviewQuery),
    responses(
        (status = 200, description = "Un élément par service, avec ses données ou son erreur", body = DataResponse<Vec<OverviewItem>>),
        (status = 400, description = "`invalid_request` : service inconnu ou dates invalides", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn overview(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
    query: Result<Query<OverviewQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(query) = query.map_err(|e| invalid(e.body_text()))?;
    let registry = state.registry.load_full();
    let university = registry
        .get(&session.university)
        .ok_or(ErrorCode::UnknownUniversity)?;
    let (from, to) = resolve_range(
        &Range {
            from: query.from,
            to: query.to,
        },
        &university.university.timezone,
    )?;
    let services = requested(query.services.as_deref(), university, &session)?;
    let (items, sessions) = run_all(&registry, &session, &services, from, to).await?;

    let merged = session.merged(&sessions);
    let meta = Meta {
        university: merged.university.clone(),
        establishment: merged.establishment.as_ref().map(|e| e.id.clone()),
        provider: "overview".to_owned(),
        fetched_at: Utc::now().fixed_offset(),
    };
    let mut response = axum::Json(DataResponse { data: items, meta }).into_response();
    no_store(&mut response);
    if merged != session {
        attach_session(&state, &merged, &mut response)?;
    }
    Ok(response)
}

/// Services demandés (`services=…`), ou par défaut le profil et chaque service servi par l'API.
fn requested(
    list: Option<&str>,
    university: &ResolvedUniversity,
    session: &SessionData,
) -> Result<Vec<&'static str>, ApiError> {
    if let Some(list) = list {
        let mut out = Vec::new();
        for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            let key = KEYS
                .iter()
                .find(|k| **k == name)
                .ok_or_else(|| invalid(format!("service inconnu : `{name}`")))?;
            if !out.contains(key) {
                out.push(*key);
            }
        }
        return Ok(out);
    }
    let available = university
        .services_for(session.establishment.as_ref().map(|e| e.id.as_str()))
        .availability();
    let api = |kind: ServiceKind| {
        available
            .iter()
            .any(|(k, a)| *k == kind && matches!(a, ServiceAvailability::Api { .. }))
    };
    Ok(KEYS
        .iter()
        .copied()
        .filter(|key| match *key {
            "profile" => true,
            "timetable-groups" => false,
            other => ServiceKind::ALL
                .iter()
                .find(|k| k.key() == other)
                .is_some_and(|k| api(*k)),
        })
        .collect())
}

/// Un service et son résultat, à sa place dans la réponse.
type Outcome = (usize, &'static str, Result<(Value, &'static str), Problem>);

/// Lance chaque service sur sa copie de la session ; éléments dans l'ordre demandé et sessions
/// modifiées, à fusionner.
async fn run_all(
    registry: &Arc<Registry>,
    session: &SessionData,
    services: &[&'static str],
    from: NaiveDate,
    to: NaiveDate,
) -> Result<(Vec<OverviewItem>, Vec<SessionData>), ApiError> {
    // Notes et absences partagent une tâche : ScoDoc les donne d'une seule lecture.
    let both = services.contains(&"grades") && services.contains(&"absences");
    let partner = services.iter().position(|k| *k == "absences");
    let mut tasks = JoinSet::new();
    for (index, key) in services.iter().copied().enumerate() {
        if both && key == "absences" {
            continue;
        }
        let paired = both && key == "grades";
        let registry = registry.clone();
        let mut copy = session.clone();
        tasks.spawn(async move {
            let started = Instant::now();
            let results: Vec<Outcome> = match registry.get(&copy.university) {
                Some(university) => {
                    let mut ctx = Context {
                        university,
                        session: &mut copy,
                        now: Utc::now(),
                        net: NetSettings::default(),
                    };
                    if paired {
                        let (g, a) = tonapi_providers::grades_and_absences(&mut ctx).await;
                        vec![
                            (index, key, g.and_then(value)),
                            (partner.unwrap_or(index), "absences", a.and_then(value)),
                        ]
                    } else {
                        vec![(index, key, fetch(key, &mut ctx, from, to).await)]
                    }
                }
                None => vec![(index, key, Err(Problem::new(ErrorCode::UnknownUniversity)))],
            };
            let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            (results, copy, ms)
        });
    }
    let mut items: Vec<(usize, OverviewItem)> = Vec::with_capacity(services.len());
    let mut sessions = Vec::with_capacity(services.len());
    while let Some(joined) = tasks.join_next().await {
        let (results, copy, ms) = joined.map_err(|_| ErrorCode::Internal)?;
        sessions.push(copy);
        for (index, key, result) in results {
            let (provider, data, problem) = match result {
                Ok((data, provider)) => (Some(provider.to_owned()), Some(data), None),
                Err(problem) => (None, None, Some(problem)),
            };
            items.push((
                index,
                OverviewItem {
                    service: key.to_owned(),
                    provider,
                    data,
                    problem,
                    duration_ms: ms,
                },
            ));
        }
    }
    items.sort_by_key(|(index, _)| *index);
    Ok((items.into_iter().map(|(_, item)| item).collect(), sessions))
}

fn value<T: Serialize>(fetched: Fetched<T>) -> Result<(Value, &'static str), Problem> {
    let data = serde_json::to_value(fetched.data).map_err(|_| Problem::new(ErrorCode::Internal))?;
    Ok((data, fetched.provider))
}

async fn fetch(
    key: &str,
    ctx: &mut Context<'_>,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<(Value, &'static str), Problem> {
    use tonapi_providers as p;
    match key {
        "profile" => value(p::profile(ctx).await?),
        "timetable" => value(p::timetable(ctx, from, to).await?),
        "timetable-groups" => value(p::timetable_groups(ctx).await?),
        "grades" => value(p::grades(ctx).await?),
        "absences" => value(p::absences(ctx).await?),
        "exams" => value(p::exams(ctx).await?),
        "homework" => value(p::homework(ctx).await?),
        "mail" => value(p::mail_inbox(ctx).await?),
        "apps" => value(p::apps(ctx).await?),
        "documents" => value(p::documents(ctx).await?),
        _ => Err(Problem::new(ErrorCode::InvalidRequest)),
    }
}
