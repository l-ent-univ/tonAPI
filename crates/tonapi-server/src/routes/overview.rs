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
    let groups = groups(&services, university, &session);
    let (items, sessions) = run_all(&registry, &session, groups, from, to).await?;

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

/// Un service, son résultat et sa durée, à sa place dans la réponse.
type Outcome = (
    usize,
    &'static str,
    Result<(Value, &'static str), Problem>,
    u64,
);

/// Services d'une tâche : leur place dans la réponse et leur nom.
type Group = Vec<(usize, &'static str)>;

/// Services regroupés par fournisseur. Ceux d'un même fournisseur passent l'un après l'autre
/// dans une même tâche : un service web n'accepte pas toujours deux ouvertures simultanées de la
/// même session (ADE MyPlanning répond 500, une session PHP sérialise). Les groupes, eux,
/// partent en parallèle.
fn groups(
    services: &[&'static str],
    university: &ResolvedUniversity,
    session: &SessionData,
) -> Vec<Group> {
    let available = university
        .services_for(session.establishment.as_ref().map(|e| e.id.as_str()))
        .availability();
    let provider = |key: &str| -> Option<String> {
        let kind = match key {
            "profile" => ServiceKind::Profile,
            "timetable-groups" => ServiceKind::Timetable,
            other => *ServiceKind::ALL.iter().find(|k| k.key() == other)?,
        };
        available.iter().find_map(|(k, a)| match a {
            ServiceAvailability::Api { provider } if *k == kind => Some(provider.clone()),
            _ => None,
        })
    };
    let mut out: Vec<(Option<String>, Group)> = Vec::new();
    for (index, key) in services.iter().copied().enumerate() {
        match provider(key) {
            Some(p) => match out
                .iter_mut()
                .find(|(g, _)| g.as_deref() == Some(p.as_str()))
            {
                Some((_, members)) => members.push((index, key)),
                None => out.push((Some(p), vec![(index, key)])),
            },
            // Service absent ou simple lien : réponse immédiate, sans voisin.
            None => out.push((None, vec![(index, key)])),
        }
    }
    out.into_iter().map(|(_, members)| members).collect()
}

/// Lance chaque groupe sur sa copie de la session ; éléments dans l'ordre demandé et sessions
/// modifiées, à fusionner.
async fn run_all(
    registry: &Arc<Registry>,
    session: &SessionData,
    groups: Vec<Group>,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<(Vec<OverviewItem>, Vec<SessionData>), ApiError> {
    let mut tasks = JoinSet::new();
    // Membres de chaque tâche : si elle panique, seuls ses services passent en erreur.
    let mut members_of = std::collections::HashMap::new();
    for members in groups {
        let registry = registry.clone();
        let mut copy = session.clone();
        let spawned_members = members.clone();
        let handle = tasks.spawn(async move {
            let mut results: Vec<Outcome> = Vec::with_capacity(members.len());
            let Some(university) = registry.get(&copy.university) else {
                for (index, key) in members {
                    results.push((
                        index,
                        key,
                        Err(Problem::new(ErrorCode::UnknownUniversity)),
                        0,
                    ));
                }
                return (results, copy);
            };
            let mut ctx = Context {
                university,
                session: &mut copy,
                now: Utc::now(),
                net: NetSettings::default(),
            };
            // Notes et absences d'une même passerelle : une seule lecture (ScoDoc).
            let absences = members.iter().find(|(_, k)| *k == "absences").copied();
            let paired = absences.is_some() && members.iter().any(|(_, k)| *k == "grades");
            let group_start = Instant::now();
            for (index, key) in members {
                let started = Instant::now();
                if paired && key == "absences" {
                    continue;
                }
                // Temps restant du groupe : un service lent ne fait échouer que lui et les
                // suivants du même fournisseur, jamais toute la réponse.
                let remaining = GROUP_BUDGET.saturating_sub(group_start.elapsed());
                if paired && key == "grades" {
                    let (g, a) = tokio::time::timeout(
                        remaining,
                        tonapi_providers::grades_and_absences(&mut ctx),
                    )
                    .await
                    .unwrap_or_else(|_| (Err(too_slow()), Err(too_slow())));
                    let ms = elapsed(started);
                    results.push((index, key, g.and_then(value), ms));
                    if let Some((at, name)) = absences {
                        results.push((at, name, a.and_then(value), ms));
                    }
                    continue;
                }
                let result = tokio::time::timeout(remaining, fetch(key, &mut ctx, from, to))
                    .await
                    .unwrap_or_else(|_| Err(too_slow()));
                results.push((index, key, result, elapsed(started)));
            }
            (results, copy)
        });
        members_of.insert(handle.id(), spawned_members);
    }
    let mut items: Vec<(usize, OverviewItem)> = Vec::new();
    let mut sessions = Vec::new();
    while let Some(joined) = tasks.join_next_with_id().await {
        let results = match joined {
            Ok((_, (results, copy))) => {
                sessions.push(copy);
                results
            }
            Err(error) => members_of
                .remove(&error.id())
                .unwrap_or_default()
                .into_iter()
                .map(|(index, key)| (index, key, Err(Problem::new(ErrorCode::Internal)), 0))
                .collect(),
        };
        for (index, key, result, ms) in results {
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

/// Temps accordé aux services d'un même fournisseur, sous le délai de la route (60 s).
const GROUP_BUDGET: std::time::Duration = std::time::Duration::from_secs(50);

fn too_slow() -> Problem {
    Problem::new(ErrorCode::ServiceUnavailable)
        .detail("le service de l'université n'a pas répondu à temps")
}

fn elapsed(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tonapi_config::UniversityFile;

    use super::*;

    fn university() -> ResolvedUniversity {
        let file: UniversityFile = serde_json::from_value(serde_json::json!({
            "id": "test",
            "name": "Université de test",
            "sso": { "kind": "demo" },
            "services": {
                "timetable": { "provider": "ade-myplanning", "base-url": "https://ade.exemple.test/" },
                "grades": { "provider": "etudiants-api", "base-url": "https://scol.exemple.test/" },
                "exams": { "provider": "etudiants-api", "base-url": "https://scol.exemple.test/" },
                "mail": { "provider": "link", "url": "https://mail.exemple.test/" }
            }
        }))
        .unwrap();
        ResolvedUniversity {
            dir: std::path::PathBuf::new(),
            university: file,
            establishments: BTreeMap::new(),
        }
    }

    fn session() -> SessionData {
        SessionData {
            university: "test".to_owned(),
            username: "e".to_owned(),
            establishment: None,
            cookies: Vec::new(),
            tokens: BTreeMap::new(),
            pending: false,
            issued_at: 0,
            expires_at: 1,
        }
    }

    #[test]
    fn un_fournisseur_une_tache() {
        let services = ["timetable", "grades", "timetable-groups", "mail", "exams"];
        let groups = groups(&services, &university(), &session());
        assert_eq!(
            groups,
            [
                vec![(0, "timetable"), (2, "timetable-groups")],
                vec![(1, "grades"), (4, "exams")],
                vec![(3, "mail")],
            ]
        );
    }
}
