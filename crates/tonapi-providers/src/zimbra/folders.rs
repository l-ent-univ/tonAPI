//! Dossiers de la boîte, lus dans la `GetFolderResponse` de Zimbra. Repris de
//! `server/mailFolders.js` de Tafac, pour `/mail/folders` (les pastilles et le menu « Dossiers »
//! de la page) et pour vérifier qu'un déplacement vise un dossier listé.
//!
//! Écart voulu (W10) : les noms perdent leurs caractères de contrôle et de direction avant d'être
//! coupés à 128 unités UTF-16.

use std::collections::HashSet;

use serde_json::{Value, json};
use unicode_normalization::UnicodeNormalization;

use crate::mail::text::{cut_utf16, is_control, is_direction, is_mark, trim_js};

use super::listing::{as_array, id_text, js_string, positive_count};

/// Boîte de réception (même identifiant dans toute boîte Zimbra).
pub const INBOX_FOLDER: &str = "2";
/// Corbeille.
pub const TRASH_FOLDER: &str = "3";
/// Indésirables.
pub const JUNK_FOLDER: &str = "4";
/// Envoyés.
pub const SENT_FOLDER: &str = "5";
/// Brouillons.
pub const DRAFTS_FOLDER: &str = "6";

/// Dossiers système, dans l'ordre où la page les montre, avec leur nature.
pub const SYSTEM_FOLDERS: [(&str, &str); 5] = [
    (INBOX_FOLDER, "inbox"),
    (SENT_FOLDER, "sent"),
    (DRAFTS_FOLDER, "drafts"),
    (JUNK_FOLDER, "junk"),
    (TRASH_FOLDER, "trash"),
];

/// Discussions (14) : les anciens messages instantanés, pas du mail.
const HIDDEN_FOLDER: &str = "14";
/// En dessous, les identifiants sont ceux de Zimbra (contacts, agenda, étiquettes…) ; ceux que
/// l'étudiant crée commencent ici.
const FIRST_USER_FOLDER_ID: u32 = 256;
/// Profondeur maximale de l'arbre (0 en haut).
pub const MAX_FOLDER_DEPTH: usize = 12;
/// Dossiers listés au plus.
pub const MAX_FOLDER_COUNT: usize = 400;
/// Longueur maximale d'un nom, en unités UTF-16.
pub const MAX_FOLDER_NAME_LENGTH: usize = 128;

/// Identifiant de dossier : `^[0-9]{1,9}$` (chiffres ASCII seulement).
#[must_use]
pub fn is_valid_folder_id(value: &str) -> bool {
    (1..=9).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit())
}

/// Les seuls dossiers qu'on peut vider pour de bon : la corbeille et les indésirables.
#[must_use]
pub fn is_emptiable_folder(id: &str) -> bool {
    id == TRASH_FOLDER || id == JUNK_FOLDER
}

/// Un dossier de la liste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailFolder {
    /// Identifiant (`^[0-9]{1,9}$`).
    pub id: String,
    /// Nature : celle d'un dossier système (`inbox`, `sent`, `drafts`, `junk`, `trash`) ou
    /// `folder`.
    pub kind: &'static str,
    /// Nom, nettoyé et coupé.
    pub name: String,
    /// Non lus.
    pub unread: u64,
    /// Messages.
    pub count: u64,
    /// Dossier parent, `None` en haut.
    pub parent_id: Option<String>,
    /// Noms depuis le haut, joints par « / » (« Inbox/Moodle »).
    pub path: String,
    /// Profondeur, 0 en haut.
    pub depth: usize,
}

impl MailFolder {
    /// Le dossier tel que la page le lit : `{id, kind, name, unread, count, parentId, path,
    /// depth}`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind,
            "name": self.name,
            "unread": self.unread,
            "count": self.count,
            "parentId": self.parent_id,
            "path": self.path,
            "depth": self.depth,
        })
    }
}

/// Le corps de `GetFolderRequest` (urn:zimbraMail) qui lit tout l'arbre : la racine
/// (`USER_ROOT`, id 1) et tout ce qu'elle contient.
#[must_use]
pub fn folder_tree_request() -> Value {
    json!({ "folder": { "l": "1" } })
}

/// La racine d'une `GetFolderResponse` : `folder[0]`, ou `folder` quand Zimbra n'en donne qu'un.
#[must_use]
pub fn folder_tree_root(response: &Value) -> Option<&Value> {
    match response.get("folder")? {
        Value::Array(list) => list.first(),
        other => Some(other),
    }
}

/// Un dossier qui contient du mail : sans vue (celle par défaut de Zimbra), ou la vue message ou
/// conversation.
fn is_mail_view(folder: &Value) -> bool {
    match folder.get("view") {
        None | Some(Value::Null) => true,
        Some(Value::String(view)) => view.is_empty() || view == "message" || view == "conversation",
        Some(Value::Bool(b)) => !b,
        Some(Value::Number(n)) => n.as_f64() == Some(0.0),
        Some(_) => false,
    }
}

/// Nom d'un dossier : une chaîne, sans caractère de contrôle ni de direction, sans espaces de
/// bord, 128 unités au plus.
fn folder_name(folder: &Value) -> String {
    let Some(name) = folder.get("name").and_then(Value::as_str) else {
        return String::new();
    };
    // Un nom très long est coupé avant le travail : seules 128 unités restent.
    let kept: String = cut_utf16(name, MAX_FOLDER_NAME_LENGTH * 8)
        .chars()
        .filter(|&c| !is_control(c) && !is_direction(c))
        .collect();
    trim_js(cut_utf16(trim_js(&kept), MAX_FOLDER_NAME_LENGTH)).to_owned()
}

/// Clé de tri d'un nom, proche de `localeCompare(…, 'fr', {sensitivity: 'base'})` : sans casse ni
/// accents, quelques ligatures développées. L'ordre de la ponctuation reste celui des points de
/// code (ICU en a un autre) : seul l'ordre d'affichage en dépend.
fn collation_key(name: &str) -> String {
    let mut key = String::with_capacity(name.len());
    for c in name.nfd().filter(|&c| !is_mark(c)) {
        match c {
            'œ' | 'Œ' => key.push_str("oe"),
            'æ' | 'Æ' => key.push_str("ae"),
            'ß' => key.push_str("ss"),
            _ => key.extend(c.to_lowercase()),
        }
    }
    key
}

/// Les éléments d'une liste de dossiers triés par nom (ordre stable à égalité).
fn sorted_by_name(folders: Vec<&Value>) -> Vec<&Value> {
    let mut keyed: Vec<(String, &Value)> = folders
        .into_iter()
        .map(|folder| {
            let name = folder.get("name").and_then(js_string).unwrap_or_default();
            (collation_key(&name), folder)
        })
        .collect();
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    keyed.into_iter().map(|(_, folder)| folder).collect()
}

struct Walk {
    result: Vec<MailFolder>,
    seen: HashSet<String>,
}

impl Walk {
    fn visit(&mut self, folder: &Value, parent: Option<(String, String)>, depth: usize) {
        if self.result.len() >= MAX_FOLDER_COUNT || depth > MAX_FOLDER_DEPTH {
            return;
        }
        let id = id_text(folder.get("id"));
        if !is_valid_folder_id(&id)
            || self.seen.contains(&id)
            || id == HIDDEN_FOLDER
            || !is_mail_view(folder)
        {
            return;
        }
        // En haut, seulement les dossiers système connus et ceux de l'étudiant ; plus bas,
        // seulement ceux de l'étudiant (Zimbra garde les siens en haut).
        let kind = if depth == 0 {
            SYSTEM_FOLDERS
                .iter()
                .find(|(system, _)| *system == id)
                .map(|(_, kind)| *kind)
        } else {
            None
        };
        let number: u32 = id.parse().unwrap_or(0);
        if kind.is_none() && number < FIRST_USER_FOLDER_ID {
            return;
        }
        self.seen.insert(id.clone());
        let name = folder_name(folder);
        let path = match &parent {
            Some((_, parent_path)) => format!("{parent_path}/{name}"),
            None => name.clone(),
        };
        self.result.push(MailFolder {
            id: id.clone(),
            kind: kind.unwrap_or("folder"),
            name,
            unread: positive_count(folder.get("u")),
            count: positive_count(folder.get("n")),
            parent_id: parent.map(|(parent_id, _)| parent_id),
            path: path.clone(),
            depth,
        });
        // Sous-dossiers par nom, comme le webmail les liste.
        for child in sorted_by_name(as_array(folder.get("folder"))) {
            self.visit(child, Some((id.clone(), path.clone())), depth + 1);
        }
    }
}

/// `normalizeZimbraFolderTree` : l'arbre sous la racine ([`folder_tree_root`]) en liste plate,
/// parents avant enfants. Les dossiers système d'abord, dans l'ordre de [`SYSTEM_FOLDERS`],
/// chacun suivi de ses sous-dossiers, puis ceux de l'étudiant par nom avec les leurs. Ce qui n'est
/// pas un dossier de mail (contacts, agenda, dossier partagé, dossier de recherche, Discussions)
/// est laissé de côté avec tout ce qu'il contient. 12 niveaux et 400 dossiers au plus ; un
/// identifiant déjà vu n'est pas repris (pas de boucle).
#[must_use]
pub fn normalize_folder_tree(root: Option<&Value>) -> Vec<MailFolder> {
    let top_level: Vec<&Value> = as_array(root.and_then(|root| root.get("folder")))
        .into_iter()
        .filter(|folder| is_valid_folder_id(&id_text(folder.get("id"))))
        .collect();
    let mut walk = Walk {
        result: Vec::new(),
        seen: HashSet::new(),
    };
    for (system, _) in SYSTEM_FOLDERS {
        if let Some(folder) = top_level
            .iter()
            .find(|folder| id_text(folder.get("id")) == system)
        {
            walk.visit(folder, None, 0);
        }
    }
    let custom: Vec<&Value> = top_level
        .iter()
        .copied()
        .filter(|folder| {
            let id = id_text(folder.get("id"));
            !SYSTEM_FOLDERS.iter().any(|(system, _)| *system == id)
        })
        .collect();
    for folder in sorted_by_name(custom) {
        walk.visit(folder, None, 0);
    }
    walk.result
}

/// Le dossier `id` est-il dans la liste ? (Un déplacement ou une action sur un dossier n'en vise
/// jamais un autre.)
#[must_use]
pub fn is_listed_folder(folders: &[MailFolder], id: &str) -> bool {
    folders.iter().any(|folder| folder.id == id)
}

/// La réponse de `/mail/folders` : `{folders: [...]}`.
#[must_use]
pub fn folders_answer(folders: &[MailFolder]) -> Value {
    json!({ "folders": folders.iter().map(MailFolder::to_json).collect::<Vec<_>>() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> Value {
        json!({
            "id": "1",
            "name": "USER_ROOT",
            "folder": [
                { "id": "3", "name": "Trash", "u": 0, "n": 4, "folder": [{ "id": "400", "name": "Vieux dossier", "n": 2 }] },
                { "id": "2", "name": "Inbox", "u": 5, "n": 120, "folder": [
                    { "id": "300", "name": "Moodle", "u": 2, "n": 40, "folder": [{ "id": "301", "name": "L2", "u": 1, "n": 3 }] },
                    { "id": "302", "name": "Admin", "n": 7 },
                ] },
                { "id": "7", "name": "Contacts", "view": "contact" },
                { "id": "10", "name": "Calendar", "view": "appointment" },
                { "id": "14", "name": "Chats", "view": "message" },
                { "id": "16", "name": "Briefcase", "view": "document" },
                { "id": "5", "name": "Sent", "n": 30 },
                { "id": "6", "name": "Drafts" },
                { "id": "4", "name": "Junk", "u": "1", "n": "1" },
                { "id": "257", "name": "Archive", "n": 12, "folder": [{ "id": "258", "name": "2025", "n": 3 }] },
                { "id": "256", "name": "Associations", "u": 3 },
                { "id": "500", "name": "Agenda perso", "view": "appointment", "folder": [{ "id": "501", "name": "Sous-agenda" }] },
            ],
        })
    }

    fn ids(folders: &[MailFolder]) -> Vec<&str> {
        folders.iter().map(|folder| folder.id.as_str()).collect()
    }

    #[test]
    fn tout_l_arbre_systeme_d_abord() {
        let folders = normalize_folder_tree(Some(&root()));
        assert_eq!(
            ids(&folders),
            [
                "2", "302", "300", "301", "5", "6", "4", "3", "400", "257", "258", "256"
            ]
        );
    }

    #[test]
    fn nature_compteurs_parent_chemin_profondeur() {
        let folders = normalize_folder_tree(Some(&root()));
        let by_id = |id: &str| folders.iter().find(|folder| folder.id == id).unwrap();
        assert_eq!(
            by_id("2").to_json(),
            json!({ "id": "2", "kind": "inbox", "name": "Inbox", "unread": 5, "count": 120, "parentId": null, "path": "Inbox", "depth": 0 })
        );
        assert_eq!(
            by_id("301").to_json(),
            json!({ "id": "301", "kind": "folder", "name": "L2", "unread": 1, "count": 3, "parentId": "300", "path": "Inbox/Moodle/L2", "depth": 2 })
        );
        assert_eq!(by_id("4").kind, "junk");
        assert_eq!(by_id("4").unread, 1);
        assert_eq!(by_id("3").kind, "trash");
        assert_eq!(by_id("400").parent_id.as_deref(), Some("3"));
        assert_eq!(by_id("256").kind, "folder");
        assert_eq!(by_id("256").count, 0);
        assert_eq!(
            folders_answer(&folders)["folders"]
                .as_array()
                .unwrap()
                .len(),
            12
        );
    }

    #[test]
    fn discussions_et_autres_vues_laissees_de_cote() {
        let folders = normalize_folder_tree(Some(&root()));
        for hidden in ["1", "7", "10", "14", "16", "500", "501"] {
            assert!(!is_listed_folder(&folders, hidden), "{hidden} listé");
        }
    }

    #[test]
    fn reponses_bizarres() {
        assert!(normalize_folder_tree(None).is_empty());
        assert!(normalize_folder_tree(Some(&json!({}))).is_empty());
        assert!(normalize_folder_tree(Some(&json!("x"))).is_empty());
        assert_eq!(
            ids(&normalize_folder_tree(Some(
                &json!({ "folder": { "id": "2", "name": "Inbox" } })
            ))),
            ["2"]
        );
        let odd = normalize_folder_tree(Some(&json!({ "folder": [
            { "id": "abc", "name": "x" },
            { "id": "2", "name": "Inbox", "u": -3, "n": "many", "folder": [{ "id": "12", "name": "system-like" }, { "id": "2", "name": "loop" }] },
            { "id": "99", "name": "unknown system id" },
        ] })));
        assert_eq!(ids(&odd), ["2"]);
        assert_eq!(odd[0].unread, 0);
        assert_eq!(odd[0].count, 0);
        // Identifiant numérique accepté, chiffres Unicode et `__proto__` jamais.
        let tree = normalize_folder_tree(Some(&json!({ "folder": [
            { "id": 2, "name": "Inbox" },
            { "id": "٣٠٠", "name": "arabe" },
            { "id": "__proto__", "name": "proto" },
            { "id": "constructor", "name": "c" },
            { "id": 300.5, "name": "flottant" },
        ] })));
        assert_eq!(ids(&tree), ["2"]);
        // Les compteurs ne prennent pas de chiffres Unicode.
        let tree = normalize_folder_tree(Some(
            &json!({ "folder": [{ "id": "2", "u": "٣", "n": "1e2" }] }),
        ));
        assert_eq!((tree[0].unread, tree[0].count), (0, 100));
    }

    #[test]
    fn arbre_tres_profond_coupe() {
        let mut deepest = json!({ "id": "999", "name": "bottom" });
        for id in (900..=998).rev() {
            deepest =
                json!({ "id": id.to_string(), "name": format!("n{id}"), "folder": [deepest] });
        }
        let folders = normalize_folder_tree(Some(&json!({ "folder": [deepest] })));
        assert!(folders.len() <= 13);
        assert_eq!(
            folders.iter().map(|f| f.depth).max(),
            Some(MAX_FOLDER_DEPTH)
        );
    }

    #[test]
    fn arbre_tres_large_coupe() {
        let many: Vec<Value> = (1000..2000)
            .map(|id| json!({ "id": id.to_string(), "name": format!("d{id}") }))
            .collect();
        let folders = normalize_folder_tree(Some(&json!({ "folder": many })));
        assert_eq!(folders.len(), MAX_FOLDER_COUNT);
    }

    #[test]
    fn boucles_repetees_arretees() {
        // Le même identifiant partout : listé une fois.
        let mut nested = json!({ "id": "300", "name": "x" });
        for _ in 0..50 {
            nested = json!({ "id": "300", "name": "x", "folder": [{ "id": "300" }, nested] });
        }
        let folders = normalize_folder_tree(Some(&json!({ "folder": [nested] })));
        assert_eq!(ids(&folders), ["300"]);
    }

    #[test]
    fn noms_nettoyes_et_coupes() {
        let tree = normalize_folder_tree(Some(&json!({ "folder": [
            { "id": "256", "name": " Rapport\u{202e}fdp.exe\u{0}\n " },
            { "id": "257", "name": "é".repeat(300) },
            { "id": "258", "name": 42 },
        ] })));
        let by_id = |id: &str| tree.iter().find(|folder| folder.id == id).unwrap();
        assert_eq!(by_id("256").name, "Rapportfdp.exe");
        assert_eq!(by_id("257").name.chars().count(), MAX_FOLDER_NAME_LENGTH);
        assert_eq!(by_id("258").name, "");
    }

    #[test]
    fn tri_par_nom_sans_casse_ni_accents() {
        let tree = normalize_folder_tree(Some(&json!({ "folder": [
            { "id": "260", "name": "zèbre" },
            { "id": "261", "name": "Écoles" },
            { "id": "262", "name": "ecole" },
            { "id": "263", "name": "Archive" },
            { "id": "264", "name": "2025" },
        ] })));
        assert_eq!(ids(&tree), ["264", "263", "262", "261", "260"]);
    }

    #[test]
    fn identifiants_et_dossiers_a_vider() {
        assert!(is_valid_folder_id("2"));
        assert!(is_valid_folder_id("300"));
        for bad in ["", "-1", "1,2", "2 ", "inbox", "1234567890", "٢", "２"] {
            assert!(!is_valid_folder_id(bad), "{bad}");
        }
        assert!(is_emptiable_folder("3"));
        assert!(is_emptiable_folder("4"));
        for other in ["2", "5", "6", "300", ""] {
            assert!(!is_emptiable_folder(other), "{other}");
        }
        assert_eq!(folder_tree_request(), json!({ "folder": { "l": "1" } }));
        let response = json!({ "folder": [{ "id": "1" }] });
        assert_eq!(folder_tree_root(&response), Some(&json!({ "id": "1" })));
        assert_eq!(folder_tree_root(&json!({})), None);
    }
}
