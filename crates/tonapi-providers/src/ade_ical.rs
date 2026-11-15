//! Fournisseur `ade-ical-link` : emploi du temps ADE lu dans le flux iCal personnel de l'étudiant.
//!
//! Chaîne suivie (`docs/recherche/montpellier/lien-ade.md` et `emploi-du-temps.md` §9) :
//! 1. la page `link-page` de la configuration, derrière le CAS de l'université, contient le lien du
//!    flux `…/direct_cal.jsp?data=<hex>,1` ;
//! 2. ce lien se télécharge sans cookie, mais uniquement sur `feed-host` : un lien lu sur une page
//!    qui vise un autre hôte est refusé ;
//! 3. le flux iCal (VEVENT, dates UTC) est converti en [`Course`] dans le fuseau de l'université.
//!
//! Le lien donne l'emploi du temps sans connexion : c'est un secret. Il est gardé dans
//! `ctx.session.tokens["ade.ical"]` (session chiffrée) pour ne pas rouvrir la page à chaque appel,
//! et n'apparaît jamais dans une erreur. Quand le flux ne renvoie plus un calendrier (lien périmé
//! après un changement de groupes, ou générateur désactivé), la page est relue **une seule fois**.
//!
//! `feed-host` s'écrit `hôte` (port par défaut) ou `hôte:port`.

use chrono::NaiveDate;
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode};
use tonapi_core::{Course, ErrorCode, Problem};
use url::Url;

use crate::Context;
use crate::common::university_tz;
use crate::net::{Net, upstream};

mod fields;
mod ics;
mod vaadin;

pub(crate) use ics::{is_calendar, parse_timetable};
use vaadin::VaadinPage;

const PROVIDER: &str = "ade-ical-link";
/// Clé de [`tonapi_session::SessionData::tokens`] qui garde le lien du flux (secret).
const TOKEN_KEY: &str = "ade.ical";
/// Page ADE qui sert le flux.
const FEED_PAGE: &str = "direct_cal.jsp";
/// Vue voisine de prose par laquelle on passe pour que la vue iCal renvoie son lien.
const OTHER_VIEW: &str = "protected/resume";

// Récupération

/// Emploi du temps de l'étudiant entre `from` et `to` inclus.
pub(crate) async fn fetch_timetable(
    ctx: &mut Context<'_>,
    link_page: &Url,
    feed_host: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Course>, Problem> {
    let tz = university_tz(ctx)?;
    let https_only = ctx.net.https_only;
    let stored = ctx
        .session
        .tokens
        .get(TOKEN_KEY)
        .and_then(|raw| feed_link(raw, feed_host, https_only));

    let mut net = Net::open(ctx, PROVIDER)?;
    let feed = read_feed(&mut net, link_page, feed_host, https_only, stored).await;
    net.save(ctx);

    let (ics, link) = feed?;
    ctx.session
        .tokens
        .insert(TOKEN_KEY.to_owned(), link.to_string());
    parse_timetable(&ics, tz, from, to)
}

/// `timetable_groups` : la liste des groupes n'est pas lisible avec ce fournisseur.
///
/// La sélection de groupes vit dans prose-etudiant (pages `preferences` et `resume`, appli Vaadin
/// dont la grille se charge par des échanges UIDL non documentés) ; le flux iCal, lui, ne porte
/// que les séances, sans identifiant de groupe ni arbre. Dériver les groupes des séances
/// donnerait des groupes de séance (CM, TD parents…), pas la sélection de l'étudiant.
pub(crate) fn groups_unsupported() -> Problem {
    Problem::new(ErrorCode::NotImplemented).detail(format!(
        "le fournisseur `{PROVIDER}` ne peut pas lister les groupes : la sélection se fait sur le \
         portail de l'université, et le flux iCal ne contient que les séances"
    ))
}

/// Résultat d'un téléchargement du flux.
enum Feed {
    Calendar(String),
    /// Réponse qui n'est pas un calendrier : lien périmé ou fonction désactivée.
    Stale,
}

/// Lit le flux avec le lien gardé ; s'il est périmé (ou absent), relit la page une seule fois.
async fn read_feed(
    net: &mut Net,
    link_page: &Url,
    feed_host: &str,
    https_only: bool,
    stored: Option<Url>,
) -> Result<(String, Url), Problem> {
    if let Some(link) = stored
        && let Feed::Calendar(ics) = download(net, &link).await?
    {
        return Ok((ics, link));
    }
    let link = discover(net, link_page, feed_host, https_only).await?;
    match download(net, &link).await? {
        Feed::Calendar(ics) => Ok((ics, link)),
        Feed::Stale => Err(upstream(
            PROVIDER,
            "le lien lu sur la page ne renvoie pas un calendrier",
        )),
    }
}

/// Ouvre la page du lien derrière le CAS et en extrait le lien du flux.
async fn discover(
    net: &mut Net,
    link_page: &Url,
    feed_host: &str,
    https_only: bool,
) -> Result<Url, Problem> {
    let mut page = net.open_protected(link_page.clone()).await?;
    // Vaadin redirige vers `/invalidSession` quand sa session vient de naître : un nouvel essai.
    if page.url.path().ends_with("/invalidSession") {
        page = net.open_protected(link_page.clone()).await?;
    }
    if page.status == StatusCode::UNAUTHORIZED || page.status == StatusCode::FORBIDDEN {
        return Err(Problem::new(ErrorCode::SessionExpired));
    }
    if page.status.is_server_error() {
        return Err(unavailable(page.status));
    }
    if !page.status.is_success() {
        return Err(upstream(
            PROVIDER,
            format!("la page du lien répond HTTP {}", page.status),
        ));
    }
    if let Some(link) = find_feed_link(&page.body, feed_host, https_only) {
        return Ok(link);
    }
    // Prose (Vaadin Flow) ne met le lien dans la page que si la vue a déjà été affichée dans la
    // session ; sinon il l'envoie quand le client revient sur la vue depuis une autre. On rejoue
    // ce clic : « Groupes sélectionnés » puis « iCal ». [vu] 27/10/2026 : naviguer directement
    // vers la vue déjà affichée ne renvoie pas le lien.
    if let Some(mut vaadin) = VaadinPage::read(&page.url, &page.body) {
        let here = vaadin.location.clone();
        for location in [OTHER_VIEW, here.as_str()] {
            let body = vaadin.navigate(net, location).await?;
            if location == here
                && let Some(link) = find_feed_link(&body, feed_host, https_only)
            {
                return Ok(link);
            }
            if tracing::enabled!(tracing::Level::DEBUG) {
                tracing::debug!(
                    "{PROVIDER} : navigation Vaadin vers {location} : {} octets, marqueurs {:?}",
                    body.len(),
                    markers(&body)
                );
            }
        }
    }
    if tracing::enabled!(tracing::Level::DEBUG) {
        tracing::debug!(
            "{PROVIDER} : page sans lien de flux, marqueurs {:?}, liens {:?}",
            markers(&page.body),
            page_links(&page.body)
        );
    }
    Err(upstream(PROVIDER, "aucun lien de flux iCal sur la page"))
}

/// Mots-clés présents dans une réponse, pour le diagnostic (jamais de valeur).
fn markers(body: &str) -> Vec<&'static str> {
    [
        "uidl",
        "v-r=init",
        "initApplication",
        "Vaadin.Flow",
        "appConfig",
        "direct_cal",
        "proseconsult",
        "anonymous_cal",
        "ical",
        "invalidSession",
    ]
    .into_iter()
    .filter(|marker| body.contains(marker))
    .collect()
}

/// Adresses de la page, décrites sans valeur (hôte, chemin masqué, noms de paramètres).
fn page_links(body: &str) -> std::collections::BTreeSet<String> {
    let text = body
        .replace("\\/", "/")
        .replace("\\u0026", "&")
        .replace("&amp;", "&");
    text.match_indices("http")
        .filter_map(|(start, _)| {
            let candidate = text.get(start..)?;
            let end = candidate
                .find(|c: char| {
                    c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '\\' | '`')
                })
                .unwrap_or(candidate.len());
            Url::parse(candidate.get(..end)?).ok()
        })
        .map(|url| tonapi_http::describe(&url))
        .take(40)
        .collect()
}

fn unavailable(status: StatusCode) -> Problem {
    Problem::new(ErrorCode::ServiceUnavailable)
        .detail(format!("{PROVIDER} : le serveur ADE répond HTTP {status}"))
}

/// `GET` du flux, sans suivre de redirection (ADE répond 302 pour un lien invalide).
async fn download(net: &mut Net, link: &Url) -> Result<Feed, Problem> {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("text/calendar, text/*;q=0.9"),
    );
    // Le flux part sans aucun cookie de la session, et le `JSESSIONID` du serveur ADE ne la
    // rejoint pas.
    let jar = std::mem::take(&mut net.browser.jar);
    let reply = net.request(Method::GET, link.clone(), headers, None).await;
    net.browser.jar = jar;
    let (status, _, body) = reply?;
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        return Err(unavailable(status));
    }
    if status.is_success() && is_calendar(&body) {
        Ok(Feed::Calendar(body))
    } else {
        Ok(Feed::Stale)
    }
}

// Lien du flux

/// Longueur maximale d'un lien de flux.
const MAX_LINK: usize = 2048;

/// Premier lien `direct_cal.jsp?data=…` de la page qui vise `feed_host`.
///
/// La page est de l'HTML Vaadin avec un état JSON : le lien y est en clair, mais on défait les
/// échappements usuels (`\/`, `&amp;`, `&`) au cas où le rendu change.
fn find_feed_link(body: &str, feed_host: &str, https_only: bool) -> Option<Url> {
    let text = body
        .replace("\\/", "/")
        .replace("\\u0026", "&")
        .replace("&amp;", "&");
    let prefixes = [
        format!("https://{feed_host}"),
        format!("http://{feed_host}"),
    ];
    text.match_indices("http").find_map(|(start, _)| {
        let candidate = text.get(start..)?;
        // Seuls les liens vers l'hôte du flux, et au plus `MAX_LINK` octets : sinon chaque
        // `http` relirait toute la fin de la page (quadratique sur `httphttp…`).
        if !prefixes.iter().any(|p| {
            candidate
                .get(..p.len())
                .is_some_and(|c| c.eq_ignore_ascii_case(p))
        }) {
            return None;
        }
        let end = candidate.bytes().take(MAX_LINK).position(|b| {
            b.is_ascii_whitespace() || matches!(b, b'"' | b'\'' | b'<' | b'>' | b'\\' | b'`')
        });
        let end = end.or((candidate.len() <= MAX_LINK).then_some(candidate.len()))?;
        feed_link(candidate.get(..end)?, feed_host, https_only)
    })
}

/// Valide un lien de flux : https (http dans les tests), sans identifiants, sur `feed_host`, page
/// `direct_cal.jsp`, paramètre `data` composé de chiffres hexadécimaux et de virgules.
fn feed_link(raw: &str, feed_host: &str, https_only: bool) -> Option<Url> {
    let url = Url::parse(raw.trim()).ok()?;
    let scheme_ok = url.scheme() == "https" || (!https_only && url.scheme() == "http");
    let page_ok = url.path().rsplit('/').next() == Some(FEED_PAGE);
    let data_ok = url.query_pairs().any(|(key, value)| {
        key == "data"
            && !value.is_empty()
            && value.chars().all(|c| c.is_ascii_hexdigit() || c == ',')
    });
    (scheme_ok
        && url.username().is_empty()
        && url.password().is_none()
        && page_ok
        && data_ok
        && host_matches(&url, feed_host))
    .then_some(url)
}

fn host_matches(url: &Url, feed_host: &str) -> bool {
    let expected = feed_host.trim().to_ascii_lowercase();
    let Some(host) = url.host_str() else {
        return false;
    };
    match url.port() {
        Some(port) => expected == format!("{host}:{port}"),
        None => expected == host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = include_str!("../tests/fixtures/ade_ical/page-ical.html");
    const HOTE: &str = "ade.example.test";

    #[test]
    fn lien_extrait_de_la_page() {
        let link = find_feed_link(PAGE, HOTE, true).unwrap();
        assert_eq!(link.host_str(), Some(HOTE));
        assert!(link.path().ends_with("/direct_cal.jsp"));
        // Le lien « Visualiser le planning » (`direct/?data=`, interface GWT) n'est pas le flux.
        assert!(!link.path().contains("/direct/"));
        // Autre hôte, ou schéma http : refusé.
        assert!(find_feed_link(PAGE, "autre.example.test", true).is_none());
        let http = PAGE.replace("https://", "http://");
        assert!(find_feed_link(&http, HOTE, true).is_none());
        assert!(find_feed_link(&http, HOTE, false).is_some());
        assert!(find_feed_link("<html>rien</html>", HOTE, true).is_none());
    }

    /// Chaque `http` relisait toute la fin de la page.
    #[test]
    fn recherche_du_lien_lineaire() {
        let page = format!("{}{}", "http".repeat(200_000), PAGE);
        let started = std::time::Instant::now();
        assert!(find_feed_link(&page, HOTE, true).is_some());
        let long = format!("https://{HOTE}/{}", "a".repeat(MAX_LINK));
        assert!(find_feed_link(&long, HOTE, true).is_none());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn lien_tolere_les_echappements_du_rendu() {
        let page = r#"{"value":"https:\/\/ade.example.test\/jsp\/custom\/modules\/plannings\/direct_cal.jsp?data=0a1b2c,1"}"#;
        assert!(find_feed_link(page, HOTE, true).is_some());
        let page = r#"<a href="https://ade.example.test/jsp/direct_cal.jsp?data=0a1b&amp;x=1">"#;
        assert!(find_feed_link(page, HOTE, true).is_some());
    }

    #[test]
    fn lien_invalide_refuse() {
        let ok = "https://ade.example.test/jsp/custom/modules/plannings/direct_cal.jsp?data=0a1b,1";
        assert!(feed_link(ok, HOTE, true).is_some());
        for bad in [
            // identifiants dans l'URL
            "https://u:p@ade.example.test/jsp/direct_cal.jsp?data=0a1b,1",
            // autre page
            "https://ade.example.test/jsp/direct/?data=0a1b,1",
            "https://ade.example.test/jsp/direct_cal.jsp.evil?data=0a1b,1",
            // données qui ne sont pas de l'hexadécimal
            "https://ade.example.test/jsp/direct_cal.jsp?data=zz,1",
            "https://ade.example.test/jsp/direct_cal.jsp?data=",
            "https://ade.example.test/jsp/direct_cal.jsp",
            // port non prévu par la configuration
            "https://ade.example.test:8443/jsp/direct_cal.jsp?data=0a1b,1",
            // hôte voisin
            "https://ade.example.test.evil.fr/jsp/direct_cal.jsp?data=0a1b,1",
            "ftp://ade.example.test/jsp/direct_cal.jsp?data=0a1b,1",
            "pas une url",
        ] {
            assert!(feed_link(bad, HOTE, true).is_none(), "{bad}");
        }
        // Hôte configuré avec un port.
        let with_port = "https://ade.example.test:8443/jsp/direct_cal.jsp?data=0a1b,1";
        assert!(feed_link(with_port, "ade.example.test:8443", true).is_some());
        // La casse de la configuration est sans effet.
        assert!(feed_link(ok, "ADE.example.test", true).is_some());
    }

    #[test]
    fn groupes_non_pris_en_charge() {
        let problem = groups_unsupported();
        assert_eq!(problem.code, ErrorCode::NotImplemented);
        assert!(problem.detail.unwrap().contains("groupes"));
    }
}
