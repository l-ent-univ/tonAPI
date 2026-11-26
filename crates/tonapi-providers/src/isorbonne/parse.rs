//! Page d'accueil d'iSorbonne, sans réseau : session ouverte ou non, nom de l'étudiant, tuiles
//! des applications. Repris des analyses de Tafac (`server/portalSession.js`), dont les tests
//! portent une page réelle anonymisée ; aucune page réelle n'a été lue par tonAPI.

use tonapi_core::{App, Problem, Profile};
use url::Url;

use super::PROVIDER;
use crate::html::{Tag, inner, tags, text};
use crate::net::upstream;

/// Au-delà, la liste est suspecte et tronquée.
const MAX_APPS: usize = 500;
/// Le texte d'aide d'une tuile est long (du HTML) : seul son début sert de description.
const DESCRIPTION_MAX: usize = 160;
/// Formulaire qui porte les tuiles.
const TILES_FORM: &str = "id=\"portalForm\"";

/// La page est celle d'un étudiant connecté : elle a le lien de déconnexion et le formulaire des
/// tuiles.
pub(crate) fn signed_in(html: &str) -> bool {
    logout_link(html).is_some() && has_tiles_form(html)
}

fn has_tiles_form(html: &str) -> bool {
    html.match_indices(TILES_FORM).any(|(at, _)| {
        // L'attribut `id`, pas la fin d'un autre (`data-id`, `forid`).
        at.checked_sub(1)
            .and_then(|before| html.as_bytes().get(before))
            .is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-'))
    })
}

/// Lien de déconnexion : `logout.xhtml`, que le portail écrit relatif et qu'une page enregistrée
/// par un navigateur porte en absolu.
fn logout_link(html: &str) -> Option<Tag<'_>> {
    tags(html, "a").into_iter().find(|link| {
        link.attr("href").is_some_and(|href| {
            href.eq_ignore_ascii_case("logout.xhtml")
                || href.to_ascii_lowercase().ends_with("/logout.xhtml")
        })
    })
}

/// Profil lu sur la page d'accueil. Le portail ne donne ni identifiant ni adresse : l'identifiant
/// est celui de la connexion. Le lien de déconnexion porte « Nom Prénom », le nom d'abord : le
/// dernier mot est pris pour le prénom, et un prénom en deux mots laisse son premier mot au nom.
pub(crate) fn parse_profile(html: &str, username: &str) -> Profile {
    let written = logout_link(html)
        .and_then(|link| inner(html, &link, "a"))
        .map(text)
        .filter(|name| !name.is_empty());
    let (display_name, first_name, last_name) = match written.as_deref().map(|n| n.rsplit_once(' '))
    {
        Some(Some((family, given))) => (
            Some(format!("{given} {family}")),
            Some(given.to_owned()),
            Some(family.to_owned()),
        ),
        Some(None) => (written.clone(), written.clone(), None),
        None => (None, None, None),
    };
    Profile {
        username: username.to_owned(),
        display_name,
        first_name,
        last_name,
        email: None,
        student_number: None,
        program: None,
    }
}

/// Tuiles de la page : les boutons de rubrique (`input.toglem`, identifiant `t<n>`, libellé dans
/// `value`), puis un `div.tuile` par application, rangé dans la rubrique de sa classe `t<n>`,
/// avec son lien (`a.links`), son libellé (`label#portalForm:p<identifiant>`) et un texte d'aide
/// facultatif (`div#infos<identifiant>`). Les liens relatifs partent de la racine de l'hôte du
/// portail. Les applications sont rendues rubrique par rubrique, dans l'ordre des boutons.
///
/// Une page sans bouton de rubrique, ou dont aucune tuile n'est lisible, n'est plus celle
/// attendue.
pub(crate) fn parse_tiles(html: &str, portal: &Url) -> Result<Vec<App>, Problem> {
    let origin = portal
        .join("/")
        .map_err(|_| upstream(PROVIDER, "adresse du portail illisible"))?;

    // Rubriques, dans l'ordre des boutons.
    let mut sections: Vec<(String, String, Vec<App>)> = Vec::new();
    for button in tags(html, "input") {
        if !button.has_class("toglem") {
            continue;
        }
        let (Some(id), Some(title)) = (button.attr("id"), button.attr("value")) else {
            continue;
        };
        let title = title.trim().to_owned();
        if !is_section_id(&id) || title.is_empty() {
            continue;
        }
        match sections.iter_mut().find(|(found, _, _)| *found == id) {
            Some(section) => section.1 = title,
            None => sections.push((id, title, Vec::new())),
        }
    }
    if sections.is_empty() {
        return Err(upstream(
            PROVIDER,
            "rubriques des tuiles absentes de la page",
        ));
    }

    let tiles: Vec<Tag<'_>> = tags(html, "div")
        .into_iter()
        .filter(|div| {
            div.attr("class")
                .is_some_and(|classes| classes.split_whitespace().next() == Some("tuile"))
        })
        .collect();
    let mut seen: Vec<String> = Vec::new();
    for (at, tile) in tiles.iter().enumerate() {
        let end = tiles.get(at + 1).map_or(html.len(), |next| next.start);
        let Some(block) = html.get(tile.end..end) else {
            continue;
        };
        // La dernière classe `t<n>` de la tuile nomme sa rubrique.
        let category = tile.attr("class").and_then(|classes| {
            classes
                .split_whitespace()
                .rfind(|class| is_section_id(class))
                .map(str::to_owned)
        });
        let Some(section) =
            category.and_then(|category| sections.iter_mut().find(|(id, _, _)| *id == category))
        else {
            continue;
        };
        let Some(app) = tile_app(block, &origin, &section.1) else {
            continue;
        };
        if seen.len() >= MAX_APPS || seen.contains(&app.id) {
            continue;
        }
        seen.push(app.id.clone());
        section.2.push(app);
    }
    if seen.is_empty() && !tiles.is_empty() {
        return Err(upstream(PROVIDER, "aucune tuile lisible sur la page"));
    }
    Ok(sections.into_iter().flat_map(|(_, _, apps)| apps).collect())
}

/// `t` suivi de chiffres : l'identifiant d'un bouton de rubrique, et la classe d'une tuile.
fn is_section_id(id: &str) -> bool {
    id.strip_prefix('t')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// L'application d'une tuile, rangée dans `category`.
fn tile_app(block: &str, origin: &Url, category: &str) -> Option<App> {
    let link = tags(block, "a")
        .into_iter()
        .find(|link| link.attr("class").as_deref() == Some("links"))?;
    let (id, label) = tags(block, "label").into_iter().find_map(|label| {
        let id = label.attr("id")?;
        let id = id.strip_prefix("portalForm:p")?;
        (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then(|| (id.to_owned(), label))
    })?;
    let url = origin
        .join(link.attr("href")?.trim())
        .ok()
        .filter(|url| matches!(url.scheme(), "https" | "http"))?;
    // Le portail écrit certains libellés avec un deux-points final (« Mes documents : »).
    let title = text(inner(block, &label, "label")?);
    let title = title
        .strip_suffix(':')
        .map_or(title.as_str(), str::trim_end)
        .to_owned();
    if title.is_empty() {
        return None;
    }
    let infos = format!("infos{id}");
    let description = tags(block, "div")
        .into_iter()
        .find(|div| div.attr("id").as_deref() == Some(infos.as_str()))
        .and_then(|div| inner(block, &div, "div"))
        .map(|help| shortened(&text(help)))
        .filter(|help| !help.is_empty());
    Some(App {
        id,
        title,
        description,
        url: url.into(),
        icon_url: None,
        category: Some(category.to_owned()),
        favorite: false,
    })
}

/// Début d'un texte d'aide : au plus [`DESCRIPTION_MAX`] caractères, coupé sur un mot entier et
/// suivi de « … ».
fn shortened(help: &str) -> String {
    if help.chars().count() <= DESCRIPTION_MAX {
        return help.to_owned();
    }
    let head: String = help.chars().take(DESCRIPTION_MAX).collect();
    let cut = match head.rfind(char::is_whitespace) {
        Some(space) => head.get(..space).unwrap_or_default().trim_end(),
        None => head.as_str(),
    };
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonapi_core::ErrorCode;

    const HOME: &str = include_str!("../../tests/fixtures/isorbonne/accueil.html");

    fn portal() -> Url {
        Url::parse("https://portail.exemple.test/ENT/").unwrap()
    }

    #[test]
    fn session_ouverte_avec_le_lien_de_deconnexion_et_les_tuiles() {
        assert!(signed_in(HOME));
        let form = r#"<form id="portalForm"></form>"#;
        // Lien relatif (tel que servi) ou absolu (page enregistrée).
        assert!(signed_in(&format!(r#"<a href="logout.xhtml">x</a>{form}"#)));
        assert!(signed_in(&format!(
            r#"<a href="https://portail.exemple.test/ENT/logout.xhtml">x</a>{form}"#
        )));
        assert!(!signed_in(&format!(
            r#"<a href="nologout.xhtml">x</a>{form}"#
        )));
        // L'un sans l'autre ne suffit pas.
        assert!(!signed_in(r#"<a href="/ENT/logout.xhtml">x</a>"#));
        assert!(!signed_in(form));
        assert!(!signed_in(
            r#"<a href="logout.xhtml">x</a><form data-id="portalForm"></form>"#
        ));
        assert!(!signed_in(""));
    }

    #[test]
    fn nom_lu_dans_le_lien_de_deconnexion_le_nom_d_abord() {
        let profile = parse_profile(HOME, "cdemo");
        assert_eq!(profile.username, "cdemo");
        assert_eq!(profile.display_name.as_deref(), Some("Camille Le Demo"));
        assert_eq!(profile.first_name.as_deref(), Some("Camille"));
        assert_eq!(profile.last_name.as_deref(), Some("Le Demo"));
        assert_eq!(profile.email, None);

        let alone = parse_profile(r#"<a href="/ENT/logout.xhtml">Camille</a>"#, "cdemo");
        assert_eq!(alone.display_name.as_deref(), Some("Camille"));
        assert_eq!(alone.first_name.as_deref(), Some("Camille"));
        assert_eq!(alone.last_name, None);

        let empty = parse_profile(r#"<a href="/ENT/logout.xhtml"> <i></i></a>"#, "cdemo");
        assert_eq!(empty.display_name, None);
        assert_eq!(empty.username, "cdemo");
    }

    #[test]
    fn tuiles_rangees_par_bouton_de_rubrique() {
        let apps = parse_tiles(HOME, &portal()).unwrap();
        let read: Vec<_> = apps
            .iter()
            .map(|app| {
                (
                    app.id.as_str(),
                    app.title.as_str(),
                    app.url.as_str(),
                    app.category.as_deref(),
                )
            })
            .collect();
        // Le lien `javascript:` et la tuile d'une rubrique sans bouton sont écartés ; un lien
        // relatif part de la racine de l'hôte.
        assert_eq!(
            read,
            [
                (
                    "35",
                    "Mes documents",
                    "https://apps.exemple.test/Fichiers",
                    Some("Accueil")
                ),
                (
                    "54",
                    "Mon dossier de scolarité",
                    "https://apps.exemple.test/dossier/",
                    Some("Etudiant")
                ),
                (
                    "61",
                    "Aide",
                    "https://portail.exemple.test/ENT/secure/aide.xhtml",
                    Some("Etudiant")
                ),
            ]
        );
        assert_eq!(apps.first().unwrap().description, None);
        let help = apps.get(1).unwrap().description.as_deref().unwrap();
        assert!(help.starts_with("Notes et certificat de scolarité."));
        assert!(help.ends_with('…') && help.chars().count() <= DESCRIPTION_MAX + 1);
        assert!(!help.ends_with(" …"));
    }

    #[test]
    fn page_sans_rubrique_ou_sans_tuile_lisible() {
        let no_buttons = parse_tiles(r#"<form id="portalForm"></form>"#, &portal());
        assert_eq!(no_buttons.unwrap_err().code, ErrorCode::UpstreamChanged);

        // Des rubriques sans aucune tuile : une liste vide, pas une erreur.
        let empty = r#"<input class="toglem" value="Accueil" id="t1">"#;
        assert!(parse_tiles(empty, &portal()).unwrap().is_empty());

        let unreadable = r#"<input class="toglem" value="Accueil" id="t1">
            <div class="tuile t1"><a class="links" href="/x"><span>Sans libellé</span></a></div>"#;
        assert_eq!(
            parse_tiles(unreadable, &portal()).unwrap_err().code,
            ErrorCode::UpstreamChanged
        );
    }

    #[test]
    fn texte_d_aide_court_garde_entier() {
        assert_eq!(shortened("Notes et certificat."), "Notes et certificat.");
        let long = "mot ".repeat(60);
        let cut = shortened(long.trim());
        assert!(cut.ends_with("mot…"));
        assert!(cut.chars().count() <= DESCRIPTION_MAX + 1);
        // Sans espace où couper, le début est gardé tel quel.
        assert_eq!(
            shortened(&"x".repeat(200)).chars().count(),
            DESCRIPTION_MAX + 1
        );
    }
}
