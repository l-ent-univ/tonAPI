//! Données de l'étudiant connecté.

use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use chrono::{Datelike, Duration, NaiveDate, Utc};
use serde::Deserialize;
use tonapi_config::ServiceKind;
use tonapi_core::{
    AbsenceReport, App, Course, Document, Exam, GradeDistribution, GradeNotice, GradeReport,
    Homework, MailInbox, Problem, TimetableGroup,
};
use utoipa::{IntoParams, ToSchema};

use crate::AppState;
use crate::auth::AuthSession;
use crate::error::{ApiError, invalid};
use crate::respond::{DataResponse, respond, respond_file};

/// Écart maximal entre `from` et `to`, en jours.
const MAX_RANGE_DAYS: i64 = 92;
/// Ressources d'emploi du temps demandées au plus dans une requête.
const MAX_RESOURCES: usize = 30;
/// Longueur maximale d'un identifiant de ressource.
const MAX_RESOURCE_LENGTH: usize = 64;

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

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TimetableQuery {
    /// Premier jour inclus (`AAAA-MM-JJ`). Défaut : lundi de la semaine en cours.
    from: Option<NaiveDate>,
    /// Dernier jour inclus. Défaut : dimanche qui suit `from`.
    to: Option<NaiveDate>,
    /// Ressources dont on veut les cours, séparées par des virgules : identifiants de
    /// `/v1/timetable/groups` ou d'une autre ressource du logiciel (30 au plus). Défaut : les
    /// groupes de l'étudiant. `not_implemented` si le fournisseur ne sait pas le faire.
    resources: Option<String>,
}

/// Identifiants de la liste `resources`, sans vide ni doublon.
fn requested_resources(raw: Option<&str>) -> Result<Vec<String>, ApiError> {
    let mut resources: Vec<String> = Vec::new();
    for id in raw.unwrap_or_default().split(',').map(str::trim) {
        if id.is_empty() || resources.iter().any(|known| known == id) {
            continue;
        }
        let plain = id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if !plain || id.len() > MAX_RESOURCE_LENGTH {
            return Err(invalid("`resources` : identifiant de ressource invalide"));
        }
        if resources.len() == MAX_RESOURCES {
            return Err(invalid(format!(
                "`resources` : {MAX_RESOURCES} ressources au plus"
            )));
        }
        resources.push(id.to_owned());
    }
    Ok(resources)
}

/// Cours entre deux dates incluses : ceux de l'étudiant, ou ceux des ressources demandées.
#[utoipa::path(
    get,
    path = "/v1/timetable",
    tag = "données",
    security(("session" = [])),
    params(TimetableQuery),
    responses(
        (status = 200, description = "Succès", body = DataResponse<Vec<Course>>),
        (status = 400, description = "`invalid_request` : paramètres invalides", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`service_absent`", body = Problem, content_type = "application/problem+json"),
        (status = 501, description = "`not_implemented` : ce fournisseur ne lit pas une ressource choisie", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn timetable(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
    query: Result<Query<TimetableQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(query) = query.map_err(|e| invalid(e.body_text()))?;
    let resources = requested_resources(query.resources.as_deref())?;
    let range = Range {
        from: query.from,
        to: query.to,
    };
    let timezone = state
        .registry
        .load()
        .get(&session.university)
        .map(|u| u.university.timezone.clone())
        .unwrap_or_default();
    let (from, to) = resolve_range(&range, &timezone)?;
    respond(&state, session, Some(ServiceKind::Timetable), async |ctx| {
        tonapi_providers::timetable_of(ctx, from, to, &resources).await
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
    respond(&state, session, Some(ServiceKind::Timetable), async |ctx| {
        tonapi_providers::timetable_groups(ctx).await
    })
    .await
}

/// Longueur maximale d'un identifiant de semestre, d'évaluation ou de département.
const MAX_ID_LENGTH: usize = 64;

/// Identifiant passé par le client (semestre, évaluation, département) : court et sans séparateur.
fn plain_id(id: &str, what: &str) -> Result<String, ApiError> {
    let id = id.trim();
    let plain = id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if id.is_empty() || id.len() > MAX_ID_LENGTH || !plain {
        return Err(invalid(format!("{what} invalide")));
    }
    Ok(id.to_owned())
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct GradesQuery {
    /// Semestre dont on veut le relevé : un `id` de `available` ou d'un semestre déjà rendu.
    /// Défaut : le semestre courant, ou tous ceux que le logiciel donne d'un coup.
    semester: Option<String>,
}

/// Relevé de notes.
#[utoipa::path(
    get,
    path = "/v1/grades",
    tag = "données",
    security(("session" = [])),
    params(GradesQuery),
    responses(
        (status = 200, description = "Succès", body = DataResponse<GradeReport>),
        (status = 400, description = "`invalid_request` : identifiant de semestre invalide", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`service_absent`, avec `link` quand les notes ne sont qu'un lien ; `not_found` : semestre inconnu", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn grades(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
    query: Result<Query<GradesQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(query) = query.map_err(|e| invalid(e.body_text()))?;
    let semester = query
        .semester
        .as_deref()
        .filter(|semester| !semester.trim().is_empty())
        .map(|semester| plain_id(semester, "`semester`"))
        .transpose()?;
    respond(&state, session, Some(ServiceKind::Grades), async |ctx| {
        tonapi_providers::grades_of(ctx, semester.as_deref()).await
    })
    .await
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct NoticeQuery {
    /// Sigle du département, tel que `department` dans un semestre du relevé.
    department: String,
}

/// Message d'un département à afficher avec ses relevés.
#[utoipa::path(
    get,
    path = "/v1/grades/notice",
    tag = "données",
    security(("session" = [])),
    params(NoticeQuery),
    responses(
        (status = 200, description = "Succès ; `message` absent si le département n'a rien écrit", body = DataResponse<GradeNotice>),
        (status = 400, description = "`invalid_request` : département absent ou invalide", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`service_absent`", body = Problem, content_type = "application/problem+json"),
        (status = 501, description = "`not_implemented` : ce fournisseur de notes n'a pas de message", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn grade_notice(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
    query: Result<Query<NoticeQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(query) = query.map_err(|e| invalid(e.body_text()))?;
    let department = plain_id(&query.department, "`department`")?;
    respond(&state, session, Some(ServiceKind::Grades), async |ctx| {
        tonapi_providers::grade_notice(ctx, &department).await
    })
    .await
}

/// Répartition des notes de la promotion à une évaluation du relevé d'un semestre.
#[utoipa::path(
    get,
    path = "/v1/grades/{semester}/evaluations/{evaluation}/distribution",
    tag = "données",
    security(("session" = [])),
    params(
        ("semester" = String, Path, description = "Identifiant du semestre"),
        ("evaluation" = String, Path, description = "Identifiant de l'évaluation, lu dans le relevé de ce semestre"),
    ),
    responses(
        (status = 200, description = "Succès ; `unavailable` dit pourquoi il n'y a pas de répartition", body = DataResponse<GradeDistribution>),
        (status = 400, description = "`invalid_request` : identifiant invalide", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`not_found` : évaluation absente du relevé de l'étudiant, ou `service_absent`", body = Problem, content_type = "application/problem+json"),
        (status = 501, description = "`not_implemented` : ce fournisseur de notes n'a pas de répartition", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn grade_distribution(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
    Path((semester, evaluation)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let semester = plain_id(&semester, "semestre")?;
    let evaluation = plain_id(&evaluation, "évaluation")?;
    respond(&state, session, Some(ServiceKind::Grades), async |ctx| {
        tonapi_providers::grade_distribution(ctx, &semester, &evaluation).await
    })
    .await
}

/// Relevé PDF d'un semestre, en pièce jointe.
#[utoipa::path(
    get,
    path = "/v1/grades/{semester}/transcript",
    tag = "données",
    security(("session" = [])),
    params(("semester" = String, Path, description = "Identifiant du semestre")),
    responses(
        (status = 200, description = "Relevé du semestre", body = File, content_type = "application/pdf"),
        (status = 400, description = "`invalid_request` : identifiant invalide", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "`invalid_token` ou `session_expired`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "`not_found` : pas de PDF pour ce semestre, ou `service_absent`", body = Problem, content_type = "application/problem+json"),
        (status = 501, description = "`not_implemented` : ce fournisseur de notes n'a pas de PDF", body = Problem, content_type = "application/problem+json"),
    )
)]
pub async fn grade_transcript(
    State(state): State<AppState>,
    AuthSession(session): AuthSession,
    Path(semester): Path<String>,
) -> Result<Response, ApiError> {
    let semester = plain_id(&semester, "semestre")?;
    respond_file(&state, session, async |ctx| {
        tonapi_providers::grade_transcript(ctx, &semester).await
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
    respond(&state, session, Some(ServiceKind::Absences), async |ctx| {
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
    respond(&state, session, Some(ServiceKind::Homework), async |ctx| {
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
    respond(&state, session, Some(ServiceKind::Mail), async |ctx| {
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
    respond(&state, session, Some(ServiceKind::Apps), async |ctx| {
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
    respond(&state, session, Some(ServiceKind::Exams), async |ctx| {
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
    respond(&state, session, Some(ServiceKind::Documents), async |ctx| {
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
