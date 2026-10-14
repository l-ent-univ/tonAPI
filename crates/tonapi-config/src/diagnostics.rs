//! Messages d'erreur lisibles : position dans le fichier, traduction, suggestion.

use std::fmt::Write as _;
use std::path::Path;

use crate::validate::Issue;

/// Ligne et colonne (à partir de 1) d'un décalage en octets.
pub(crate) fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(src.len());
    while !src.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &src[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

/// Position de la première occurrence de `needle` dans le fichier.
pub(crate) fn locate(src: &str, needle: &str) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return None;
    }
    src.find(needle).map(|offset| line_col(src, offset))
}

/// Erreur de lecture TOML → [`Issue`] avec position, message en français et suggestion.
pub(crate) fn from_toml_error(file: &Path, src: &str, err: &toml::de::Error) -> Issue {
    let (message, suggestion) = translate(err.message());
    let position = err.span().map(|span| line_col(src, span.start));
    Issue {
        file: file.to_path_buf(),
        line: position.map(|p| p.0),
        column: position.map(|p| p.1),
        path: None,
        message,
        suggestion,
    }
}

/// Traduit les messages serde les plus courants et propose la valeur attendue la plus proche.
fn translate(message: &str) -> (String, Option<String>) {
    let ticks = backticked(message);
    let first_line = message.lines().next().unwrap_or(message).trim();
    let (kind, list_start) = if first_line.starts_with("unknown variant") {
        ("valeur inconnue", 1)
    } else if first_line.starts_with("unknown field") {
        ("champ inconnu", 1)
    } else if first_line.starts_with("missing field") {
        ("champ obligatoire manquant", 1)
    } else if first_line.contains("relative URL without a base") {
        return ("URL invalide : il manque `https://`".to_owned(), None);
    } else if first_line.starts_with("invalid") && first_line.contains("URL") {
        return (format!("URL invalide ({first_line})"), None);
    } else {
        return (first_line.to_owned(), None);
    };

    let Some(got) = ticks.first() else {
        return (first_line.to_owned(), None);
    };
    let expected = &ticks[list_start.min(ticks.len())..];
    let mut message = format!("{kind} `{got}`");
    if !expected.is_empty() {
        let list = expected
            .iter()
            .map(|e| format!("`{e}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = write!(message, " (attendu : {list})");
    }
    (message, closest(got, expected))
}

fn backticked(message: &str) -> Vec<&str> {
    message.split('`').skip(1).step_by(2).collect()
}

/// Candidat le plus proche, s'il est assez proche pour être une faute de frappe.
pub(crate) fn closest(got: &str, candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .map(|c| (strsim::jaro_winkler(got, c), *c))
        .filter(|(score, _)| *score >= 0.8)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, c)| c.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_en_lignes_et_colonnes() {
        let src = "a = 1\nbé = 2\nc = 3";
        assert_eq!(line_col(src, 0), (1, 1));
        assert_eq!(line_col(src, src.find("= 2").unwrap()), (2, 4));
        assert_eq!(locate(src, "c ="), Some((3, 1)));
    }

    #[test]
    fn variante_inconnue_traduite_avec_suggestion() {
        let (msg, sugg) = translate(
            "unknown variant `scodoc`, expected one of `scodoc-gateway`, `link`, `none`, `demo`",
        );
        assert!(msg.starts_with("valeur inconnue `scodoc`"), "{msg}");
        assert_eq!(sugg.as_deref(), Some("scodoc-gateway"));
    }

    #[test]
    fn champ_inconnu_sans_candidat_proche() {
        let (msg, sugg) = translate("unknown field `zzz`, expected `base-url` or `options`");
        assert!(msg.starts_with("champ inconnu `zzz`"), "{msg}");
        assert_eq!(sugg, None);
    }
}
