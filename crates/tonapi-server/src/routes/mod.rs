//! Routes de l'API.

mod data;
mod me;
mod overview;
mod session;
mod universities;

use tonapi_config::{ResolvedUniversity, ServiceAvailability, Services};
use tonapi_core::{Availability, EstablishmentChoice, ServiceStatus};
use tonapi_session::EstablishmentPick;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(universities::health))
        .routes(routes!(universities::ready))
        .routes(routes!(universities::list))
        .routes(routes!(universities::detail))
        .routes(routes!(session::login))
        .routes(routes!(session::mfa))
        .routes(routes!(session::service_login))
        .routes(routes!(session::refresh))
        .routes(routes!(session::logout))
        .routes(routes!(me::me))
        .routes(routes!(me::set_establishment))
        .routes(routes!(data::timetable))
        .routes(routes!(data::timetable_groups))
        .routes(routes!(data::timetable_tree))
        .routes(routes!(data::timetable_resources))
        .routes(routes!(data::grades))
        .routes(routes!(data::grade_notice))
        .routes(routes!(data::grade_distribution))
        .routes(routes!(data::grade_transcript))
        .routes(routes!(data::absences))
        .routes(routes!(data::exams))
        .routes(routes!(data::homework))
        .routes(routes!(data::mail_inbox))
        .routes(routes!(data::apps))
        .routes(routes!(data::documents))
        .routes(routes!(data::document))
        .routes(routes!(overview::overview))
}

/// Disponibilité de chaque service, au format de l'API.
fn service_statuses(services: &Services) -> Vec<ServiceStatus> {
    services
        .availability()
        .into_iter()
        .map(|(kind, availability)| {
            let service = kind.key().to_owned();
            match availability {
                ServiceAvailability::Api { provider } => ServiceStatus {
                    service,
                    availability: Availability::Api,
                    provider: Some(provider),
                    link: None,
                },
                ServiceAvailability::Link { url } => ServiceStatus {
                    service,
                    availability: Availability::Link,
                    provider: None,
                    link: Some(url.to_string()),
                },
                ServiceAvailability::Absent => ServiceStatus {
                    service,
                    availability: Availability::Absent,
                    provider: None,
                    link: None,
                },
            }
        })
        .collect()
}

/// Composante retenue, avec son nom.
fn establishment_choice(
    university: &ResolvedUniversity,
    pick: Option<&EstablishmentPick>,
) -> Option<EstablishmentChoice> {
    let pick = pick?;
    let establishment = university.establishments.get(&pick.id)?;
    Some(EstablishmentChoice {
        id: pick.id.clone(),
        name: establishment.name.clone(),
        source: pick.source,
        confidence: pick.confidence,
        signals: Vec::new(),
    })
}
