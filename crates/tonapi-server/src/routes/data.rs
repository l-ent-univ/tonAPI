//! Données de l'étudiant connecté.

use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use chrono::{Datelike, Duration, NaiveDate, Utc};
use serde::Deserialize;
use tonapi_core::{
    AbsenceReport, App, Course, Document, Exam, GradeReport, Homework, MailInbox, Problem,
    TimetableGroup,
};
use utoipa::{IntoParams, ToSchema};

use crate::AppState;
use crate::auth::AuthSession;
use crate::error::{ApiError, invalid};
use crate::respond::{DataResponse, respond, respond_file};

/// Écart maximal entre `from` et `to`, en jours.
const MAX_RANGE_DAYS: i64 = 92;

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct Range {
    /// Premier jour inclus (`AAAA-MM-JJ`). Défaut : lundi de la semaine en cours.
    pub(crate) from: Option<NaiveDate>,
    /// Dernier jour inclus. Défaut : dimanche qui suit `from`.
    pub(crate) to: Option<NaiveDate>,
}

pub(crate) fn resolve_range(
    range: &Range,
    timezone: &str,
) -> Result<(NaiveDate, NaiveDate), ApiError> {
    let today = timezone.parse::<chrono_tz::Tz>().map_or_else(
        |_| Utc::now().date_naive(),
        |tz| Utc::now().with_timezone(&tz).date_naive(),
    );
    let monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let from = range.from.unwrap_or(monday);
    // Dates extrêmes acceptées par le format : refusées avant tout calcul (débordement).
    let plausible = |d: NaiveDate| (1900..=2200).contains(&d.year());
    if !plausible(from) || range.to.is_some_and(|to| !plausible(to)) {
        return Err(invalid("dates entre 1900 et 2200 seulement"));
    }
    let to = range.to.unwrap_or(from + Duration::days(6));
    if to < from {
        return Err(invalid("`to` précède `from`"));
    }
    if (to - from).num_days() > MAX_RANGE_DAYS {
        return Err(invalid(format!(
            "{MAX_RANGE_DAYS} jours au plus entre `from` et `to`"
        )));
    }
    Ok((from, to))
}

/// Cours entre deux dates incluses.
#[utoipa::path(
    get,
    path = "/v1/timetable",
    tag = "données",
    security(("session" = [])),
    params(Range),
    responses(
        (status = 200, description = "Succès", body = DataResponse<Vec<Course>>),
        (status = 400, description = "`invalid_request` : paramètres invalides", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`service_absent`", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn timetable(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
    query: Result<Query<Range>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(range) = query.map_err(|e| invalid(e.body_text()))?;
    let timezone = state
        .registry
        .load()
        .get(&session.university)
        .map(|u| u.university.timezone.clone())
        .unwrap_or_default();
    let (from, to) = resolve_range(&range, &timezone)?;
    respond(&state, session, async |ctx| {
        tonapi_providers::timetable(ctx, from, to).await
    })
    .await
}

/// Groupes d'emploi du temps et ceux que suit l'étudiant.
#[utoipa::path(
    get,
    path = "/v1/timetable/groups",
    tag = "données",
    security(("session" = [])),
    responses(
        (status = 200, description = "Succès", body = DataResponse<Vec<TimetableGroup>>),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`service_absent` (avec `link` si le service est un simple lien) ou ressource inconnue", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn timetable_groups(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
) -> Result<Response, ApiError> {
    respond(&state, session, async |ctx| {
        tonapi_providers::timetable_groups(ctx).await
    })
    .await
}

/// Relevé de notes.
#[utoipa::path(
    get,
    path = "/v1/grades",
    tag = "données",
    security(("session" = [])),
    responses(
        (status = 200, description = "Succès", body = DataResponse<GradeReport>),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`service_absent`, avec `link` quand les notes ne sont qu'un lien", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn grades(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
) -> Result<Response, ApiError> {
    respond(&state, session, async |ctx| {
        tonapi_providers::grades(ctx).await
    })
    .await
}

/// Absences et totaux.
#[utoipa::path(
    get,
    path = "/v1/absences",
    tag = "données",
    security(("session" = [])),
    responses(
        (status = 200, description = "Succès", body = DataResponse<AbsenceReport>),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`service_absent` (avec `link` si le service est un simple lien) ou ressource inconnue", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn absences(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
) -> Result<Response, ApiError> {
    respond(&state, session, async |ctx| {
        tonapi_providers::absences(ctx).await
    })
    .await
}

/// Devoirs et échéances.
#[utoipa::path(
    get,
    path = "/v1/homework",
    tag = "données",
    security(("session" = [])),
    responses(
        (status = 200, description = "Succès", body = DataResponse<Vec<Homework>>),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`service_absent` (avec `link` si le service est un simple lien) ou ressource inconnue", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn homework(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
) -> Result<Response, ApiError> {
    respond(&state, session, async |ctx| {
        tonapi_providers::homework(ctx).await
    })
    .await
}

/// Derniers messages reçus.
#[utoipa::path(
    get,
    path = "/v1/mail/inbox",
    tag = "données",
    security(("session" = [])),
    responses(
        (status = 200, description = "Succès", body = DataResponse<MailInbox>),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`service_absent` (avec `link` si le service est un simple lien) ou ressource inconnue", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn mail_inbox(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
) -> Result<Response, ApiError> {
    respond(&state, session, async |ctx| {
        tonapi_providers::mail_inbox(ctx).await
    })
    .await
}

/// Applications du portail.
#[utoipa::path(
    get,
    path = "/v1/apps",
    tag = "données",
    security(("session" = [])),
    responses(
        (status = 200, description = "Succès", body = DataResponse<Vec<App>>),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`service_absent` (avec `link` si le service est un simple lien) ou ressource inconnue", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn apps(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
) -> Result<Response, ApiError> {
    respond(&state, session, async |ctx| {
        tonapi_providers::apps(ctx).await
    })
    .await
}

/// Épreuves du calendrier d'examens.
#[utoipa::path(
    get,
    path = "/v1/exams",
    tag = "données",
    security(("session" = [])),
    responses(
        (status = 200, description = "Succès", body = DataResponse<Vec<Exam>>),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`service_absent` (avec `link` si le service est un simple lien) ou ressource inconnue", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn exams(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
) -> Result<Response, ApiError> {
    respond(&state, session, async |ctx| {
        tonapi_providers::exams(ctx).await
    })
    .await
}

/// Documents administratifs : certificat, attestations.
#[utoipa::path(
    get,
    path = "/v1/documents",
    tag = "données",
    security(("session" = [])),
    responses(
        (status = 200, description = "Succès", body = DataResponse<Vec<Document>>),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`service_absent` (avec `link` si le service est un simple lien) ou ressource inconnue", body = Problem, content_type = "application/problem+json"),
        (status = 501, description = "`not_implemented` : fournisseur de documents pas encore pris en charge", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn documents(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
) -> Result<Response, ApiError> {
    respond(&state, session, async |ctx| {
        tonapi_providers::documents(ctx).await
    })
    .await
}

/// Fichier binaire (PDF le plus souvent).
#[derive(ToSchema)]
#[schema(value_type = String, format = Binary)]
pub struct File(
    #[allow(
        dead_code,
        reason = "type de schéma OpenAPI seulement : le champ n'est jamais lu"
    )]
    Vec<u8>,
);

/// Fichier d'un document marqué `downloadable`, en pièce jointe.
#[utoipa::path(
    get,
    path = "/v1/documents/{id}",
    tag = "données",
    security(("session" = [])),
    params(("id" = String, Path, description = "Identifiant donné par `GET /v1/documents`")),
    responses(
        (status = 200, description = "Fichier du document", body = File, content_type = "application/pdf"),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`not_found` : document inconnu, ou `service_absent`", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn document(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    respond_file(&state, session, async |ctx| {
        tonapi_providers::document(ctx, &id).await
    })
    .await
}
