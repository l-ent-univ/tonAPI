//! Fournisseurs de données. Chaque fonction choisit le fournisseur configuré pour le service de
//! l'étudiant (université + composante) et convertit sa réponse vers les modèles de `tonapi-core`.
//!
//! Un service absent donne `service_absent` (avec le lien quand il y en a un) ; un fournisseur déclaré
//! mais pas encore écrit donne `not_implemented`.

// Les fournisseurs réseau sont asynchrones ; `demo` ne l'est pas mais partage la même signature.
#![allow(clippy::unused_async)]

mod ade_ical;
mod ade_rest;
mod demo;
#[allow(dead_code)] // utilisé par les fournisseurs réseau au fur et à mesure de leur ajout
mod net;
mod sso;

use chrono::{DateTime, NaiveDate, Utc};
use tonapi_config::{
    AbsencesService, AppsService, GradesService, HomeworkService, MailService, ProfileService,
    ResolvedUniversity, ServiceAvailability, ServiceKind, Services, TimetableService,
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

fn not_implemented(provider: &str) -> Problem {
    Problem::new(ErrorCode::NotImplemented).detail(format!(
        "le fournisseur `{provider}` n'est pas encore pris en charge"
    ))
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
        Some(ProfileService::Portal) => Err(not_implemented("portal")),
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
        Some(TimetableService::Link(_) | TimetableService::None) | None => {
            Err(absent(&services, ServiceKind::Timetable))
        }
    }
}

pub async fn grades(ctx: &mut Context<'_>) -> Result<Fetched<GradeReport>, Problem> {
    let services = ctx.services();
    match &services.grades {
        Some(GradesService::Demo) => Ok(fetched(demo::grades(), "demo")),
        Some(GradesService::ScodocGateway(_)) => Err(not_implemented("scodoc-gateway")),
        Some(GradesService::Link(_) | GradesService::None) | None => {
            Err(absent(&services, ServiceKind::Grades))
        }
    }
}

pub async fn absences(ctx: &mut Context<'_>) -> Result<Fetched<AbsenceReport>, Problem> {
    let services = ctx.services();
    match &services.absences {
        Some(AbsencesService::Demo) => Ok(fetched(demo::absences(ctx.now), "demo")),
        Some(AbsencesService::ScodocGateway(_)) => Err(not_implemented("scodoc-gateway")),
        Some(AbsencesService::Link(_) | AbsencesService::None) | None => {
            Err(absent(&services, ServiceKind::Absences))
        }
    }
}

pub async fn homework(ctx: &mut Context<'_>) -> Result<Fetched<Vec<Homework>>, Problem> {
    let services = ctx.services();
    match &services.homework {
        Some(HomeworkService::Demo) => Ok(fetched(demo::homework(ctx.now), "demo")),
        Some(HomeworkService::MoodleWebservice { .. }) => Err(not_implemented("moodle-webservice")),
        Some(HomeworkService::MoodleAjax { .. }) => Err(not_implemented("moodle-ajax")),
        Some(HomeworkService::Link(_) | HomeworkService::None) | None => {
            Err(absent(&services, ServiceKind::Homework))
        }
    }
}

pub async fn mail_inbox(ctx: &mut Context<'_>) -> Result<Fetched<MailInbox>, Problem> {
    let services = ctx.services();
    match &services.mail {
        Some(MailService::Demo) => Ok(fetched(demo::inbox(ctx.now), "demo")),
        Some(MailService::ZimbraRest { .. }) => Err(not_implemented("zimbra-rest")),
        Some(MailService::Link(_) | MailService::None) | None => {
            Err(absent(&services, ServiceKind::Mail))
        }
    }
}

pub async fn apps(ctx: &mut Context<'_>) -> Result<Fetched<Vec<App>>, Problem> {
    let services = ctx.services();
    match &services.apps {
        Some(AppsService::Demo) => Ok(fetched(demo::apps(), "demo")),
        Some(AppsService::Portal) => Err(not_implemented("portal")),
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
    let _ = (username, password);
    let services = ctx.services();
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
