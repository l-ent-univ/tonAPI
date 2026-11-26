//! Pages d'un portail Drupal, sans réseau : session ouverte ou non, nom de l'étudiant, catalogue
//! des outils. Repris des analyses de Tafac (`server/portalSession.js`), dont les tests portent
//! des pages réelles anonymisées ; aucune page réelle n'a été lue par tonAPI.

use tonapi_config::DrupalCatalogue;
use tonapi_core::{App, Problem, Profile};
use url::Url;

use super::PROVIDER;
use crate::html::{inner, tags, text};
use crate::net::upstream;

/// Au-delà, la liste est suspecte et tronquée (un portail réel a quelques dizaines d'outils).
const MAX_APPS: usize = 500;

/// La page est celle d'un étudiant connecté : Drupal ajoute la classe `user-logged-in` au
/// `<body>` de toute page servie à un compte, et `user-anonymous` sinon.
pub(crate) fn signed_in(html: &str) -> bool {
    tags(html, "body")
        .iter()
        .find_map(|body| body.attr("class"))
        .is_some_and(|classes| {
            classes
                .split_whitespace()
                .any(|class| class == "user-logged-in")
        })
}

/// Profil lu sur la page `user`. Le portail ne donne ni identifiant ni adresse : l'identifiant
/// est celui de la connexion. `name_field` : le champ qui porte « Prénom Nom », en un seul texte.
/// Le premier mot est pris pour le prénom : un prénom en deux mots sans trait d'union laisse son
/// second mot au nom.
pub(crate) fn parse_profile(html: &str, name_field: Option<&str>, username: &str) -> Profile {
    let name = name_field.and_then(|field| field_text(html, field));
    let (first_name, last_name) = match name.as_deref().map(|name| name.split_once(' ')) {
        Some(Some((first, rest))) => (Some(first.to_owned()), Some(rest.to_owned())),
        Some(None) => (name.clone(), None),
        None => (None, None),
    };
    Profile {
        username: username.to_owned(),
        display_name: name,
        first_name,
        last_name,
        email: None,
        student_number: None,
        program: None,
    }
}

/// Texte du premier `<div class="… field--name-<champ> …">` qui ne contient que du texte. Une
/// page n'en a qu'un : au-delà de quelques-uns, on ne cherche plus.
fn field_text(html: &str, field: &str) -> Option<String> {
    let class = format!("field--name-{field}");
    tags(html, "div")
        .iter()
        .filter(|div| div.has_class(&class))
        .take(8)
        .filter_map(|div| inner(html, div, "div"))
        .filter(|content| !content.contains('<'))
        .map(text)
        .find(|name| !name.is_empty())
}

/// Outils de la page du catalogue : dans le bloc de la vue, un `<h3>` par rubrique, puis un
/// `<div about="<item-path><identifiant>">` par outil. Son premier lien est l'outil (nom dans le
/// lien, description dans son `title`) ; un second, facultatif, mène à sa documentation et n'est
/// pas repris. Les liens relatifs partent de la racine de l'hôte du portail.
///
/// Un outil rangé dans deux rubriques n'est rendu qu'une fois, dans la première. Une page sans
/// le bloc de la vue, ou dont aucun outil n'est lisible, n'est plus celle attendue.
pub(crate) fn parse_catalogue(
    html: &str,
    catalogue: &DrupalCatalogue,
    portal: &Url,
) -> Result<Vec<App>, Problem> {
    let view = view_block(html, &catalogue.view)
        .ok_or_else(|| upstream(PROVIDER, "vue du catalogue absente de la page"))?;
    let origin = portal
        .join("/")
        .map_err(|_| upstream(PROVIDER, "adresse du portail illisible"))?;

    let headings = tags(view, "h3");
    let mut apps: Vec<App> = Vec::new();
    let mut listed = 0_usize;
    for (index, heading) in headings.iter().enumerate() {
        let end = headings
            .get(index + 1)
            .map_or(view.len(), |next| next.start);
        let Some(section) = view.get(heading.end..end) else {
            continue;
        };
        // Sans `</h3>`, pas de titre : la rubrique est ignorée, comme une rubrique sans nom.
        let Some(title) = section
            .to_ascii_lowercase()
            .find("</h3")
            .and_then(|close| section.get(..close))
            .map(text)
            .filter(|title| !title.is_empty())
        else {
            continue;
        };

        let items: Vec<_> = tags(section, "div")
            .into_iter()
            .filter_map(|div| Some((div, item_id(&div.attr("about")?, &catalogue.item_path)?)))
            .collect();
        for (at, (div, id)) in items.iter().enumerate() {
            listed += 1;
            let end = items
                .get(at + 1)
                .map_or(section.len(), |(next, _)| next.start);
            let Some(block) = section.get(div.end..end) else {
                continue;
            };
            let Some((name, description, url)) = tool(block, &origin) else {
                continue;
            };
            if apps.len() >= MAX_APPS {
                return Ok(apps);
            }
            if apps.iter().any(|app| app.id == *id) {
                continue;
            }
            apps.push(App {
                id: id.clone(),
                title: name,
                description,
                url: url.into(),
                icon_url: None,
                category: Some(title.clone()),
                favorite: false,
            });
        }
    }
    if apps.is_empty() && listed > 0 {
        return Err(upstream(PROVIDER, "aucun outil lisible dans le catalogue"));
    }
    Ok(apps)
}

/// Du bloc de la vue (`view-id-<vue>`) à la fin du contenu principal (`</main>`), ou de la page.
fn view_block<'a>(html: &'a str, view: &str) -> Option<&'a str> {
    let marker = format!("view-id-{view}");
    let start = html.match_indices(&marker).find_map(|(at, _)| {
        // `view-id-outils`, pas `view-id-outils_archives`.
        let whole = html
            .as_bytes()
            .get(at + marker.len())
            .is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-'));
        whole.then_some(at)
    })?;
    let rest = html.get(start..)?;
    let end = rest.find("</main>").unwrap_or(rest.len());
    rest.get(..end)
}

/// Identifiant d'un outil : ce qui suit `item_path` dans son attribut `about`.
fn item_id(about: &str, item_path: &str) -> Option<String> {
    let head = about.get(..item_path.len())?;
    let id = about.get(item_path.len()..)?;
    (head.eq_ignore_ascii_case(item_path) && !id.is_empty()).then(|| id.to_owned())
}

/// Nom, description et adresse de l'outil d'un bloc : son premier lien.
fn tool(block: &str, origin: &Url) -> Option<(String, Option<String>, Url)> {
    let links = tags(block, "a");
    let link = links.first()?;
    let name = text(inner(block, link, "a")?);
    let url = origin
        .join(link.attr("href")?.trim())
        .ok()
        .filter(|url| matches!(url.scheme(), "https" | "http"))?;
    if name.is_empty() {
        return None;
    }
    // Un `title` qui répète le nom n'apprend rien.
    let description = link
        .attr("title")
        .map(|title| title.trim().to_owned())
        .filter(|title| !title.is_empty() && *title != name);
    Some((name, description, url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonapi_core::ErrorCode;

    const USER: &str = include_str!("../../tests/fixtures/drupal/utilisateur.html");
    const CATALOGUE: &str = include_str!("../../tests/fixtures/drupal/catalogue.html");

    fn portal() -> Url {
        Url::parse("https://portail.exemple.test/").unwrap()
    }

    fn outils() -> DrupalCatalogue {
        DrupalCatalogue {
            path: "tous-outils".to_owned(),
            view: "outils".to_owned(),
            item_path: "/outil/".to_owned(),
        }
    }

    #[test]
    fn session_ouverte_seulement_par_la_classe_du_body() {
        assert!(signed_in(USER));
        assert!(signed_in(CATALOGUE));
        assert!(signed_in(
            "<html><body class='path-frontpage user-logged-in'></body></html>"
        ));
        assert!(!signed_in(
            r#"<body class="path-frontpage user-anonymous">"#
        ));
        assert!(!signed_in(r#"<body class="not-user-logged-in-yet">"#));
        // La classe ne compte que sur le `<body>`.
        assert!(!signed_in(
            r#"<body class="login"><div class="user-logged-in"></div></body>"#
        ));
        assert!(!signed_in(""));
    }

    #[test]
    fn nom_complet_prenom_et_nom() {
        let profile = parse_profile(USER, Some("field-nomprenom"), "e00000000");
        assert_eq!(profile.username, "e00000000");
        assert_eq!(profile.display_name.as_deref(), Some("Camille Le Demo"));
        assert_eq!(profile.first_name.as_deref(), Some("Camille"));
        assert_eq!(profile.last_name.as_deref(), Some("Le Demo"));
        assert_eq!(profile.email, None);
    }

    #[test]
    fn entites_decodees_et_mot_seul() {
        let html = r#"<div class="field field--name-field-nomprenom field--item">Ana&#039;s</div>"#;
        let profile = parse_profile(html, Some("field-nomprenom"), "e00000000");
        assert_eq!(profile.display_name.as_deref(), Some("Ana's"));
        assert_eq!(profile.first_name.as_deref(), Some("Ana's"));
        assert_eq!(profile.last_name, None);
    }

    #[test]
    fn sans_champ_du_nom_le_profil_garde_l_identifiant() {
        for (html, field) in [
            (
                r#"<body class="user-logged-in"></body>"#,
                Some("field-nomprenom"),
            ),
            (
                r#"<div class="field--name-field-nomprenom"> </div>"#,
                Some("field-nomprenom"),
            ),
            // Un autre champ que celui de la configuration.
            (
                r#"<div class="field--name-field-autre">Camille Demo</div>"#,
                Some("field-nomprenom"),
            ),
            (USER, None),
        ] {
            let profile = parse_profile(html, field, "e00000000");
            assert_eq!(profile.username, "e00000000");
            assert_eq!(profile.display_name, None, "{html}");
            assert_eq!(profile.first_name, None);
        }
    }

    #[test]
    fn rubriques_outils_et_liens_de_documentation_ecartes() {
        let apps = parse_catalogue(CATALOGUE, &outils(), &portal()).unwrap();
        let read: Vec<_> = apps
            .iter()
            .map(|app| {
                (
                    app.id.as_str(),
                    app.title.as_str(),
                    app.description.as_deref(),
                    app.url.as_str(),
                    app.category.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            read,
            [
                (
                    "planning",
                    "PLANNING",
                    Some("Emplois du temps"),
                    "https://planning.exemple.test/",
                    Some("Bureau")
                ),
                (
                    "imprimer",
                    "Imprimer",
                    None,
                    "https://portail.exemple.test/numerique/impression#accordion-1",
                    Some("Bureau")
                ),
                (
                    "disques",
                    "Mes disques \"M\"",
                    Some("Recharger mon cr'dit"),
                    "https://fichiers.exemple.test/login.php?url=x&app=gollem",
                    Some("Ressources")
                ),
            ]
        );
        assert!(
            apps.iter()
                .all(|app| !app.favorite && app.icon_url.is_none())
        );
    }

    #[test]
    fn page_sans_la_vue_ou_sans_outil_lisible() {
        let no_view = parse_catalogue(
            r#"<body class="user-logged-in"><h3>Accueil</h3></body>"#,
            &outils(),
            &portal(),
        );
        assert_eq!(no_view.unwrap_err().code, ErrorCode::UpstreamChanged);

        // La vue sans aucun outil : une liste vide, pas une erreur.
        let empty = r#"<div class="view view-id-outils"><h3>Bureau</h3><ul></ul></div>"#;
        assert!(
            parse_catalogue(empty, &outils(), &portal())
                .unwrap()
                .is_empty()
        );

        // Des outils dont aucun n'a de lien : la page a changé.
        let unreadable = r#"<div class="view-id-outils"><h3>Bureau</h3>
            <div about="/outil/a"><span>Sans lien</span></div></div>"#;
        assert_eq!(
            parse_catalogue(unreadable, &outils(), &portal())
                .unwrap_err()
                .code,
            ErrorCode::UpstreamChanged
        );
    }

    #[test]
    fn autre_vue_autre_chemin_et_doublon() {
        let html = r#"<div class="view-id-services_archives"><h3>Archives</h3>
            <div about="/service/vieux"><a href="/vieux">Vieux</a></div></div>
          <div class="view-id-services"><h3>Un</h3>
            <div about="/service/a"><a href="/a" title="A">A</a></div>
            <div about="/outil/b"><a href="/b">B</a></div>
            <h3>Deux</h3>
            <div about="/service/a"><a href="/a">A</a></div>
            <div about="/service/c"><a href="/c">C</a></div></div>"#;
        let catalogue = DrupalCatalogue {
            path: "services".to_owned(),
            view: "services".to_owned(),
            item_path: "/service/".to_owned(),
        };
        let apps = parse_catalogue(html, &catalogue, &portal()).unwrap();
        let read: Vec<_> = apps
            .iter()
            .map(|app| (app.id.as_str(), app.category.as_deref()))
            .collect();
        assert_eq!(read, [("a", Some("Un")), ("c", Some("Deux"))]);
        // `title` égal au nom : pas de description.
        assert_eq!(apps.first().unwrap().description, None);
    }
}
