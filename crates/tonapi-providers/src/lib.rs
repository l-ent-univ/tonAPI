//! Fournisseurs de données. Chaque fonction choisit le fournisseur configuré pour le service de
//! l'étudiant (université + composante) et convertit sa réponse vers les modèles de `tonapi-core`.
//!
//! Un service absent donne `service_absent` (avec le lien quand il y en a un) ; une fonction qu'un
//! fournisseur ne peut pas assurer (groupes d'un lien iCal, par exemple) donne `not_implemented`.

mod ade;
mod ardemat;
mod bluemind;
mod cache;
mod common;
mod demo;
mod etudiants_api;
mod moodle;
mod net;
mod scodoc;
mod sso;
mod tiles_portal;
mod uportal;
mod zimbra;

use chrono::{DateTime, NaiveDate, Utc};
use tonapi_config::{
    AbsencesService, AppsService, DocumentsService, ExamsService, GradesService, HomeworkService,
    MailService, PortalConfig, ProfileService, ResolvedUniversity, ServiceAvailability,
    ServiceKind, Services, Signal, SignalSource, StudentRecord, TimetableService,
};
use tonapi_core::{
    AbsenceReport, App, Course, Document, ErrorCode, EstablishmentSource, Exam, GradeDistribution,
    GradeNotice, GradeReport, Homework, MailInbox, Problem, Profile, TimetableGroup,
};
use tonapi_session::{EstablishmentPick, SessionData};

pub use net::NetSettings;
pub use sso::{
    LoginInput, LoginOutcome, MfaAction, MfaPrompt, complete_mfa, login, logout, refresh,
    still_signed_in,
};
pub use tonapi_http::otp::OtpMethods;

/// Ce dont un fournisseur a besoin pour une requête.
pub struct Context<'a> {
    pub university: &'a ResolvedUniversity,
    pub session: &'a mut SessionData,
    pub now: DateTime<Utc>,
    pub net: NetSettings,
}

impl Context<'_> {
    pub(crate) fn services(&self) -> Services {
        self.university
            .services_for(self.session.establishment.as_ref().map(|e| e.id.as_str()))
    }
}

/// Client HTTP impossible à construire (certificat supplémentaire illisible…) : le détail, qui
/// contient un chemin du serveur, va dans le journal, pas dans la réponse.
pub(crate) fn client_problem(error: &tonapi_http::HttpError) -> Problem {
    tracing::error!(%error, "client HTTP impossible à construire");
    Problem::new(ErrorCode::Internal).detail("configuration TLS de l'université illisible")
}

/// Durée par défaut pendant laquelle un client peut garder la réponse d'un service : ce qui change
/// peu se garde longtemps, la boîte de réception presque pas.
const fn default_keep_secs(kind: ServiceKind) -> u32 {
    match kind {
        ServiceKind::Mail => 60,
        ServiceKind::Timetable | ServiceKind::Homework => 5 * 60,
        ServiceKind::Grades
        | ServiceKind::Absences
        | ServiceKind::Exams
        | ServiceKind::Documents => 10 * 60,
        ServiceKind::Apps | ServiceKind::Profile => 60 * 60,
    }
}

/// Au-delà d'un jour, une réponse gardée n'est plus celle du jour.
const MAX_KEEP_SECS: u32 = 24 * 60 * 60;

/// Durée, en secondes, pendant laquelle un client peut resservir la réponse d'un service sans la
/// redemander : `options.cache-ttl-secs` de la configuration, sinon un défaut par service. `None`
/// pour un service absent, un simple lien, la démo, ou `cache-ttl-secs = 0`.
///
/// C'est une indication donnée au client (`meta.cacheSecs`) : ni le serveur ni les fournisseurs
/// ne gardent de réponse.
#[must_use]
pub fn keep_secs(services: &Services, kind: ServiceKind) -> Option<u32> {
    let remote = services.availability().into_iter().any(|(found, how)| {
        found == kind && matches!(how, ServiceAvailability::Api { provider } if provider != "demo")
    });
    if !remote {
        return None;
    }
    match services
        .options(kind)
        .and_then(|options| options.cache_ttl_secs)
    {
        Some(0) => None,
        Some(secs) => Some(secs.min(MAX_KEEP_SECS)),
        None => Some(default_keep_secs(kind)),
    }
}

/// Fichier servi tel quel (attestation PDF…).
#[derive(Debug, Clone)]
pub struct Download {
    pub content_type: String,
    pub file_name: String,
    pub bytes: Vec<u8>,
}

/// Données et fournisseur qui les a produites.
#[derive(Debug, Clone)]
pub struct Fetched<T> {
    pub data: T,
    pub provider: &'static str,
}

fn fetched<T>(data: T, provider: &'static str) -> Fetched<T> {
    Fetched { data, provider }
}

/// `service_absent`, avec le lien si le service n'existe que sous cette forme.
fn absent(services: &Services, kind: ServiceKind) -> Problem {
    let problem = Problem::new(ErrorCode::ServiceAbsent)
        .detail(format!("le service `{}` n'est pas proposé", kind.key()));
    match services
        .availability()
        .into_iter()
        .find(|(k, _)| *k == kind)
    {
        Some((_, ServiceAvailability::Link { url })) => problem.link(url.as_str()),
        _ => problem,
    }
}

fn mon_dossier_web_pending() -> Problem {
    Problem::new(ErrorCode::NotImplemented)
        .detail("le fournisseur `mon-dossier-web` n'est pas encore implémenté")
}

fn tiles(ctx: &Context<'_>) -> bool {
    matches!(
        ctx.university.university.portal,
        PortalConfig::TilesPortal { .. }
    )
}

/// Une composante au moins a une règle sur ce signal : sinon, inutile d'aller le chercher.
fn wanted(university: &ResolvedUniversity, source: SignalSource) -> bool {
    university.establishments.values().any(|file| {
        file.affiliation
            .rules
            .iter()
            .any(|rule| rule.signal == source)
    })
}

/// Signaux de rattachement : attributs du portail à tuiles, et chemin des groupes de l'étudiant
/// dans le portail ADE quand une règle `ade-group-path` existe. Une source en échec n'empêche pas
/// l'autre.
async fn signals(ctx: &mut Context<'_>) -> Vec<Signal> {
    let mut out = Vec::new();
    if tiles(ctx)
        && let Ok(found) = tiles_portal::fetch_signals(ctx).await
    {
        out.extend(found);
    }
    if wanted(ctx.university, SignalSource::AdeGroupPath)
        && let Some(TimetableService::AdePortalRest { base_url, .. }) = ctx.services().timetable
        && let Ok(found) = ade::portal_rest::fetch_signals(ctx, &base_url).await
    {
        out.extend(found);
    }
    out
}

/// Composante dont les règles obtiennent le meilleur score, s'il est unique et non nul. La
/// confiance est la part des poids de ses règles qui correspondent.
#[must_use]
pub fn detect_establishment(
    university: &ResolvedUniversity,
    signals: &[Signal],
) -> Option<EstablishmentPick> {
    let mut scored: Vec<(&String, u32, u32)> = university
        .establishments
        .iter()
        .map(|(id, file)| {
            let total = file
                .affiliation
                .rules
                .iter()
                .map(|r| u32::from(r.weight))
                .sum::<u32>();
            (id, file.affiliation.score(signals), total)
        })
        .filter(|(_, score, _)| *score > 0)
        .collect();
    scored.sort_by_key(|(_, score, _)| std::cmp::Reverse(*score));
    match scored.as_slice() {
        [(id, score, total), rest @ ..] if rest.first().is_none_or(|next| next.1 < *score) => {
            #[allow(clippy::cast_precision_loss)]
            let confidence = (*score as f32 / (*total).max(1) as f32).min(1.0);
            Some(EstablishmentPick {
                id: (*id).clone(),
                source: EstablishmentSource::Detected,
                confidence,
            })
        }
        _ => None,
    }
}

/// Après la connexion : sans choix de l'étudiant, détecte sa composante d'après son profil. Un
/// échec garde la composante par défaut.
///
/// La connexion par tonAPI l'appelle elle-même. Un client qui bâtit sa `SessionData` à partir de
/// cookies obtenus ailleurs doit l'appeler une fois, puis garder la composante rendue : sans elle,
/// les services d'une composante (notes d'un IUT) ne sont jamais atteints. La détection interroge
/// l'université (portail, arbre ADE) : elle peut prendre plusieurs secondes et n'est pas à refaire
/// à chaque requête. Les cookies et jetons reçus pendant la détection sont dans la session rendue.
pub async fn with_detected_establishment(
    university: &ResolvedUniversity,
    mut session: SessionData,
    now: DateTime<Utc>,
    net: NetSettings,
) -> SessionData {
    let chosen = session
        .establishment
        .as_ref()
        .is_some_and(|e| e.source == EstablishmentSource::Student);
    if chosen || university.establishments.is_empty() {
        return session;
    }
    let mut ctx = Context {
        university,
        session: &mut session,
        now,
        net,
    };
    let signals = signals(&mut ctx).await;
    if let Some(pick) = detect_establishment(university, &signals) {
        ctx.session.establishment = Some(pick);
    }
    session
}

/// Composante choisie par l'étudiant, sinon celle par défaut de l'université.
pub fn pick_establishment(
    university: &ResolvedUniversity,
    requested: Option<&str>,
) -> Result<Option<EstablishmentPick>, Problem> {
    if let Some(id) = requested {
        if !university.establishments.contains_key(id) {
            return Err(
                Problem::new(ErrorCode::UnknownEstablishment).detail(format!(
                    "`{id}` n'est pas une composante de `{}`",
                    university.university.id
                )),
            );
        }
        return Ok(Some(EstablishmentPick {
            id: id.to_owned(),
            source: EstablishmentSource::Student,
            confidence: 1.0,
        }));
    }
    Ok(university
        .default_establishment()
        .map(|e| EstablishmentPick {
            id: e.id.clone(),
            source: EstablishmentSource::Default,
            confidence: 0.0,
        }))
}

pub async fn profile(ctx: &mut Context<'_>) -> Result<Fetched<Profile>, Problem> {
    let services = ctx.services();
    match &services.profile {
        Some(ProfileService::Demo) => Ok(fetched(demo::profile(&ctx.session.username), "demo")),
        Some(ProfileService::Portal { record }) => {
            let (mut profile, provider) = if tiles(ctx) {
                (tiles_portal::fetch_profile(ctx).await?, "tiles-portal")
            } else {
                (uportal::fetch_profile(ctx).await?, "portal")
            };
            // Le dossier complète le profil ; s'il est indisponible, le profil du portail reste.
            if let Some(StudentRecord::EtudiantsApi { base_url }) = record
                && let Ok(found) = etudiants_api::fetch_record(ctx, base_url).await
            {
                profile.student_number = found.student_number.or(profile.student_number);
                profile.program = found.program.or(profile.program);
            }
            Ok(fetched(profile, provider))
        }
        Some(ProfileService::None) | None => Err(absent(&services, ServiceKind::Profile)),
    }
}

/// Emploi du temps de l'étudiant : ses propres groupes.
pub async fn timetable(
    ctx: &mut Context<'_>,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Fetched<Vec<Course>>, Problem> {
    timetable_of(ctx, from, to, &[]).await
}

/// Emploi du temps des ressources demandées (identifiants de `timetable_groups`, ou toute autre
/// ressource du logiciel : groupe, salle, enseignant). Sans ressource : comme [`timetable`].
///
/// Seul `ade-portal-rest` (et la démo) sait interroger une ressource choisie ; les autres
/// fournisseurs répondent `not_implemented` plutôt que l'emploi du temps de l'étudiant.
pub async fn timetable_of(
    ctx: &mut Context<'_>,
    from: NaiveDate,
    to: NaiveDate,
    resources: &[String],
) -> Result<Fetched<Vec<Course>>, Problem> {
    let services = ctx.services();
    let by_resource = |provider: &str| {
        Problem::new(ErrorCode::NotImplemented).detail(format!(
            "{provider} : l'emploi du temps d'une ressource choisie n'est pas disponible"
        ))
    };
    match &services.timetable {
        Some(TimetableService::Demo) if !resources.is_empty() => {
            Ok(fetched(demo::timetable_of(from, to, resources), "demo"))
        }
        Some(TimetableService::AdeIcalLink { .. }) if !resources.is_empty() => {
            Err(by_resource("ade-ical-link"))
        }
        Some(TimetableService::AdeMyplanning { .. }) if !resources.is_empty() => {
            Err(by_resource("ade-myplanning"))
        }
        Some(TimetableService::Demo) => Ok(fetched(demo::timetable(from, to), "demo")),
        Some(TimetableService::AdeIcalLink {
            link_page,
            feed_host,
            ..
        }) => {
            let courses =
                ade::ical_link::fetch_timetable(ctx, link_page, feed_host, from, to).await?;
            Ok(fetched(courses, "ade-ical-link"))
        }
        Some(TimetableService::AdePortalRest { base_url, .. }) => {
            let courses =
                ade::portal_rest::fetch_timetable(ctx, base_url, from, to, resources).await?;
            Ok(fetched(courses, "ade-portal-rest"))
        }
        Some(TimetableService::AdeMyplanning {
            base_url,
            project_id,
            ..
        }) => {
            let courses =
                ade::myplanning::fetch_timetable(ctx, base_url, *project_id, from, to).await?;
            Ok(fetched(courses, "ade-myplanning"))
        }
        Some(TimetableService::Link(_) | TimetableService::None) | None => {
            Err(absent(&services, ServiceKind::Timetable))
        }
    }
}

pub async fn timetable_groups(
    ctx: &mut Context<'_>,
) -> Result<Fetched<Vec<TimetableGroup>>, Problem> {
    let services = ctx.services();
    match &services.timetable {
        Some(TimetableService::Demo) => Ok(fetched(demo::groups(), "demo")),
        Some(TimetableService::AdeIcalLink { .. }) => Err(ade::ical_link::groups_unsupported()),
        Some(TimetableService::AdePortalRest { base_url, .. }) => {
            let groups = ade::portal_rest::fetch_groups(ctx, base_url).await?;
            Ok(fetched(groups, "ade-portal-rest"))
        }
        Some(TimetableService::AdeMyplanning {
            base_url,
            project_id,
            ..
        }) => Ok(fetched(
            ade::myplanning::fetch_groups(ctx, base_url, *project_id).await?,
            "ade-myplanning",
        )),
        Some(TimetableService::Link(_) | TimetableService::None) | None => {
            Err(absent(&services, ServiceKind::Timetable))
        }
    }
}

/// Relevé de notes : le semestre courant, ou tous ceux que le logiciel donne d'un coup.
pub async fn grades(ctx: &mut Context<'_>) -> Result<Fetched<GradeReport>, Problem> {
    grades_of(ctx, None).await
}

/// Le relevé réduit au semestre demandé ; `not_found` s'il n'y est pas.
fn only_semester(mut report: GradeReport, semester: Option<&str>) -> Result<GradeReport, Problem> {
    let Some(id) = semester else {
        return Ok(report);
    };
    report.semesters.retain(|found| found.id == id);
    if report.semesters.is_empty() {
        return Err(Problem::new(ErrorCode::NotFound).detail("semestre inconnu"));
    }
    Ok(report)
}

/// Relevé du semestre demandé (un identifiant de `GradeReport::available` ou d'un semestre déjà
/// rendu). Sans semestre : comme [`grades`].
pub async fn grades_of(
    ctx: &mut Context<'_>,
    semester: Option<&str>,
) -> Result<Fetched<GradeReport>, Problem> {
    let services = ctx.services();
    match &services.grades {
        Some(GradesService::Demo) => Ok(fetched(only_semester(demo::grades(), semester)?, "demo")),
        Some(GradesService::EtudiantsApi { base_url }) => {
            let report = etudiants_api::fetch_grades(ctx, base_url).await?;
            Ok(fetched(only_semester(report, semester)?, "etudiants-api"))
        }
        Some(GradesService::ScodocGateway(config)) => scodoc::fetch_grades(ctx, config, semester)
            .await
            .map(|data| fetched(data, "scodoc-gateway")),
        Some(GradesService::Link(_) | GradesService::None) | None => {
            Err(absent(&services, ServiceKind::Grades))
        }
    }
}

/// Ce que seule la passerelle ScoDoc sait faire autour d'un relevé.
fn scodoc_only(services: &Services, what: &str) -> Problem {
    match &services.grades {
        Some(GradesService::Demo | GradesService::EtudiantsApi { .. }) => {
            Problem::new(ErrorCode::NotImplemented).detail(format!(
                "{what} : pas disponible avec ce fournisseur de notes"
            ))
        }
        _ => absent(services, ServiceKind::Grades),
    }
}

/// Message d'un département (`SemesterGrades::department`) à afficher avec ses relevés.
pub async fn grade_notice(
    ctx: &mut Context<'_>,
    department: &str,
) -> Result<Fetched<GradeNotice>, Problem> {
    let services = ctx.services();
    match &services.grades {
        Some(GradesService::ScodocGateway(config)) => scodoc::fetch_notice(ctx, config, department)
            .await
            .map(|data| fetched(data, "scodoc-gateway")),
        _ => Err(scodoc_only(&services, "message du département")),
    }
}

/// Répartition des notes de la promotion à une évaluation du relevé d'un semestre.
pub async fn grade_distribution(
    ctx: &mut Context<'_>,
    semester: &str,
    evaluation: &str,
) -> Result<Fetched<GradeDistribution>, Problem> {
    let services = ctx.services();
    match &services.grades {
        Some(GradesService::ScodocGateway(config)) => {
            scodoc::fetch_distribution(ctx, config, semester, evaluation)
                .await
                .map(|data| fetched(data, "scodoc-gateway"))
        }
        _ => Err(scodoc_only(&services, "répartition des notes")),
    }
}

/// Relevé PDF d'un semestre.
pub async fn grade_transcript(
    ctx: &mut Context<'_>,
    semester: &str,
) -> Result<Fetched<Download>, Problem> {
    let services = ctx.services();
    match &services.grades {
        Some(GradesService::ScodocGateway(config)) => {
            scodoc::fetch_transcript(ctx, config, semester)
                .await
                .map(|data| fetched(data, "scodoc-gateway"))
        }
        _ => Err(scodoc_only(&services, "relevé PDF")),
    }
}

/// Notes puis absences ; une seule lecture quand les deux viennent de la même passerelle ScoDoc.
pub async fn grades_and_absences(
    ctx: &mut Context<'_>,
) -> (
    Result<Fetched<GradeReport>, Problem>,
    Result<Fetched<AbsenceReport>, Problem>,
) {
    grades_and_absences_of(ctx, None).await
}

/// Notes et absences du semestre demandé (comme [`grades_of`]) ; sans semestre, du semestre
/// courant. Les absences d'un semestre choisi ne se lisent qu'avec ses notes, dans la même
/// réponse de la passerelle ScoDoc : un autre fournisseur répond `not_implemented` pour elles.
pub async fn grades_and_absences_of(
    ctx: &mut Context<'_>,
    semester: Option<&str>,
) -> (
    Result<Fetched<GradeReport>, Problem>,
    Result<Fetched<AbsenceReport>, Problem>,
) {
    let services = ctx.services();
    if let (
        Some(GradesService::ScodocGateway(grades_config)),
        Some(AbsencesService::ScodocGateway(absences_config)),
    ) = (&services.grades, &services.absences)
        && grades_config == absences_config
    {
        let (g, a) = scodoc::fetch_both(ctx, grades_config, semester).await;
        return (
            g.map(|data| fetched(data, "scodoc-gateway")),
            a.map(|data| fetched(data, "scodoc-gateway")),
        );
    }
    let g = grades_of(ctx, semester).await;
    let a = if semester.is_some() {
        Err(Problem::new(ErrorCode::NotImplemented)
            .detail("absences d'un semestre choisi : pas disponibles avec ce fournisseur"))
    } else {
        absences(ctx).await
    };
    (g, a)
}

pub async fn absences(ctx: &mut Context<'_>) -> Result<Fetched<AbsenceReport>, Problem> {
    let services = ctx.services();
    match &services.absences {
        Some(AbsencesService::Demo) => Ok(fetched(demo::absences(ctx.now), "demo")),
        Some(AbsencesService::ScodocGateway(config)) => scodoc::fetch_absences(ctx, config)
            .await
            .map(|data| fetched(data, "scodoc-gateway")),
        Some(AbsencesService::Link(_) | AbsencesService::None) | None => {
            Err(absent(&services, ServiceKind::Absences))
        }
    }
}

pub async fn exams(ctx: &mut Context<'_>) -> Result<Fetched<Vec<Exam>>, Problem> {
    let services = ctx.services();
    match &services.exams {
        Some(ExamsService::Demo) => Ok(fetched(demo::exams(ctx.now), "demo")),
        Some(ExamsService::EtudiantsApi { base_url }) => Ok(fetched(
            etudiants_api::fetch_exams(ctx, base_url).await?,
            "etudiants-api",
        )),
        Some(ExamsService::Link(_) | ExamsService::None) | None => {
            Err(absent(&services, ServiceKind::Exams))
        }
    }
}

pub async fn documents(ctx: &mut Context<'_>) -> Result<Fetched<Vec<Document>>, Problem> {
    let services = ctx.services();
    match &services.documents {
        Some(DocumentsService::Demo) => Ok(fetched(demo::documents(), "demo")),
        Some(DocumentsService::EtudiantsApi {
            base_url,
            certificate_url,
        }) => Ok(fetched(
            etudiants_api::fetch_documents(ctx, base_url, certificate_url.as_ref()).await?,
            "etudiants-api",
        )),
        Some(DocumentsService::Ardemat {
            base_url,
            service_url,
            ..
        }) => Ok(fetched(
            ardemat::fetch_documents(ctx, base_url, service_url).await?,
            "ardemat",
        )),
        Some(DocumentsService::MonDossierWeb { .. }) => Err(mon_dossier_web_pending()),
        Some(DocumentsService::Link(_) | DocumentsService::None) | None => {
            Err(absent(&services, ServiceKind::Documents))
        }
    }
}

/// Fichier d'un document de [`documents`] marqué `downloadable`.
pub async fn document(ctx: &mut Context<'_>, id: &str) -> Result<Fetched<Download>, Problem> {
    let services = ctx.services();
    match &services.documents {
        Some(DocumentsService::Demo) => Ok(fetched(demo::document(id)?, "demo")),
        Some(DocumentsService::EtudiantsApi { base_url, .. }) => Ok(fetched(
            etudiants_api::download(ctx, base_url, id).await?,
            "etudiants-api",
        )),
        Some(DocumentsService::Ardemat {
            base_url,
            service_url,
            ..
        }) => Ok(fetched(
            ardemat::download(ctx, base_url, service_url, id).await?,
            "ardemat",
        )),
        Some(DocumentsService::MonDossierWeb { .. }) => Err(mon_dossier_web_pending()),
        Some(DocumentsService::Link(_) | DocumentsService::None) | None => {
            Err(absent(&services, ServiceKind::Documents))
        }
    }
}

pub async fn homework(ctx: &mut Context<'_>) -> Result<Fetched<Vec<Homework>>, Problem> {
    let services = ctx.services();
    match &services.homework {
        Some(HomeworkService::Demo) => Ok(fetched(demo::homework(ctx.now), "demo")),
        Some(HomeworkService::MoodleWebservice {
            base_url, sign_in, ..
        }) => moodle::fetch_webservice(ctx, base_url, sign_in)
            .await
            .map(|data| fetched(data, "moodle-webservice")),
        Some(HomeworkService::MoodleAjax {
            base_url, entry, ..
        }) => moodle::fetch_ajax(ctx, base_url, *entry)
            .await
            .map(|data| fetched(data, "moodle-ajax")),
        Some(HomeworkService::Link(_) | HomeworkService::None) | None => {
            Err(absent(&services, ServiceKind::Homework))
        }
    }
}

pub async fn mail_inbox(ctx: &mut Context<'_>) -> Result<Fetched<MailInbox>, Problem> {
    let services = ctx.services();
    match &services.mail {
        Some(MailService::Demo) => Ok(fetched(demo::inbox(ctx.now), "demo")),
        Some(MailService::ZimbraRest {
            base_url,
            entry_url,
            ..
        }) => {
            let inbox = zimbra::fetch_inbox(ctx, base_url, entry_url.as_ref()).await?;
            Ok(fetched(inbox, "zimbra-rest"))
        }
        Some(MailService::Bluemind { base_url, .. }) => Ok(fetched(
            bluemind::fetch_inbox(ctx, base_url).await?,
            "bluemind",
        )),
        Some(MailService::Link(_) | MailService::None) | None => {
            Err(absent(&services, ServiceKind::Mail))
        }
    }
}

pub async fn apps(ctx: &mut Context<'_>) -> Result<Fetched<Vec<App>>, Problem> {
    let services = ctx.services();
    match &services.apps {
        Some(AppsService::Demo) => Ok(fetched(demo::apps(), "demo")),
        Some(AppsService::Portal) if tiles(ctx) => Ok(fetched(
            tiles_portal::fetch_apps(ctx).await?,
            "tiles-portal",
        )),
        Some(AppsService::Portal) => Ok(fetched(uportal::fetch_apps(ctx).await?, "portal")),
        Some(AppsService::None) | None => Err(absent(&services, ServiceKind::Apps)),
    }
}

/// Connexion à un service qui a son propre compte (CAS de composante, Moodle en connexion
/// locale). Le mot de passe sert une fois ; seuls les cookies ou le jeton du service restent dans
/// la session.
pub async fn service_login(
    ctx: &mut Context<'_>,
    kind: ServiceKind,
    username: &str,
    password: &str,
) -> Result<(), Problem> {
    let services = ctx.services();
    match (
        kind,
        &services.grades,
        &services.absences,
        &services.homework,
    ) {
        (ServiceKind::Grades, Some(GradesService::ScodocGateway(config)), _, _)
        | (ServiceKind::Absences, _, Some(AbsencesService::ScodocGateway(config)), _) => {
            return scodoc::sign_in(ctx, config, username, password).await;
        }
        (ServiceKind::Homework, _, _, Some(config @ HomeworkService::MoodleWebservice { .. })) => {
            return moodle::sign_in(ctx, config, username, password).await;
        }
        _ => {}
    }
    let provider = services
        .availability()
        .into_iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, a)| a);
    match provider {
        Some(ServiceAvailability::Api { provider }) => Err(Problem::new(ErrorCode::InvalidRequest)
            .detail(format!(
                "le fournisseur `{provider}` n'a pas de connexion séparée"
            ))),
        _ => Err(absent(&services, kind)),
    }
}
