//! Arbre des ressources de MyPlanning : format texte d'ADE transporté dans une chaîne GWT
//! (`{` ouvre un nœud, `[n]` annonce n éléments, `"…"` une valeur).

use std::collections::HashSet;

use tonapi_core::Problem;

use super::bad_tree;

/// Racine de l'arbre des ressources, telle que MyPlanning la demande.
pub(super) const TREE_ROOT: &str = r#"{"-100""true""-1""-1""-1""-1""0""false"[0]"""""0""0"[0][0]"#;

/// Élément du format d'arbre : `{` ouvre un nœud, `[n]` annonce n éléments, `"…"` une valeur.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Token<'a> {
    Open,
    Count(usize),
    Text(&'a str),
}

/// Découpe du format ; chaque élément garde sa position de début et de fin.
fn tokens(text: &str) -> Option<Vec<(Token<'_>, usize, usize)>> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&b) = text.as_bytes().get(i) {
        let (token, end) = match b {
            b'{' => (Token::Open, i + 1),
            b'[' => {
                let close = i + 1 + text.get(i + 1..)?.find(']')?;
                (
                    Token::Count(text.get(i + 1..close)?.parse().ok()?),
                    close + 1,
                )
            }
            b'"' => {
                let close = i + 1 + text.get(i + 1..)?.find('"')?;
                (Token::Text(text.get(i + 1..close)?), close + 1)
            }
            _ => return None,
        };
        out.push((token, i, end));
        i = end;
    }
    Some(out)
}

/// Dossiers de la réponse de `searchDataById` : tous les nœuds marqués `"true"`.
pub(super) fn path_folders(text: &str) -> Result<HashSet<i64>, Problem> {
    let tokens = tokens(text).ok_or_else(bad_tree)?;
    let mut out = HashSet::new();
    for window in tokens.windows(3) {
        if let [
            (Token::Open, ..),
            (Token::Text(id), ..),
            (Token::Text("true"), ..),
        ] = window
        {
            out.insert(id.parse().map_err(|_| bad_tree())?);
        }
    }
    if out.is_empty() {
        return Err(bad_tree());
    }
    Ok(out)
}

/// Enfant d'un dossier : numéro, dossier ou ressource, nom et texte du nœud.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TreeNode {
    pub(super) id: i64,
    pub(super) folder: bool,
    pub(super) name: String,
    pub(super) raw: String,
}

/// Lecteur des nœuds de `getChildren` :
/// `{` 8 valeurs, `[n]` n champs (`{` 6 valeurs), 4 valeurs, `[0][0]`.
struct Reader<'a> {
    text: &'a str,
    tokens: Vec<(Token<'a>, usize, usize)>,
    at: usize,
}

impl<'a> Reader<'a> {
    fn next(&mut self) -> Option<(Token<'a>, usize, usize)> {
        let token = *self.tokens.get(self.at)?;
        self.at += 1;
        Some(token)
    }

    fn open(&mut self) -> Option<usize> {
        match self.next()? {
            (Token::Open, start, _) => Some(start),
            _ => None,
        }
    }

    fn count(&mut self) -> Option<(usize, usize)> {
        match self.next()? {
            (Token::Count(n), _, end) => Some((n, end)),
            _ => None,
        }
    }

    fn texts(&mut self, n: usize) -> Option<Vec<&'a str>> {
        (0..n)
            .map(|_| match self.next()? {
                (Token::Text(t), ..) => Some(t),
                _ => None,
            })
            .collect()
    }

    fn node(&mut self) -> Option<TreeNode> {
        let start = self.open()?;
        let head = self.texts(8)?;
        let mut name = None;
        for _ in 0..self.count()?.0 {
            self.open()?;
            let field = self.texts(6)?;
            if field.get(1) == Some(&"NAME") {
                name = field.get(3).map(|s| (*s).to_owned());
            }
        }
        self.texts(4)?;
        // Enfants déjà chargés : MyPlanning n'en renvoie pas, on refuse plutôt que deviner.
        let (first, _) = self.count()?;
        let (second, end) = self.count()?;
        if first != 0 || second != 0 {
            return None;
        }
        Some(TreeNode {
            id: head.first()?.parse().ok()?,
            folder: *head.get(1)? == "true",
            name: name.unwrap_or_default(),
            raw: self.text.get(start..end)?.to_owned(),
        })
    }
}

/// Réponse de `getChildren` : `{"0"`, le dossier ouvert, `[n]`, ses n enfants, puis des valeurs
/// finales (`"false"` à Paul-Valéry [vu] 27/10/2026, sens [inconnu]) qui sont ignorées.
pub(super) fn children(text: &str) -> Result<Vec<TreeNode>, Problem> {
    let parse = || {
        let mut reader = Reader {
            text,
            tokens: tokens(text)?,
            at: 0,
        };
        reader.open()?;
        reader.texts(1)?;
        reader.node()?;
        let (n, _) = reader.count()?;
        let nodes = (0..n).map(|_| reader.node()).collect::<Option<Vec<_>>>()?;
        let rest = reader.tokens.get(reader.at..)?;
        rest.iter()
            .all(|(token, ..)| matches!(token, Token::Text(_)))
            .then_some(nodes)
    };
    parse().ok_or_else(bad_tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arbre inventé : catégorie -1, dossier 10, ressources 41 et 42.
    const SEARCH: &str = r#"{"-100""true""0""-1""-1""-1""0""false"[0]""[0][1]{"-1""true""0""-1""-1""-1""0""false"[0]""[0][1]{"10""true""0""-1""-1""-1""0""false"[0]""[2]{"41""false""0""-1""-1""-1""0""false"[0]""[0]{"42""false""0""-1""-1""-1""0""false"[0]""[0][0]"#;
    const CHILDREN: &str = r#"{"0"{"10""true""1""-1""0""0""0""false"[1]{"StringField""NAME""LabelName""L1 Démo""false""false""L1 Démo""trainee""1""0"[0][0][2]{"41""false""2""-1""0""0""0""false"[2]{"ColorField""COLOR""LabelColor""255,0,0""false""false"{"StringField""NAME""LabelName""Groupe A""false""false""L1 Démo.Groupe A""trainee""1""0"[0][0]{"42""false""2""-1""1""1""0""false"[2]{"ColorField""COLOR""LabelColor""0,0,255""false""false"{"StringField""NAME""LabelName""Groupe B""false""false""L1 Démo.Groupe B""trainee""1""0"[0][0]"false""#;

    #[test]
    fn dossiers_du_chemin() {
        let folders = path_folders(SEARCH).unwrap();
        assert_eq!(folders, HashSet::from([-100, -1, 10]));
        assert!(path_folders("{\"1\"").is_err());
    }

    #[test]
    fn enfants_d_un_dossier() {
        let nodes = children(CHILDREN).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!((nodes[0].id, nodes[0].folder), (41, false));
        assert_eq!(nodes[1].name, "Groupe B");
        // Le nœud est recopié tel quel pour être rouvert.
        assert!(nodes[0].raw.starts_with(r#"{"41""false""2""#));
        assert!(nodes[0].raw.ends_with(r#""trainee""1""0"[0][0]"#));
        assert!(CHILDREN.contains(&nodes[1].raw));
        // Racine demandée par MyPlanning : un nœud sans champ.
        let root = format!(r#"{{"0"{TREE_ROOT}[0]"false""#);
        assert_eq!(children(&root).unwrap(), []);
        // Après les enfants, seules des valeurs sont admises.
        assert!(children(&format!(r#"{{"0"{TREE_ROOT}[0][0]"#)).is_err());
        assert!(children(&CHILDREN[..CHILDREN.len() - 3]).is_err());
    }
}
