//! Fournisseur `portal` pour un portail Drupal (`portal.kind = "drupal"`).
//!
//! Le portail est un site Drupal derrière le CAS de l'université : sa racine renvoie au CAS tant
//! que l'étudiant n'y a pas de session, et le CAS revient avec un ticket. Il n'a ni layout ni
//! API ; tout se lit sur des pages :
//!
//! | Donnée | Page | Lecture |
//! |---|---|---|
//! | session ouverte | n'importe laquelle | classe `user-logged-in` du `<body>` |
//! | nom de l'étudiant | `user` (redirigée vers `user/<numéro>`) | champ `field--name-<name-field>` |
//! | applications | `[portal.catalogue] path` | vue `view-id-<view>`, un `<h3>` par rubrique |
//!
//! Le portail ne donne ni identifiant ni adresse de l'étudiant : le profil garde l'identifiant de
//! la connexion, sans e-mail.
//!
//! Session perdue : la page demandée renvoie au CAS. Avec le TGC de la session, le CAS rend un
//! ticket et la page arrive, sinon c'est `session_expired` sans nouvel essai. Une page servie par
//! le portail sans la classe du compte donne lieu à **un seul** nouvel essai, après une entrée par
//! la racine du portail, ses anciens cookies retirés.
//!
//! Source : le code de Tafac pour Rennes 2 (`server/portalSession.js`, `server/entAuthApp.js`),
//! `docs/recherche/rennes-2.md`. Jamais essayé avec un compte réel par tonAPI.

use reqwest::StatusCode;
use tonapi_config::{DrupalCatalogue, PortalConfig};
use tonapi_core::{App, ErrorCode, Problem, Profile};
use url::Url;

use crate::Context;
use crate::common::root;
use crate::net::{Net, upstream};

mod parse;

use parse::{parse_catalogue, parse_profile, signed_in};

/// Nom du logiciel dans les messages d'erreur.
const PROVIDER: &str = "drupal";

/// Portail configuré pour l'université de l'étudiant.
struct Portal {
    /// Racine du portail, avec une barre finale.
    base: Url,
    name_field: Option<String>,
    catalogue: Option<DrupalCatalogue>,
}

fn portal(ctx: &Context<'_>) -> Result<Portal, Problem> {
    match &ctx.university.university.portal {
        PortalConfig::Drupal {
            base_url,
            name_field,
            catalogue,
        } => Ok(Portal {
            base: root(base_url),
            name_field: name_field.clone(),
            catalogue: catalogue.clone(),
        }),
        _ => Err(Problem::new(ErrorCode::ServiceAbsent)
            .detail("aucun portail Drupal n'est déclaré pour cette université")),
    }
}

/// Page qui ouvre une session du portail : sa racine, qui renvoie au CAS avec son propre service.
pub(crate) fn entry_url(base_url: &Url) -> Url {
    root(base_url)
}

// Récupération

/// Profil de l'étudiant, lu sur la page `user`.
pub(crate) async fn fetch_profile(ctx: &mut Context<'_>) -> Result<Profile, Problem> {
    let portal = portal(ctx)?;
    let page = fetch_page(ctx, &portal, "user").await?;
    Ok(parse_profile(
        &page,
        portal.name_field.as_deref(),
        &ctx.session.username,
    ))
}

/// Applications de l'étudiant, lues sur la page du catalogue.
pub(crate) async fn fetch_apps(ctx: &mut Context<'_>) -> Result<Vec<App>, Problem> {
    let portal = portal(ctx)?;
    let Some(catalogue) = &portal.catalogue else {
        return Err(Problem::new(ErrorCode::ServiceAbsent)
            .detail("aucune page de catalogue n'est déclarée pour ce portail"));
    };
    let page = fetch_page(ctx, &portal, &catalogue.path).await?;
    parse_catalogue(&page, catalogue, &portal.base)
}

/// Une page du portail, servie à l'étudiant connecté. Les cookies repartent toujours dans la
/// session de l'étudiant.
async fn fetch_page(ctx: &mut Context<'_>, portal: &Portal, path: &str) -> Result<String, Problem> {
    let url = portal
        .base
        .join(path)
        .ok()
        .filter(|url| same_host(url, &portal.base))
        .ok_or_else(|| Problem::new(ErrorCode::Internal).detail("adresse du portail illisible"))?;
    let mut net = Net::open(ctx, PROVIDER)?;
    let result = async {
        if let Some(page) = page_once(&mut net, portal, url.clone()).await? {
            return Ok(page);
        }
        // Le portail a servi la page sans la session de l'étudiant : on entre de nouveau par sa
        // racine, sans ses anciens cookies, puis un seul nouvel essai. Un portail servi par
        // l'hôte du CAS garde ses cookies : le TGC partirait avec eux.
        let cas_host = net.cas.as_ref().and_then(|cas| cas.base.host_str());
        if let Some(host) = portal.base.host_str()
            && cas_host != Some(host)
        {
            net.browser.jar.remove_host(host);
        }
        net.open_protected(entry_url(&portal.base)).await?;
        page_once(&mut net, portal, url).await?.ok_or_else(lost)
    }
    .await;
    net.save(ctx);
    result
}

/// La page, si le portail la sert à l'étudiant connecté ; `None` s'il la sert sans session. Le
/// CAS qui redemande le mot de passe donne `session_expired` ([`Net::open_protected`]).
async fn page_once(net: &mut Net, portal: &Portal, url: Url) -> Result<Option<String>, Problem> {
    let page = net.open_protected(url).await?;
    if !same_host(&page.url, &portal.base) {
        return Err(upstream(
            PROVIDER,
            "la page demandée a mené hors du portail",
        ));
    }
    if page.status.is_server_error() {
        return Err(Problem::new(ErrorCode::ServiceUnavailable).detail(format!(
            "{PROVIDER} : le portail répond HTTP {}",
            page.status.as_u16()
        )));
    }
    Ok((page.status == StatusCode::OK && signed_in(&page.body)).then_some(page.body))
}

fn same_host(url: &Url, base: &Url) -> bool {
    url.host_str() == base.host_str() && url.port_or_known_default() == base.port_or_known_default()
}

/// Session perdue que le CAS n'a pas rouverte.
fn lost() -> Problem {
    Problem::new(ErrorCode::SessionExpired)
        .detail("le portail n'a pas de session ouverte pour ce compte")
}
