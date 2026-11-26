//! Fournisseur `tempo` : TEMPO, l'emploi du temps personnel des étudiants de Rennes 2, derrière
//! le CAS de l'université.
//!
//! `GET api/events?start=AAAA-MM-JJ&end=AAAA-MM-JJ` (les deux jours compris) répond un tableau
//! JSON des séances de l'étudiant connecté ; il n'y a ni groupe ni ressource à choisir. Sans
//! session, l'API répond 401 en JSON (ou redirige, ou sert une page) : la session s'ouvre par le
//! CAS avec le service `<tempo>/auth`, qui reçoit le ticket, puis la lecture est rejouée **une
//! seule fois**.
//!
//! Ouverture de session : CAS → `auth?ticket=…` → `auth` → accueil. Elle est acquise dès que la
//! chaîne revient sur TEMPO ailleurs que sur `auth` ; la page d'accueil n'est pas chargée. Une
//! session qui a déjà vu `auth` y tourne jusqu'à un 403, ouverte quand même.
//!
//! Ce que tonAPI ne rend pas : le lien d'une séance (`url`, `url_label`), qui n'a pas de champ
//! dans `Course`. Une journée entière va de minuit à minuit.
//!
//! Source : le code de Tafac (`tempoApi.js`), `docs/recherche/rennes-2.md`. Jamais essayé avec
//! un compte réel par tonAPI.

use chrono::NaiveDate;
use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use tonapi_core::{Course, ErrorCode, Problem};
use url::Url;

use crate::Context;
use crate::common::{root, university_tz};
use crate::net::{Net, upstream};

mod parse;

use parse::parse_events;

/// Nom du logiciel dans les messages d'erreur.
const PROVIDER: &str = "tempo";
/// Sauts suivis au plus pendant l'ouverture de session.
const SIGN_IN_MAX_HOPS: usize = 6;

/// Ce que l'API a répondu à une lecture.
enum Reply {
    Events(Value),
    /// 401 ou redirection : pas de session TEMPO pour ce navigateur.
    Refused,
    /// Une page à la place du JSON : celle d'une connexion à refaire, ou autre chose.
    Page,
}

/// TEMPO n'a pas de groupes : l'emploi du temps est celui du compte.
pub(crate) fn groups_unsupported() -> Problem {
    Problem::new(ErrorCode::NotImplemented)
        .detail("tempo : l'emploi du temps est celui de l'étudiant, sans groupe à choisir")
}

/// Séances de l'étudiant qui chevauchent `from..=to` (jours à l'heure de l'université).
pub(crate) async fn fetch_timetable(
    ctx: &mut Context<'_>,
    base_url: &Url,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Course>, Problem> {
    let tz = university_tz(ctx)?;
    if from > to {
        return Ok(Vec::new());
    }
    let base = root(base_url);
    let mut net = Net::open(ctx, PROVIDER)?;
    let result = read(&mut net, &base, from, to).await;
    net.save(ctx);
    parse_events(&result?, tz, from, to)
}

/// La liste des événements, après une ouverture de session si l'API en demande une.
async fn read(net: &mut Net, base: &Url, from: NaiveDate, to: NaiveDate) -> Result<Value, Problem> {
    if let Reply::Events(events) = events(net, base, from, to).await? {
        return Ok(events);
    }
    sign_in(net, base).await?;
    match events(net, base, from, to).await? {
        Reply::Events(events) => Ok(events),
        Reply::Refused => Err(Problem::new(ErrorCode::SessionExpired)
            .detail("tempo : pas de session après le passage par le CAS")),
        // Une page même après le CAS (maintenance…) : ni une session perdue, ni une période
        // sans cours.
        Reply::Page => Err(upstream(
            PROVIDER,
            "réponse non JSON après l'ouverture de session",
        )),
    }
}

async fn events(
    net: &mut Net,
    base: &Url,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Reply, Problem> {
    let mut url = base
        .join("api/events")
        .map_err(|_| upstream(PROVIDER, "adresse de TEMPO illisible"))?;
    url.query_pairs_mut()
        .append_pair("start", &from.format("%Y-%m-%d").to_string())
        .append_pair("end", &to.format("%Y-%m-%d").to_string());
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(
        HeaderName::from_static("x-requested-with"),
        HeaderValue::from_static("XMLHttpRequest"),
    );
    if let Ok(referer) = HeaderValue::from_str(base.as_str()) {
        headers.insert(header::REFERER, referer);
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
    Ok(serde_json::from_str(&body).map_or(Reply::Page, Reply::Events))
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
