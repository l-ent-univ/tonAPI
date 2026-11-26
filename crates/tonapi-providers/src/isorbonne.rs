//! Fournisseur `portal` pour iSorbonne (`portal.kind = "isorbonne"`), le portail JSF de la
//! Sorbonne Nouvelle.
//!
//! Une seule page, `secure/index.xhtml`, derrière le CAS de l'université : elle porte le lien de
//! déconnexion (avec le nom de l'étudiant) et les tuiles des applications, et renvoie au CAS tant
//! que l'étudiant n'a pas de session. Ni layout ni API. Le portail ne donne ni identifiant ni
//! adresse : le profil garde l'identifiant de la connexion, sans e-mail.
//!
//! Session perdue : la page renvoie au CAS. Avec le TGC de la session, le CAS rend un ticket et
//! la page arrive, sinon c'est `session_expired` sans nouvel essai. Une page servie sans le lien
//! de déconnexion ou sans les tuiles donne lieu à **un seul** nouvel essai, les anciens cookies
//! du portail retirés.
//!
//! Source : le code de Tafac (`server/portalSession.js`, `server/entAuthApp.js`),
//! `docs/recherche/sorbonne-nouvelle.md`. Jamais essayé avec un compte réel par tonAPI.

use reqwest::StatusCode;
use tonapi_config::PortalConfig;
use tonapi_core::{App, ErrorCode, Problem, Profile};
use url::Url;

use crate::Context;
use crate::common::root;
use crate::net::{Net, upstream};

mod parse;

use parse::{parse_profile, parse_tiles, signed_in};

/// Nom du logiciel dans les messages d'erreur.
const PROVIDER: &str = "isorbonne";
/// Page d'accueil, sous la racine de l'application.
const HOME: &str = "secure/index.xhtml";

/// Racine de l'application, avec une barre finale.
fn base(ctx: &Context<'_>) -> Result<Url, Problem> {
    match &ctx.university.university.portal {
        PortalConfig::Isorbonne { base_url } => Ok(root(base_url)),
        _ => Err(Problem::new(ErrorCode::ServiceAbsent)
            .detail("aucun portail iSorbonne n'est déclaré pour cette université")),
    }
}

/// Page qui ouvre une session du portail : son accueil, qui renvoie au CAS avec son propre
/// service.
pub(crate) fn entry_url(base_url: &Url) -> Url {
    let base = root(base_url);
    base.join(HOME).unwrap_or(base)
}

// Récupération

/// Profil de l'étudiant, lu dans le lien de déconnexion de la page d'accueil.
pub(crate) async fn fetch_profile(ctx: &mut Context<'_>) -> Result<Profile, Problem> {
    let base = base(ctx)?;
    let page = fetch_home(ctx, &base).await?;
    Ok(parse_profile(&page, &ctx.session.username))
}

/// Applications de l'étudiant : les tuiles de la page d'accueil.
pub(crate) async fn fetch_apps(ctx: &mut Context<'_>) -> Result<Vec<App>, Problem> {
    let base = base(ctx)?;
    let page = fetch_home(ctx, &base).await?;
    parse_tiles(&page, &base)
}

/// La page d'accueil, servie à l'étudiant connecté. Les cookies repartent toujours dans la
/// session de l'étudiant.
async fn fetch_home(ctx: &mut Context<'_>, base: &Url) -> Result<String, Problem> {
    let mut net = Net::open(ctx, PROVIDER)?;
    let result = async {
        if let Some(page) = home_once(&mut net, base).await? {
            return Ok(page);
        }
        // Page servie sans la session de l'étudiant : un seul nouvel essai, sans les anciens
        // cookies du portail. Un portail servi par l'hôte du CAS garde ses cookies : le TGC
        // partirait avec eux.
        let cas_host = net.cas.as_ref().and_then(|cas| cas.base.host_str());
        if let Some(host) = base.host_str()
            && cas_host != Some(host)
        {
            net.browser.jar.remove_host(host);
        }
        home_once(&mut net, base).await?.ok_or_else(|| {
            Problem::new(ErrorCode::SessionExpired)
                .detail("le portail n'a pas de session ouverte pour ce compte")
        })
    }
    .await;
    net.save(ctx);
    result
}

/// La page d'accueil, si le portail la sert à l'étudiant connecté ; `None` s'il la sert sans
/// session. Le CAS qui redemande le mot de passe donne `session_expired`
/// ([`Net::open_protected`]).
async fn home_once(net: &mut Net, base: &Url) -> Result<Option<String>, Problem> {
    let page = net.open_protected(entry_url(base)).await?;
    let on_portal = page.url.host_str() == base.host_str()
        && page.url.port_or_known_default() == base.port_or_known_default();
    if !on_portal {
        return Err(upstream(
            PROVIDER,
            "la page d'accueil a mené hors du portail",
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
