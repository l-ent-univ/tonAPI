//! Fournisseur `tempo` : TEMPO, l'emploi du temps personnel des étudiants de Rennes 2, derrière
//! le CAS de l'université.
//!
//! - `GET api/events?start=AAAA-MM-JJ&end=AAAA-MM-JJ` (les deux jours compris) répond un tableau
//!   JSON des séances de l'étudiant connecté ; il n'y a pas de groupe à choisir.
//! - `GET rooms` est la page « Disponibilité des salles » : l'arbre de toutes les salles, écrit
//!   dans la page. C'est le seul arbre de ressources que TEMPO sache interroger.
//! - `GET api/room/<salle>?start=…&end=…&c=` répond les séances d'une salle, au même format.
//!
//! Sans session, l'API répond 401 en JSON et une page renvoie au CAS : la session s'ouvre par le
//! CAS avec le service `<tempo>/auth`, qui reçoit le ticket, puis la lecture est rejouée **une
//! seule fois**.
//!
//! Ouverture de session : CAS → `auth?ticket=…` → `auth` → accueil. Elle est acquise dès que la
//! chaîne revient sur TEMPO ailleurs que sur `auth` ; la page d'accueil n'est pas chargée. Une
//! session qui a déjà vu `auth` y tourne jusqu'à un 403, ouverte quand même.
//!
//! Le lien joint à une séance (`url`, `url_label`) est rendu dans `Course::link`. Ce que tonAPI
//! ne rend pas : les emplois du temps que l'étudiant a réunis dans le sien (page
//! `planning/edit`). TEMPO montre aussi l'arbre des groupes d'ADE, pour en ajouter un au sien,
//! mais ne donne l'emploi du temps d'aucun groupe à la demande. Une journée entière va de minuit
//! à minuit.
//!
//! [vu] 27/11/2026, compte étudiant de Rennes 2 : `docs/recherche/rennes-2.md`.

use chrono::NaiveDate;
use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use tonapi_core::{Course, ErrorCode, Problem, ResourceKind, ResourceNode, TimetableResource};
use url::Url;

use crate::common::{root, university_tz};
use crate::net::{Net, upstream};
use crate::{Context, ResourceQuery, resource_tree};

mod parse;

use parse::{is_rooms_page, merge, parse_events, parse_rooms};

/// Nom du logiciel dans les messages d'erreur.
const PROVIDER: &str = "tempo";
/// Sauts suivis au plus pendant l'ouverture de session.
const SIGN_IN_MAX_HOPS: usize = 6;
/// Salles interrogées au plus pour une même plage (une requête chacune).
const MAX_ROOMS: usize = 30;
/// Un numéro de salle est celui de sa ressource dans ADE : quelques chiffres.
const MAX_ROOM_ID_LENGTH: usize = 12;

/// Ce qu'on demande à TEMPO.
#[derive(Clone, Copy)]
enum Ask<'a> {
    /// Les séances de l'étudiant.
    Events(NaiveDate, NaiveDate),
    /// Les séances d'une salle.
    Room(&'a str, NaiveDate, NaiveDate),
    /// La page des salles.
    RoomsPage,
}

/// Ce que TEMPO a répondu.
enum Reply {
    Json(Value),
    Page(String),
    /// 401 ou redirection : pas de session TEMPO pour ce navigateur.
    Refused,
}

impl Ask<'_> {
    fn url(self, base: &Url) -> Result<Url, Problem> {
        let illegible = |_| upstream(PROVIDER, "adresse de TEMPO illisible");
        let day = |day: NaiveDate| day.format("%Y-%m-%d").to_string();
        match self {
            Ask::Events(from, to) => {
                let mut url = base.join("api/events").map_err(illegible)?;
                url.query_pairs_mut()
                    .append_pair("start", &day(from))
                    .append_pair("end", &day(to));
                Ok(url)
            }
            Ask::Room(room, from, to) => {
                let mut url = base.join(&format!("api/room/{room}")).map_err(illegible)?;
                // `c` est le code du campus que la page des salles porte dans un champ caché,
                // vide pour le compte essayé. Sans ce paramètre, TEMPO répond 404.
                url.query_pairs_mut()
                    .append_pair("start", &day(from))
                    .append_pair("end", &day(to))
                    .append_pair("c", "");
                Ok(url)
            }
            Ask::RoomsPage => base.join("rooms").map_err(illegible),
        }
    }

    /// La réponse est-elle de la nature attendue ?
    fn answered_by(self, reply: &Reply) -> bool {
        match (self, reply) {
            (Ask::RoomsPage, Reply::Page(page)) => is_rooms_page(page),
            (Ask::Events(..) | Ask::Room(..), Reply::Json(_)) => true,
            _ => false,
        }
    }
}

/// TEMPO n'a pas de groupes à choisir : l'emploi du temps est celui du compte.
pub(crate) fn groups_unsupported() -> Problem {
    Problem::new(ErrorCode::NotImplemented)
        .detail("tempo : l'emploi du temps est celui de l'étudiant, sans groupe à choisir")
}

/// Séances qui chevauchent `from..=to` (jours à l'heure de l'université) : celles de l'étudiant,
/// ou celles des salles demandées (`resources`, des identifiants de [`fetch_tree`]).
pub(crate) async fn fetch_timetable(
    ctx: &mut Context<'_>,
    base_url: &Url,
    from: NaiveDate,
    to: NaiveDate,
    resources: &[String],
) -> Result<Vec<Course>, Problem> {
    let tz = university_tz(ctx)?;
    // Refusées avant toute requête : une demande mal formée ne coûte rien à TEMPO.
    let rooms = requested_rooms(resources)?;
    if from > to {
        return Ok(Vec::new());
    }
    let base = root(base_url);
    let mut net = Net::open(ctx, PROVIDER)?;
    let result = async {
        if rooms.is_empty() {
            let events = json(read(&mut net, &base, Ask::Events(from, to)).await?)?;
            return parse_events(&events, tz, from, to);
        }
        let mut found = Vec::new();
        for room in &rooms {
            let events = json(read(&mut net, &base, Ask::Room(room, from, to)).await?)?;
            found.extend(parse_events(&events, tz, from, to)?);
        }
        // Une séance qui occupe deux des salles demandées n'est rendue qu'une fois.
        Ok(merge(found))
    }
    .await;
    net.save(ctx);
    result
}

/// Salles demandées, sans doublon. Une salle se demande par le numéro de sa ressource : toute
/// autre valeur est refusée, sans être recopiée dans l'erreur.
fn requested_rooms(resources: &[String]) -> Result<Vec<&str>, Problem> {
    let mut rooms: Vec<&str> = Vec::with_capacity(resources.len());
    for resource in resources {
        let room = resource.trim();
        let numeric = !room.is_empty()
            && room.len() <= MAX_ROOM_ID_LENGTH
            && room.bytes().all(|b| b.is_ascii_digit());
        if !numeric {
            return Err(Problem::new(ErrorCode::InvalidRequest)
                .detail("tempo : seule une salle se demande, par son identifiant numérique"));
        }
        if !rooms.contains(&room) {
            rooms.push(room);
        }
    }
    if rooms.len() > MAX_ROOMS {
        return Err(Problem::new(ErrorCode::InvalidRequest)
            .detail(format!("plus de {MAX_ROOMS} salles demandées : en retirer")));
    }
    Ok(rooms)
}

/// Arbre des salles, tel que la page « Disponibilité des salles » le montre.
async fn rooms(ctx: &mut Context<'_>, base_url: &Url) -> Result<Vec<ResourceNode>, Problem> {
    let base = root(base_url);
    let mut net = Net::open(ctx, PROVIDER)?;
    let result = read(&mut net, &base, Ask::RoomsPage).await;
    net.save(ctx);
    match result? {
        Reply::Page(page) => parse_rooms(&page),
        _ => Err(upstream(PROVIDER, "page des salles illisible")),
    }
}

/// TEMPO n'a d'arbre que pour les salles : inutile de le lire pour une autre nature.
fn wants_rooms(kinds: &[ResourceKind]) -> bool {
    kinds.is_empty() || kinds.contains(&ResourceKind::Room)
}

/// Arbre des ressources : les salles, rangées par dossiers. Vide si une autre nature est
/// demandée.
pub(crate) async fn fetch_tree(
    ctx: &mut Context<'_>,
    base_url: &Url,
    kinds: &[ResourceKind],
) -> Result<Vec<ResourceNode>, Problem> {
    if !wants_rooms(kinds) {
        return Ok(Vec::new());
    }
    rooms(ctx, base_url).await
}

/// Salles qui répondent à la recherche, chacune avec son chemin.
pub(crate) async fn fetch_resources(
    ctx: &mut Context<'_>,
    base_url: &Url,
    query: &ResourceQuery,
) -> Result<Vec<TimetableResource>, Problem> {
    if !wants_rooms(&query.kinds) {
        return Ok(Vec::new());
    }
    Ok(resource_tree::resources(
        &rooms(ctx, base_url).await?,
        query,
    ))
}

fn json(reply: Reply) -> Result<Value, Problem> {
    match reply {
        Reply::Json(value) => Ok(value),
        _ => Err(upstream(PROVIDER, "réponse non JSON")),
    }
}

/// La réponse attendue, après une ouverture de session si TEMPO en demande une.
async fn read(net: &mut Net, base: &Url, ask: Ask<'_>) -> Result<Reply, Problem> {
    let first = get(net, base, ask).await?;
    if ask.answered_by(&first) {
        return Ok(first);
    }
    sign_in(net, base).await?;
    let second = get(net, base, ask).await?;
    if ask.answered_by(&second) {
        return Ok(second);
    }
    match second {
        Reply::Refused => Err(Problem::new(ErrorCode::SessionExpired)
            .detail("tempo : pas de session après le passage par le CAS")),
        // Autre chose que ce qu'on attend même après le CAS (maintenance…) : ni une session
        // perdue, ni une période sans cours.
        _ => Err(upstream(
            PROVIDER,
            "réponse inattendue après l'ouverture de session",
        )),
    }
}

async fn get(net: &mut Net, base: &Url, ask: Ask<'_>) -> Result<Reply, Problem> {
    let url = ask.url(base)?;
    let mut headers = HeaderMap::new();
    if matches!(ask, Ask::RoomsPage) {
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("text/html,application/xhtml+xml,*/*"),
        );
    } else {
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(
            HeaderName::from_static("x-requested-with"),
            HeaderValue::from_static("XMLHttpRequest"),
        );
        if let Ok(referer) = HeaderValue::from_str(base.as_str()) {
            headers.insert(header::REFERER, referer);
        }
    }
    let (status, _, body) = net.request(Method::GET, url, headers, None).await?;
    if status == reqwest::StatusCode::UNAUTHORIZED || status.is_redirection() {
        return Ok(Reply::Refused);
    }
    if status.is_server_error() {
        return Err(Problem::new(ErrorCode::ServiceUnavailable)
            .detail(format!("{PROVIDER} : réponse HTTP {}", status.as_u16())));
    }
    if !status.is_success() {
        return Err(upstream(
            PROVIDER,
            format!("réponse HTTP {} inattendue", status.as_u16()),
        ));
    }
    Ok(match serde_json::from_str(&body) {
        Ok(value) => Reply::Json(value),
        Err(_) => Reply::Page(body),
    })
}

/// Ouvre la session TEMPO avec le TGC de la session, sans mot de passe. Le CAS qui répond par une
/// page au lieu de rediriger redemande la connexion : `session_expired`.
async fn sign_in(net: &mut Net, base: &Url) -> Result<(), Problem> {
    let Some(cas) = net.cas.clone() else {
        return Err(Problem::new(ErrorCode::Internal)
            .detail("ce service demande une université derrière un CAS"));
    };
    let service = base
        .join("auth")
        .map_err(|_| upstream(PROVIDER, "adresse de TEMPO illisible"))?;
    let on_tempo = |url: &Url| {
        url.host_str() == base.host_str()
            && url.port_or_known_default() == base.port_or_known_default()
    };
    let mut url = cas.login_url(Some(&service));
    for _ in 0..SIGN_IN_MAX_HOPS {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("text/html,application/xhtml+xml,*/*"),
        );
        let (status, response, _) = net.request(Method::GET, url.clone(), headers, None).await?;
        let following = response
            .get(header::LOCATION)
            .and_then(|location| location.to_str().ok())
            .filter(|_| status.is_redirection())
            .and_then(|location| url.join(location).ok());
        let Some(following) = following else {
            // Fin de la chaîne : sur TEMPO, la session est ouverte (même sur le 403 de `auth`
            // rejoué) ; sur le CAS, c'est sa page de connexion.
            return if on_tempo(&url) {
                Ok(())
            } else if status.is_server_error() {
                Err(Problem::new(ErrorCode::ServiceUnavailable).detail(format!(
                    "{PROVIDER} : le CAS répond HTTP {}",
                    status.as_u16()
                )))
            } else {
                Err(Problem::new(ErrorCode::SessionExpired)
                    .detail("l'université redemande la connexion : se reconnecter"))
            };
        };
        if on_tempo(&following) {
            if following.path() != service.path() {
                return Ok(());
            }
        } else if !cas.owns(&following) {
            return Err(upstream(
                PROVIDER,
                "l'ouverture de session mène hors de TEMPO et du CAS",
            ));
        }
        url = following;
    }
    if on_tempo(&url) {
        Ok(())
    } else {
        Err(upstream(
            PROVIDER,
            "trop de redirections pendant l'ouverture de session",
        ))
    }
}
