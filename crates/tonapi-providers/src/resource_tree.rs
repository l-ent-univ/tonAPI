//! Ce qu'on tire d'un arbre de ressources d'emploi du temps une fois lu : l'arbre d'une nature,
//! la liste des ressources et leur recherche, les groupes de l'étudiant. Le logiciel d'origine
//! n'y change rien : chaque fournisseur lit son format, puis passe par ici.

use std::collections::HashSet;

use tonapi_core::{ResourceKind, ResourceNode, TimetableGroup, TimetableResource};

use crate::ResourceQuery;
use crate::common::fold;

/// L'arbre réduit aux natures voulues ; sans nature demandée, l'arbre entier.
pub(crate) fn only_kinds(mut tree: Vec<ResourceNode>, kinds: &[ResourceKind]) -> Vec<ResourceNode> {
    if !kinds.is_empty() {
        tree.retain(|node| kinds.contains(&node.kind));
    }
    tree
}

/// Marque `selected` les groupes que l'étudiant suit (`followed`, des identifiants), à toute
/// profondeur : un dossier suivi l'est comme une feuille.
pub(crate) fn mark_followed(tree: &mut [ResourceNode], followed: &HashSet<String>) {
    for node in tree {
        node.selected = node.kind == ResourceKind::Group && followed.contains(&node.id);
        mark_followed(&mut node.children, followed);
    }
}

/// Parcourt l'arbre en profondeur et appelle `visit` pour chaque nœud, avec son chemin (son nom
/// compris). `visit` rend `false` pour arrêter le parcours.
fn walk<'a>(
    nodes: &'a [ResourceNode],
    path: &mut Vec<&'a str>,
    visit: &mut impl FnMut(&'a ResourceNode, &[&'a str]) -> bool,
) -> bool {
    for node in nodes {
        path.push(&node.name);
        let go_on = visit(node, path) && walk(&node.children, path, visit);
        path.pop();
        if !go_on {
            return false;
        }
    }
    true
}

fn owned(path: &[&str]) -> Vec<String> {
    path.iter().map(|name| (*name).to_owned()).collect()
}

/// Groupes d'étudiants de l'arbre, avec leur chemin. `selected` (des identifiants) marque ceux
/// que l'étudiant suit. Un dossier n'apparaît que si l'étudiant le suit.
pub(crate) fn groups(tree: &[ResourceNode], selected: &HashSet<String>) -> Vec<TimetableGroup> {
    let mut groups = Vec::new();
    walk(tree, &mut Vec::new(), &mut |node, path| {
        let is_selected = selected.contains(&node.id);
        if node.kind == ResourceKind::Group && (!node.folder || is_selected) {
            groups.push(TimetableGroup {
                id: node.id.clone(),
                name: node.name.clone(),
                path: owned(path),
                selected: is_selected,
            });
        }
        true
    });
    groups
}

/// Ressources de l'arbre (tout nœud qui n'est pas un dossier) qui répondent à la recherche, dans
/// l'ordre de l'arbre. Sans tenir compte de la casse ni des accents, chaque mot cherché doit se
/// trouver dans le nom de la ressource ou d'un de ses dossiers, et l'un d'eux au moins dans son
/// nom : « amphi lannion » trouve les amphithéâtres rangés sous Lannion, « lannion » seul ne rend
/// pas tout ce que ce dossier contient.
pub(crate) fn resources(tree: &[ResourceNode], query: &ResourceQuery) -> Vec<TimetableResource> {
    let words: Vec<String> = fold(query.search.as_deref().unwrap_or_default())
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let limit = query.limit.unwrap_or(usize::MAX);
    let mut found = Vec::new();
    walk(tree, &mut Vec::new(), &mut |node, path| {
        if found.len() >= limit {
            return false;
        }
        let wanted = !node.folder && (query.kinds.is_empty() || query.kinds.contains(&node.kind));
        if wanted && matches(&words, &node.name, path) {
            found.push(TimetableResource {
                id: node.id.clone(),
                name: node.name.clone(),
                kind: node.kind,
                path: owned(path),
            });
        }
        true
    });
    found
}

/// Les mots cherchés (déjà ramenés à leur forme comparable) désignent-ils cette ressource ?
fn matches(words: &[String], name: &str, path: &[&str]) -> bool {
    if words.is_empty() {
        return true;
    }
    let name = fold(name);
    if !words.iter().any(|word| name.contains(word.as_str())) {
        return false;
    }
    let whole = fold(&path.join(" "));
    words.iter().all(|word| whole.contains(word.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: &str, name: &str, kind: ResourceKind) -> ResourceNode {
        ResourceNode {
            id: id.to_owned(),
            name: name.to_owned(),
            kind,
            folder: false,
            children: Vec::new(),
            selected: false,
        }
    }

    fn folder(id: &str, name: &str, children: Vec<ResourceNode>) -> ResourceNode {
        ResourceNode {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: children.first().map_or(ResourceKind::Other, |c| c.kind),
            folder: true,
            children,
            selected: false,
        }
    }

    /// Arbre inventé : deux groupes, deux enseignants, deux salles.
    fn tree() -> Vec<ResourceNode> {
        use ResourceKind::{Group, Room, Teacher};
        vec![
            folder(
                "1",
                "BUT Démo",
                vec![folder(
                    "2",
                    "Année 2",
                    vec![leaf("10", "TD 1", Group), leaf("11", "TD 2", Group)],
                )],
            ),
            folder(
                "3",
                "Département Démo",
                vec![
                    leaf("20", "DÉMO Camille", Teacher),
                    leaf("21", "EXEMPLE Zoé", Teacher),
                ],
            ),
            folder(
                "4",
                "Bâtiment A",
                vec![
                    leaf("30", "A001 - Amphithéâtre (120P)", Room),
                    leaf("31", "A104 - Salle de TD", Room),
                ],
            ),
        ]
    }

    fn query(kinds: &[ResourceKind], search: Option<&str>, limit: Option<usize>) -> ResourceQuery {
        ResourceQuery {
            kinds: kinds.to_vec(),
            search: search.map(str::to_owned),
            limit,
        }
    }

    fn ids(found: &[TimetableResource]) -> Vec<&str> {
        found.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn toutes_les_ressources_sans_les_dossiers() {
        let found = resources(&tree(), &ResourceQuery::default());
        assert_eq!(ids(&found), ["10", "11", "20", "21", "30", "31"]);
        assert_eq!(found[0].path, ["BUT Démo", "Année 2", "TD 1"]);
        assert_eq!(found[4].kind, ResourceKind::Room);
    }

    #[test]
    fn recherche_par_nature_et_par_mots() {
        let tree = tree();
        let rooms = resources(&tree, &query(&[ResourceKind::Room], None, None));
        assert_eq!(ids(&rooms), ["30", "31"]);
        // Sans casse ni accent, les mots dans n'importe quel ordre.
        let found = resources(&tree, &query(&[], Some("amphitheatre a001"), None));
        assert_eq!(ids(&found), ["30"]);
        let found = resources(&tree, &query(&[], Some("zoe"), None));
        assert_eq!(ids(&found), ["21"]);
        // Un mot peut désigner un dossier, pas tous : le nom de la ressource en porte un.
        let found = resources(&tree, &query(&[], Some("salle batiment"), None));
        assert_eq!(ids(&found), ["31"]);
        assert!(resources(&tree, &query(&[], Some("bâtiment"), None)).is_empty());
        assert!(resources(&tree, &query(&[], Some("amphi démo"), None)).is_empty());
        // Une nature écarte les autres.
        assert!(resources(&tree, &query(&[ResourceKind::Group], Some("salle"), None)).is_empty());
        // Une recherche de blancs seulement ne filtre rien.
        assert_eq!(resources(&tree, &query(&[], Some("  "), None)).len(), 6);
    }

    #[test]
    fn limite_respectee_dans_l_ordre_de_l_arbre() {
        let found = resources(&tree(), &query(&[], None, Some(3)));
        assert_eq!(ids(&found), ["10", "11", "20"]);
        assert!(resources(&tree(), &query(&[], None, Some(0))).is_empty());
    }

    #[test]
    fn groupes_suivis_marques_dans_l_arbre() {
        let mut tree = tree();
        let followed = HashSet::from(["2".to_owned(), "11".to_owned(), "30".to_owned()]);
        mark_followed(&mut tree, &followed);
        let year = &tree[0].children[0];
        assert!(year.selected && year.folder);
        assert_eq!(
            (year.children[0].selected, year.children[1].selected),
            (false, true)
        );
        // Une salle n'est pas un groupe : jamais marquée, même si son id est donné.
        assert!(!tree[2].children[0].selected);
        // Marquer de nouveau efface ce qui ne l'est plus.
        mark_followed(&mut tree, &HashSet::new());
        assert!(!tree[0].children[0].selected);
    }

    #[test]
    fn arbre_d_une_nature() {
        let rooms = only_kinds(tree(), &[ResourceKind::Room]);
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].name, "Bâtiment A");
        assert_eq!(only_kinds(tree(), &[]).len(), 3);
    }

    #[test]
    fn groupes_et_dossier_suivi() {
        let tree = tree();
        let selected = HashSet::from(["11".to_owned(), "2".to_owned(), "30".to_owned()]);
        let found: Vec<_> = groups(&tree, &selected)
            .into_iter()
            .map(|g| (g.id, g.selected))
            .collect();
        // Le dossier suivi apparaît ; une salle suivie n'est pas un groupe.
        assert_eq!(
            found,
            [
                ("2".to_owned(), true),
                ("10".to_owned(), false),
                ("11".to_owned(), true)
            ]
        );
    }
}
