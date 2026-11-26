//! uPortal : applications et profil de l'étudiant, lus dans le portail déclaré dans `[portal]`.
//!
//! Deux variantes, choisies par la configuration de l'université :
//!
//! | Variante | Applications | Profil |
//! |---|---|---|
//! | `uportal4-prolongation` | `layout.jsp` (JSONP d'ESUP ProlongationENT) | `api/session.json` puis `api/people/<id>.json` |
//! | `uportal5` | `api/v4-3/dlm/layout.json` | `api/v5-1/userinfo` (JWT), sinon comme uPortal 4 |
//!
//! Le portail ne signale pas toujours une session perdue par une erreur : il redirige vers
//! `/Login`, ou répond 200 avec un compte invité (`guest`, `authenticated: "false"`). Les deux cas
//! donnent `session_expired` ; le fournisseur rouvre alors le portail par le CAS (TGC de la
//! session) et réessaie **une seule fois**.
//!
//! Sources : `docs/recherche/montpellier/ent.md` (uPortal 4.3.1, confirmé avec un compte réel),
//! `docs/recherche/rennes/apis/uportal5.md` (uPortal 5 : réponse invitée, puis session connectée
//! observée le 26/11/2026, §10).

use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode};
use tonapi_config::PortalConfig;
use tonapi_core::{App, ErrorCode, Problem, Profile};
use url::Url;

use std::time::Duration;

use crate::Context;
use crate::cache::Shared;
use crate::common::root;
use crate::net::{Net, upstream};

mod parse;

use parse::{
    fragment_link, parse_profile_v4, parse_prolongation_layout, parse_session,
    parse_uportal5_layout, parse_userinfo, portal_page, shareable,
};

/// Lien direct de chaque portlet (`None` : pas de lien dans le fragment), par portail et `fname`.
static PORTLET_LINKS: Shared<Option<String>> = Shared::new(Duration::from_secs(6 * 3600));

/// Nom du logiciel dans les messages d'erreur.
const PROVIDER: &str = "uportal";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variant {
    /// uPortal 4 avec ESUP ProlongationENT.
    Prolongation,
    /// uPortal 5.
    Uportal5,
}

/// Portail configuré pour l'université de l'étudiant.
struct Portal {
    variant: Variant,
    /// Racine du portail, avec une barre finale.
    base: Url,
    /// Identifiant de connexion de l'étudiant, si le portail n'en donne pas.
    account: String,
}

fn portal(ctx: &Context<'_>) -> Result<Portal, Problem> {
    let (variant, base_url) = match &ctx.university.university.portal {
        PortalConfig::Uportal4Prolongation { base_url } => (Variant::Prolongation, base_url),
        PortalConfig::Uportal5 { base_url } => (Variant::Uportal5, base_url),
        PortalConfig::TilesPortal { .. } | PortalConfig::None => {
            return Err(Problem::new(ErrorCode::ServiceAbsent)
                .detail("aucun portail n'est déclaré pour cette université"));
        }
    };
    Ok(Portal {
        variant,
        base: root(base_url),
        account: ctx.session.username.clone(),
    })
}

// Récupération

/// Applications de l'étudiant, lues dans le layout du portail.
pub(crate) async fn fetch_apps(ctx: &mut Context<'_>) -> Result<Vec<App>, Problem> {
    with_portal(ctx, async |net: &mut Net, portal: &Portal| {
        apps_once(net, portal).await
    })
    .await
}

/// Profil de l'étudiant, lu dans le portail.
pub(crate) async fn fetch_profile(ctx: &mut Context<'_>) -> Result<Profile, Problem> {
    with_portal(ctx, async |net: &mut Net, portal: &Portal| {
        profile_once(net, portal).await
    })
    .await
}

/// Ouvre le réseau, exécute `attempt` avec la session mémorisée, et la rouvre une fois si le
/// portail dit qu'elle est perdue. Les cookies repartent toujours dans la session de l'étudiant.
async fn with_portal<T>(
    ctx: &mut Context<'_>,
    mut attempt: impl AsyncFnMut(&mut Net, &Portal) -> Result<T, Problem>,
) -> Result<T, Problem> {
    let portal = portal(ctx)?;
    let mut net = Net::open(ctx, PROVIDER)?;
    let entry = entry_url(&net, &portal.base);
    let result = async {
        match attempt(&mut net, &portal).await {
            Err(problem) if problem.code == ErrorCode::SessionExpired => {
                // Le CAS redemande le mot de passe : `open_protected` rend `session_expired`
                // sans nouvel essai.
                net.open_protected(entry).await?;
                attempt(&mut net, &portal).await
            }
            other => other,
        }
    }
    .await;
    net.save(ctx);
    result
}

/// Page qui ouvre une session portail : le CAS avec le service `<portail>/Login`, comme le bouton
/// « Connexion » ; la racine d'un uPortal peut afficher une page invitée au lieu de rediriger.
fn entry_url(net: &Net, base: &Url) -> Url {
    let login = base.join("Login").unwrap_or_else(|_| base.clone());
    match &net.cas {
        Some(cas) => cas.login_url(Some(&login)),
        None => login,
    }
}

async fn apps_once(net: &mut Net, portal: &Portal) -> Result<Vec<App>, Problem> {
    match portal.variant {
        Variant::Prolongation => {
            let url = route(&portal.base, "layout.jsp?callback=x")?;
            let body = get(
                net,
                url,
                "application/javascript, application/json, */*",
                &portal.base,
            )
            .await?;
            parse_prolongation_layout(&body, &portal.base)
        }
        Variant::Uportal5 => {
            let url = route(&portal.base, "api/v4-3/dlm/layout.json")?;
            let body = get(net, url, "application/json", &portal.base).await?;
            let mut apps = parse_uportal5_layout(&body, &portal.base)?;
            direct_links(net, portal, &mut apps).await;
            Ok(apps)
        }
    }
}

/// Fragments lus au plus par appel (un par portlet sans lien, environ 40 ms chacun).
const MAX_FRAGMENTS: usize = 60;

/// Remplace la page `<portail>/p/<fname>` par le lien du fragment du portlet
/// (`api/v4-3/portlet/<fname>.html`), quand il en contient un. Un fragment illisible garde la
/// page du portail, qui reste un lien valide.
async fn direct_links(net: &mut Net, portal: &Portal, apps: &mut [App]) {
    let pending = apps
        .iter_mut()
        .filter(|app| {
            portal_page(&portal.base, &app.id).is_some_and(|page| page.as_str() == app.url)
        })
        .take(MAX_FRAGMENTS);
    for app in pending {
        let key = format!("{}|{}", portal.base, app.id);
        if let Some(cached) = PORTLET_LINKS.get(&key) {
            if let Some(link) = cached {
                app.url = link;
            }
            continue;
        }
        let Ok(mut url) = route(&portal.base, "api/v4-3/portlet/") else {
            return;
        };
        let Ok(mut segments) = url.path_segments_mut() else {
            return;
        };
        segments.pop_if_empty().push(&format!("{}.html", app.id));
        drop(segments);
        let Ok((status, body)) = fetch(net, url, "text/html", &portal.base).await else {
            continue;
        };
        if !status.is_success() {
            continue;
        }
        let link = fragment_link(&body, &portal.base).map(String::from);
        // Un lien qui pourrait être propre à l'étudiant n'est jamais partagé.
        if link
            .as_deref()
            .is_none_or(|l| shareable(l, &portal.account))
        {
            PORTLET_LINKS.put(key, link.clone());
        }
        if let Some(link) = link {
            app.url = link;
        }
    }
}
async fn profile_once(net: &mut Net, portal: &Portal) -> Result<Profile, Problem> {
    match portal.variant {
        Variant::Prolongation => profile_people(net, portal).await,
        Variant::Uportal5 => {
            let url = route(&portal.base, "api/v5-1/userinfo")?;
            let (status, body) = fetch(
                net,
                url,
                "application/jwt, application/json, */*",
                &portal.base,
            )
            .await?;
            // Certains uPortal servent le layout v4-3 sans `userinfo` (404) mais gardent
            // `session.json` et `people` : on les lit à la place. [vu] Paul-Valéry, 27/10/2026.
            if status == StatusCode::NOT_FOUND {
                return profile_people(net, portal).await;
            }
            check_status(status)?;
            parse_userinfo(&body, &portal.account)
        }
    }
}

/// Profil par `api/session.json` puis `api/people/<identifiant>.json`.
async fn profile_people(net: &mut Net, portal: &Portal) -> Result<Profile, Problem> {
    let url = route(&portal.base, "api/session.json")?;
    let body = get(net, url, "application/json", &portal.base).await?;
    let session = parse_session(&body)?;
    // Le prénom, le nom et l'e-mail dépendent d'une permission du portail : un refus, une
    // erreur ou un portail muet laissent le profil réduit à l'identifiant et au nom
    // affiché. Seule une redirection vers `/Login` est une session perdue.
    let mut people = None;
    if let Some(url) = people_url(&portal.base, &session.username) {
        match fetch(net, url, "application/json", &portal.base).await {
            Ok((status, _)) if status.is_redirection() => return Err(lost()),
            Ok((status, body)) if status.is_success() => people = Some(body),
            Ok(_) | Err(_) => {}
        }
    }
    Ok(parse_profile_v4(&session, people.as_deref()))
}

fn route(base: &Url, path: &str) -> Result<Url, Problem> {
    base.join(path)
        .map_err(|_| Problem::new(ErrorCode::Internal).detail("adresse du portail illisible"))
}

/// `api/people/<identifiant>.json`, l'identifiant étant encodé comme un segment de chemin.
fn people_url(base: &Url, username: &str) -> Option<Url> {
    let mut url = route(base, "api/people/").ok()?;
    url.path_segments_mut()
        .ok()?
        .pop_if_empty()
        .push(&format!("{username}.json"));
    Some(url)
}

/// `GET` sans suivre de redirection : statut et corps.
async fn fetch(
    net: &mut Net,
    url: Url,
    accept: &'static str,
    referer: &Url,
) -> Result<(StatusCode, String), Problem> {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static(accept));
    if let Ok(referer) = HeaderValue::from_str(referer.as_str()) {
        headers.insert(header::REFERER, referer);
    }
    let (status, _, body) = net.request(Method::GET, url, headers, None).await?;
    Ok((status, body))
}

/// Corps d'une réponse 200 ; les autres statuts deviennent un problème (voir [`check_status`]).
async fn get(
    net: &mut Net,
    url: Url,
    accept: &'static str,
    referer: &Url,
) -> Result<String, Problem> {
    let (status, body) = fetch(net, url, accept, referer).await?;
    check_status(status)?;
    Ok(body)
}

/// Session perdue : le portail renvoie vers `/Login` ou refuse l'accès.
fn lost() -> Problem {
    Problem::new(ErrorCode::SessionExpired)
        .detail("le portail n'a pas de session ouverte pour ce compte")
}

fn check_status(status: StatusCode) -> Result<(), Problem> {
    if status.is_success() {
        Ok(())
    } else if status.is_redirection()
        || status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
    {
        Err(lost())
    } else if status.is_server_error() {
        Err(Problem::new(ErrorCode::ServiceUnavailable).detail(format!(
            "{PROVIDER} : le portail répond HTTP {}",
            status.as_u16()
        )))
    } else {
        Err(upstream(
            PROVIDER,
            format!("réponse HTTP {} inattendue", status.as_u16()),
        ))
    }
}

#[cfg(test)]
mod tests;
