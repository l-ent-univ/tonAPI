//! Recherche de la page de messagerie : les mots tapés, et la requête Zimbra qui les cherche.
//! Repris de `src/mailSearch.js` de Tafac (partagé là-bas par la page et le serveur).
//!
//! Comment Zimbra (RENATER Partage) indexe le texte :
//! - chaque caractère passe d'abord par `NormalizeTokenFilter`, qui met en minuscules et retire
//!   les accents ;
//! - un mot avec des apostrophes droites entre des lettres est un seul jeton (« l'examen ») ; une
//!   apostrophe typographique (’) n'en est pas une lettre : « l’examen » fait deux jetons ;
//! - sa grammaire de requête lit un terme comme tout sauf les espaces et `{ } : + - " ( )`, et un
//!   terme ne commence ni par `-` ni par `+`.
//!
//! Chaque mot part donc sous les formes qui peuvent trouver ces jetons, chacune suivie de `*`
//! (préfixe), et seuls lettres, chiffres et `@ . _ ' -` atteignent la requête. Le dossier est un
//! identifiant en chiffres ASCII : rien de tapé ne devient opérateur, champ ou phrase.

use unicode_normalization::UnicodeNormalization;

use crate::mail::text::{cut_utf16, is_letter, is_mark, is_number, nfc};

use super::folders::is_valid_folder_id;

/// Mots cherchés au plus.
pub const MAX_WORDS: usize = 8;
/// Longueur maximale d'un mot, en unités UTF-16.
pub const MAX_WORD_LENGTH: usize = 64;
/// Longueur maximale du texte cherché, en unités UTF-16.
pub const MAX_QUERY_LENGTH: usize = 300;

/// Élisions françaises retirées devant une apostrophe (en plus d'une lettre seule).
const ELISIONS: [&str; 5] = ["qu", "jusqu", "lorsqu", "puisqu", "quoiqu"];

/// `’ ‘ ʼ` lus comme `'`.
fn straight_apostrophe(c: char) -> char {
    if matches!(c, '\u{2018}' | '\u{2019}' | '\u{2bc}') {
        '\''
    } else {
        c
    }
}

/// `[\p{L}\p{M}\p{N}@._'-]` : ce qu'un mot garde ; tout le reste sépare les mots.
fn is_word_char(c: char) -> bool {
    is_letter(c) || is_mark(c) || is_number(c) || matches!(c, '@' | '.' | '_' | '\'' | '-')
}

/// `^[-.']+|[-.']+$` retirés.
fn trim_edges(word: &str) -> &str {
    word.trim_matches(|c| matches!(c, '-' | '.' | '\''))
}

/// Lettres et chiffres (`[\p{L}\p{N}]`).
fn count_letters_and_digits(text: &str) -> usize {
    text.chars()
        .filter(|&c| is_letter(c) || is_number(c))
        .count()
}

/// `SAFE_TERM` : `^[\p{L}\p{M}\p{N}@._'][\p{L}\p{M}\p{N}@._'-]*$`. Un terme tel que la grammaire
/// de Zimbra le lit, sans rien qui puisse être un opérateur (ni espace, ni `: ( ) { } " + *`, ni
/// `-` en tête).
#[must_use]
pub fn is_safe_term(term: &str) -> bool {
    let mut chars = term.chars();
    chars.next().is_some_and(|c| is_word_char(c) && c != '-') && chars.all(is_word_char)
}

/// `foldMailSearchText` : sans casse, sans accents, apostrophes droites. « etude » trouve
/// « Étude », « l'examen » trouve « l’examen ».
#[must_use]
pub fn fold_search_text(value: &str) -> String {
    let stripped: String = value.nfd().filter(|&c| !is_mark(c)).collect();
    nfc(&stripped)
        .chars()
        .map(straight_apostrophe)
        .collect::<String>()
        .to_lowercase()
}

/// Les parties d'un mot entre apostrophes, ses élisions de tête retirées : « l'examen » →
/// `[examen]`, « aujourd'hui » → `[aujourd, hui]`, « qu'il » → `[il]`.
fn meaningful_parts(word: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = word.split('\'').filter(|part| !part.is_empty()).collect();
    while parts.len() > 1 {
        let Some(first) = parts.first() else { break };
        let elision = count_letters_and_digits(first) <= 1
            || ELISIONS.contains(&first.to_lowercase().as_str());
        if !elision {
            break;
        }
        parts.remove(0);
    }
    parts
}

/// `parseMailSearchWords` : le texte du champ de recherche en mots. Coupé aux espaces et à toute
/// ponctuation sauf `@ . _ ' -` (une adresse, « TD-2 », « l'examen » restent entiers). Un mot doit
/// garder deux lettres ou chiffres une fois son élision retirée ; 64 unités par mot, 8 mots au
/// plus, 300 unités lues.
#[must_use]
pub fn parse_search_words(query: &str) -> Vec<String> {
    let text: String = nfc(cut_utf16(query, MAX_QUERY_LENGTH))
        .chars()
        .map(straight_apostrophe)
        .collect();
    text.split(|c: char| !is_word_char(c))
        .map(|word| trim_edges(cut_utf16(trim_edges(word), MAX_WORD_LENGTH)).to_owned())
        .filter(|word| count_letters_and_digits(&meaningful_parts(word).join("'")) >= 2)
        .take(MAX_WORDS)
        .collect()
}

/// `getMailSearchVariants` : les formes sous lesquelles un mot est cherché, en minuscules : le mot
/// lui-même, sans son élision, sa première partie, et chacune sans accents. Une forme qu'une autre
/// commence est laissée de côté (le préfixe le plus court la trouve déjà).
#[must_use]
pub fn search_variants(word: &str) -> Vec<String> {
    let lower = word.to_lowercase();
    let parts = meaningful_parts(&lower);
    let joined = parts.join("'");
    let first = parts.first().copied().unwrap_or_default().to_owned();
    let mut unique: Vec<String> = Vec::new();
    for form in [lower.clone(), joined, first] {
        let folded = fold_search_text(&form);
        for candidate in [form, folded] {
            if count_letters_and_digits(&candidate) >= 2
                && is_safe_term(&candidate)
                && !unique.contains(&candidate)
            {
                unique.push(candidate);
            }
        }
    }
    unique
        .iter()
        .filter(|form| {
            !unique
                .iter()
                .any(|other| other != *form && form.starts_with(other.as_str()))
        })
        .cloned()
        .collect()
}

/// `buildZimbraSearchQuery` : la requête Zimbra des mots (de [`parse_search_words`]) dans un
/// dossier : `inid:<dossier>` puis, pour chaque mot, `(from:x* OR to:x* OR subject:x* OR
/// content:x* …)` sur toutes ses formes. `None` si le dossier n'est pas un identifiant
/// (`^[0-9]{1,9}$`).
#[must_use]
pub fn build_search_query(words: &[String], folder: &str) -> Option<String> {
    if !is_valid_folder_id(folder) {
        return None;
    }
    let mut query = format!("inid:{folder}");
    for word in words {
        let variants = search_variants(word);
        if variants.is_empty() {
            continue;
        }
        let terms: Vec<String> = ["from", "to", "subject", "content"]
            .iter()
            .flat_map(|field| {
                variants
                    .iter()
                    .map(move |variant| format!("{field}:{variant}*"))
            })
            .collect();
        query.push_str(" (");
        query.push_str(&terms.join(" OR "));
        query.push(')');
    }
    Some(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(query: &str) -> Vec<String> {
        parse_search_words(query)
    }

    #[test]
    fn mots_coupes_aux_espaces_et_a_la_ponctuation() {
        assert_eq!(
            words("  Moodle   TD#2 (urgent) "),
            ["Moodle", "TD", "urgent"]
        );
        assert_eq!(
            words("jean.dupont@univ-rennes.fr."),
            ["jean.dupont@univ-rennes.fr"]
        );
        assert_eq!(words("TD-12"), ["TD-12"]);
        assert!(words("").is_empty());
    }

    #[test]
    fn apostrophes_et_elisions() {
        assert_eq!(words("l'examen"), ["l'examen"]);
        assert_eq!(words("l’examen d’anglais"), ["l'examen", "d'anglais"]);
        assert_eq!(search_variants("l'examen"), ["l'examen", "examen"]);
        assert_eq!(search_variants("qu'il"), ["qu'il", "il"]);
        assert_eq!(search_variants("aujourd'hui"), ["aujourd"]);
    }

    #[test]
    fn deux_lettres_ou_chiffres_au_moins() {
        for query in ["a", "é", "l'", "l'a", "-", "...", "@@", "*", "é\u{301}"] {
            assert!(words(query).is_empty(), "{query}");
        }
        assert_eq!(words("a examen 2"), ["examen"]);
        assert_eq!(words("ab"), ["ab"]);
        // Des chiffres d'autres écritures sont des chiffres (`\p{N}`), comme en JavaScript.
        assert_eq!(words("١٢"), ["١٢"]);
    }

    #[test]
    fn nombre_et_longueur_des_mots_bornes() {
        let many: Vec<String> = (0..20).map(|i| format!("mot{i}")).collect();
        assert_eq!(words(&many.join(" ")).len(), MAX_WORDS);
        assert_eq!(words(&"x".repeat(200))[0].len(), 64);
        // 300 unités lues : le neuvième mot de 40 lettres est hors de portée de toute façon.
        let long = format!("{} fin", "y".repeat(400));
        assert_eq!(words(&long), ["y".repeat(64)]);
    }

    #[test]
    fn requete_par_mot_et_par_forme() {
        assert_eq!(
            build_search_query(&words("Moodle"), "2").unwrap(),
            "inid:2 (from:moodle* OR to:moodle* OR subject:moodle* OR content:moodle*)"
        );
        assert_eq!(
            build_search_query(&words("Étude stage"), "5").unwrap(),
            "inid:5 (from:étude* OR from:etude* OR to:étude* OR to:etude* OR subject:étude* OR subject:etude* OR content:étude* OR content:etude*) (from:stage* OR to:stage* OR subject:stage* OR content:stage*)"
        );
        assert!(
            build_search_query(&words("l’examen"), "2")
                .unwrap()
                .starts_with("inid:2 (from:l'examen* OR from:examen* OR ")
        );
    }

    #[test]
    fn elisions_et_accents_comme_tafac() {
        // Sortie de `buildZimbraSearchQuery` de Tafac pour le même texte.
        assert_eq!(
            build_search_query(&words("Qu'il l'Été aujourd’hui jusqu'à"), "2").unwrap(),
            "inid:2 (from:qu'il* OR from:il* OR to:qu'il* OR to:il* OR subject:qu'il* OR subject:il* OR content:qu'il* OR content:il*) (from:l'été* OR from:l'ete* OR from:été* OR from:ete* OR to:l'été* OR to:l'ete* OR to:été* OR to:ete* OR subject:l'été* OR subject:l'ete* OR subject:été* OR subject:ete* OR content:l'été* OR content:l'ete* OR content:été* OR content:ete*) (from:aujourd* OR to:aujourd* OR subject:aujourd* OR content:aujourd*)"
        );
    }

    #[test]
    fn rien_de_tape_ne_devient_operateur() {
        let hostile = [
            "from:boss@x.fr",
            "in:trash",
            "-moodle",
            "+moodle",
            "\"exact phrase\"",
            "a OR b",
            "(x) {y}",
            "is:unread",
            "\\\"x",
            "content:*",
            "tag:\\x",
            "*",
            "x* y*",
            "not:me",
            "a\tb\nc",
            "\u{2010}moodle",
        ];
        for query in hostile {
            let built = build_search_query(&words(query), "2").unwrap();
            let groups = built.strip_prefix("inid:2").unwrap().trim();
            if groups.is_empty() {
                continue;
            }
            for group in groups.split(") (") {
                let group = group.trim_start_matches('(').trim_end_matches(')');
                for term in group.split(" OR ") {
                    let (field, rest) = term.split_once(':').unwrap();
                    assert!(
                        ["from", "to", "subject", "content"].contains(&field),
                        "{query} → {term}"
                    );
                    let value = rest.strip_suffix('*').unwrap();
                    assert!(is_safe_term(value), "{query} → {term}");
                }
            }
        }
    }

    #[test]
    fn le_dossier_est_un_identifiant() {
        let moodle = vec!["moodle".to_owned()];
        assert!(build_search_query(&moodle, "2 OR in:trash").is_none());
        assert!(build_search_query(&moodle, "").is_none());
        // Chiffres Unicode refusés : `\d` est ASCII en JavaScript.
        assert!(build_search_query(&moodle, "٢").is_none());
        assert!(build_search_query(&moodle, "1234567890").is_none());
        assert_eq!(build_search_query(&[], "257").unwrap(), "inid:257");
    }

    #[test]
    fn repli_des_accents_et_apostrophes() {
        assert_eq!(fold_search_text("Étude d’ÉTÉ"), "etude d'ete");
        assert_eq!(fold_search_text("l‘ex"), fold_search_text("l'ex"));
        assert_eq!(fold_search_text(""), "");
    }

    #[test]
    fn termes_surs() {
        for good in ["moodle", "l'examen", "a-b", "x@y.fr", "_x", ".x", "'x"] {
            assert!(is_safe_term(good), "{good}");
        }
        for bad in ["", "-x", "a b", "a:b", "a*", "(a", "a\"", "+a", "a{"] {
            assert!(!is_safe_term(bad), "{bad}");
        }
    }
}
