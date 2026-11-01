//! Conventions propres à ADE dans les champs iCal : salles, enseignants, groupes, statut et type
//! de séance.

// Heuristiques tirées de `Docs/recherche/Montpellier/emploi-du-temps.md` §5 (code d'iCalFormater, observées
// à Montpellier) : [déduit] du seul flux de l'UM, à ne pas supposer valables ailleurs.

/// Au plus autant de salles, enseignants ou groupes par séance : le dédoublonnage reste rapide
/// même sur un flux hostile.
const MAX_PER_LIST: usize = 50;

/// Salles : `LOCATION` découpé sur `,` et `;`, sans doublon.
pub(super) fn split_rooms(location: &str) -> Vec<String> {
    let mut rooms: Vec<String> = Vec::new();
    for room in location.split([',', ';']).map(str::trim) {
        if rooms.len() >= MAX_PER_LIST {
            break;
        }
        if !room.is_empty() && !rooms.iter().any(|r| r == room) {
            rooms.push(room.to_owned());
        }
    }
    rooms
}

/// Contenu de `DESCRIPTION`, ligne par ligne. L'ordre des lignes n'est pas garanti : on les
/// classe par leur forme.
#[derive(Default)]
pub(super) struct Described {
    pub(super) teachers: Vec<String>,
    pub(super) groups: Vec<String>,
    /// Lignes non reconnues ; la première est le libellé du cours.
    pub(super) other: Vec<String>,
    pub(super) cancelled: bool,
}

enum Teacher {
    Name(String),
    /// Enseignant fictif d'ADE (`A DEFINIR`, `ZZ …`) : ni enseignant ni libellé.
    Placeholder,
    Unrelated,
}

pub(super) fn describe(description: &str) -> Described {
    let mut described = Described::default();
    for line in description.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.starts_with("(Exported") {
            continue;
        }
        if let Some(status) = status_of(line) {
            described.cancelled |= status.starts_with("annul");
            continue;
        }
        match teacher(line) {
            Teacher::Name(name) => push_unique(&mut described.teachers, name),
            Teacher::Placeholder => {}
            Teacher::Unrelated if looks_like_group(line) => {
                push_unique(&mut described.groups, collapse_spaces(line));
            }
            Teacher::Unrelated => described.other.push(collapse_spaces(line)),
        }
    }
    described
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if list.len() < MAX_PER_LIST && !list.contains(&value) {
        list.push(value);
    }
}

fn collapse_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Statut de la séance (`A valider`, `Confirmé`…), en minuscules.
fn status_of(line: &str) -> Option<String> {
    const STATUSES: [&str; 10] = [
        "a valider",
        "à valider",
        "validé",
        "confirmé",
        "annulé",
        "annule",
        "prévisionnel",
        "previsionnel",
        "réservé",
        "reserve",
    ];
    let lower = line.to_lowercase();
    STATUSES.contains(&lower.as_str()).then_some(lower)
}

/// Enseignant : `NOM   PRÉNOM`, tout en majuscules, séparés par au moins deux espaces, sans chiffre,
/// six mots au plus. Rendu `Prénom Nom`.
fn teacher(line: &str) -> Teacher {
    const PLACEHOLDERS: [&str; 4] = ["A DEFINIR", "NON AFFECT", "INCONNU", "A PRECISER"];
    let Some((surname, first)) = line.split_once("  ") else {
        return Teacher::Unrelated;
    };
    let (surname, first) = (collapse_spaces(surname), collapse_spaces(first));
    let shaped = !surname.is_empty()
        && !first.is_empty()
        && line.split_whitespace().count() <= 6
        && !line.chars().any(|c| c.is_ascii_digit())
        && line
            .chars()
            .filter(|c| c.is_alphabetic())
            .all(char::is_uppercase);
    if !shaped {
        return Teacher::Unrelated;
    }
    let upper = line.to_uppercase();
    if PLACEHOLDERS.iter().any(|p| upper.contains(p)) || upper.starts_with("ZZ") {
        return Teacher::Placeholder;
    }
    Teacher::Name(format!("{} {}", title_case(&first), title_case(&surname)))
}

fn title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut capitalize = true;
    for c in text.chars() {
        if c.is_alphabetic() {
            if capitalize {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            capitalize = false;
        } else {
            out.push(c);
            capitalize = matches!(c, ' ' | '-' | '\'');
        }
    }
    out
}

/// Groupe : `Gr A`, `GRF`, `L1`, `info`, `portail`, `série`, `TDB`, `TP1`, `année`.
fn looks_like_group(line: &str) -> bool {
    line.split_whitespace().any(|token| {
        let token = token
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        let chars: Vec<char> = token.chars().collect();
        match token.as_str() {
            "gr" | "groupe" | "portail" | "info" | "série" | "serie" | "année" | "annee" | "td"
            | "tp" => true,
            _ => {
                let short = |prefix: &str, len: std::ops::RangeInclusive<usize>| {
                    token.starts_with(prefix)
                        && len.contains(&chars.len())
                        && chars.iter().all(|c| c.is_alphanumeric())
                };
                short("gr", 3..=3) || short("td", 3..=4) || short("tp", 3..=4) || is_level(&chars)
            }
        }
    })
}

/// `L1`, `L2`, `L3` (niveaux de licence).
fn is_level(chars: &[char]) -> bool {
    matches!(chars, ['l', digit] if digit.is_ascii_digit())
}

/// Retire le préfixe `DDMMYYYY - ` des événements ponctuels.
pub(super) fn strip_date_prefix(summary: &str) -> &str {
    let digits = summary.chars().take_while(char::is_ascii_digit).count();
    if digits == 8
        && let Some(rest) = summary.get(8..)
        && let Some(rest) = rest.trim_start().strip_prefix('-')
    {
        return rest.trim_start();
    }
    summary
}

/// Type de séance : suffixe explicite (`…-CM`), examen, puis salles ; « Événement » sans code de
/// module ni libellé. `None` quand rien ne permet de conclure.
pub(super) fn session_kind(summary: &str, label: Option<&str>, rooms: &[String]) -> Option<String> {
    if let Some((_, suffix)) = summary.rsplit_once('-') {
        let suffix = suffix.trim().to_uppercase();
        if matches!(suffix.as_str(), "CM" | "TD" | "TP") {
            return Some(suffix);
        }
    }
    if is_exam(summary) || label.is_some_and(is_exam) {
        return Some("Examen".to_owned());
    }
    let tokens: Vec<String> = rooms
        .iter()
        .flat_map(|room| room.split(|c: char| !c.is_alphanumeric()))
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect();
    let has = |wanted: &dyn Fn(&str) -> bool| tokens.iter().any(|t| wanted(t.as_str()));
    if has(&|t| t.starts_with("amphi")) {
        return Some("CM".to_owned());
    }
    if has(&|t| matches!(t, "tdinfo" | "tpinfo" | "labo" | "tp")) || has_salle_info(&tokens) {
        return Some("TP".to_owned());
    }
    if has(&|t| t == "salle" || t == "banalisée" || is_td_token(t)) {
        return Some("TD".to_owned());
    }
    let has_code = summary.split_whitespace().any(is_module_code);
    (label.is_none() && !has_code).then(|| "Événement".to_owned())
}

fn has_salle_info(tokens: &[String]) -> bool {
    tokens
        .windows(2)
        .any(|pair| matches!(pair, [a, b] if a == "salle" && b == "info"))
}

/// `td`, `td4`…
fn is_td_token(token: &str) -> bool {
    token
        .strip_prefix("td")
        .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
}

fn is_exam(text: &str) -> bool {
    let tokens: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect();
    tokens
        .iter()
        .enumerate()
        .any(|(i, token)| match token.as_str() {
            "examen" | "partiel" | "ds" | "cc" | "contrôle" | "controle" | "épreuve"
            | "epreuve" | "rattrapage" | "soutenance" => true,
            "session" => tokens
                .get(i + 1)
                .is_some_and(|next| next.chars().all(|c| c.is_ascii_digit())),
            _ => false,
        })
}

/// Code de module : 2 à 4 lettres majuscules, 3 chiffres, une lettre facultative (`HAI102I`).
fn is_module_code(token: &str) -> bool {
    let chars: Vec<char> = token.chars().collect();
    let letters = chars.iter().take_while(|c| c.is_ascii_uppercase()).count();
    let digits = chars
        .iter()
        .skip(letters)
        .take_while(|c| c.is_ascii_digit())
        .count();
    let tail = chars.len() - letters - digits;
    (2..=4).contains(&letters)
        && digits == 3
        && (tail == 0 || (tail == 1 && chars.last().is_some_and(char::is_ascii_uppercase)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_de_seance() {
        let kind = |summary: &str, label: Option<&str>, rooms: &[&str]| {
            let rooms: Vec<String> = rooms.iter().map(|r| (*r).to_owned()).collect();
            session_kind(summary, label, &rooms)
        };
        assert_eq!(
            kind("SAE103 Design Grphk-CM", None, &[]).as_deref(),
            Some("CM")
        );
        assert_eq!(
            kind("HAI102I Maths", Some("Examen de mathématiques"), &["TD 4"]).as_deref(),
            Some("Examen")
        );
        assert_eq!(
            kind("HAI102I Maths", None, &["Amphi B"]).as_deref(),
            Some("CM")
        );
        assert_eq!(
            kind("HAI102I Maths", None, &["Salle info 12"]).as_deref(),
            Some("TP")
        );
        assert_eq!(
            kind("HAI102I Maths", None, &["TD 4"]).as_deref(),
            Some("TD")
        );
        assert_eq!(kind("HAI102I Maths", None, &["Bât 36"]), None);
        assert_eq!(kind("Réunion", None, &[]).as_deref(), Some("Événement"));
    }

    #[test]
    fn enseignants_et_groupes() {
        let d = describe(
            "\n\nINFO GR A - L1 Portail\nBases\nA valider\nDEMO OGER   CAMILLE\nZZ FDS   A DEFINIR\n\
             MMI  1ère année BUT\nTDB\n(Exported :26/08/2026 14:25)\n",
        );
        assert_eq!(d.teachers, ["Camille Demo Oger"]);
        assert_eq!(
            d.groups,
            ["INFO GR A - L1 Portail", "MMI 1ère année BUT", "TDB"]
        );
        assert_eq!(d.other, ["Bases"]);
        assert!(!d.cancelled);
        assert!(is_module_code("HAI102I") && is_module_code("SAE103") && !is_module_code("Info"));
    }
}
