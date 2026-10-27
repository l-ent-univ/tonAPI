//! Fournisseurs de données. Chaque fonction choisit le fournisseur configuré pour le service de
//! l'étudiant (université + composante) et convertit sa réponse vers les modèles de `tonapi-core`.
//!
//! Un service absent donne `service_absent` (avec le lien quand il y en a un) ; une fonction qu'un
//! fournisseur ne peut pas assurer (groupes d'un lien iCal, par exemple) donne `not_implemented`.

// Les fournisseurs réseau sont asynchrones ; `demo` ne l'est pas mais partage la même signature.
#![allow(clippy::unused_async)]

mod ade_ical;
mod ade_myplanning;
mod ade_rest;
mod bluemind;
mod demo;
mod etudiants_api;
mod moodle;
#[allow(dead_code)] // utilisé par les fournisseurs réseau au fur et à mesure de leur ajout
mod net;
mod scodoc;
mod sso;
mod tiles_portal;
mod uportal;
mod zimbra;

use chrono::{DateTime, NaiveDate, Utc};
use tonapi_config::{
    AbsencesService, AppsService, GradesService, HomeworkService, MailService, PortalConfig,
    ProfileService, ResolvedUniversity, ServiceAvailability, ServiceKind, Services, Signal,
    StudentRecord, TimetableService,
};
use tonapi_core::{
    AbsenceReport, App, Course, ErrorCode, EstablishmentSource, GradeReport, Homework, MailInbox,
    Problem, Profile, TimetableGroup,
};
use tonapi_session::{EstablishmentPick, SessionData};

pub use net::NetSettings;
pub use sso::{LoginInput, LoginOutcome, MfaAction, MfaPrompt, complete_mfa, login, refresh};
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

fn tiles(ctx: &Context<'_>) -> bool {
    matches!(
        ctx.university.university.portal,
        PortalConfig::TilesPortal { .. }
    )
}

/// Signaux de rattachement que le portail fournit (aucun pour les autres portails).
async fn signals(ctx: &mut Context<'_>) -> Result<Vec<Signal>, Problem> {
    if tiles(ctx) {
        tiles_portal::fetch_signals(ctx).await
    } else {
        Ok(Vec::new())
    }
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
pub(crate) async fn with_detected_establishment(
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
    if let Ok(signals) = signals(&mut ctx).await
        && let Some(pick) = detect_establishment(university, &signals)
    {
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

pub async fn timetable(
    ctx: &mut Context<'_>,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Fetched<Vec<Course>>, Problem> {
    let services = ctx.services();
    match &services.timetable {
        Some(TimetableService::Demo) => Ok(fetched(demo::timetable(from, to), "demo")),
        Some(TimetableService::AdeIcalLink {
            link_page,
            feed_host,
            ..
        }) => {
            let courses = ade_ical::fetch_timetable(ctx, link_page, feed_host, from, to).await?;
            Ok(fetched(courses, "ade-ical-link"))
        }
        Some(TimetableService::AdePortalRest { base_url, .. }) => {
            let courses = ade_rest::fetch_timetable(ctx, base_url, from, to).await?;
            Ok(fetched(courses, "ade-portal-rest"))
        }
        Some(TimetableService::AdeMyplanning {
            base_url,
            project_id,
            ..
        }) => {
            let courses =
                ade_myplanning::fetch_timetable(ctx, base_url, *project_id, from, to).await?;
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
        Some(TimetableService::AdeIcalLink { .. }) => Err(ade_ical::groups_unsupported()),
        Some(TimetableService::AdePortalRest { base_url, .. }) => {
            let groups = ade_rest::fetch_groups(ctx, base_url).await?;
            Ok(fetched(groups, "ade-portal-rest"))
        }
        Some(TimetableService::AdeMyplanning { .. }) => {
            Err(Problem::new(ErrorCode::NotImplemented)
                .detail("le fournisseur `ade-myplanning` ne liste pas encore les groupes"))
        }
        Some(TimetableService::Link(_) | TimetableService::None) | None => {
            Err(absent(&services, ServiceKind::Timetable))
        }
    }
}

pub async fn grades(ctx: &mut Context<'_>) -> Result<Fetched<GradeReport>, Problem> {
    let services = ctx.services();
    match &services.grades {
        Some(GradesService::Demo) => Ok(fetched(demo::grades(), "demo")),
        Some(GradesService::ScodocGateway(config)) => scodoc::fetch_grades(ctx, config)
            .await
            .map(|data| fetched(data, "scodoc-gateway")),
        Some(GradesService::Link(_) | GradesService::None) | None => {
            Err(absent(&services, ServiceKind::Grades))
        }
    }
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
