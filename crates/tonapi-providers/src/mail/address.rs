//! Adresses et noms affichés : ce qui pourra un jour entrer dans les en-têtes d'un mail est tenu
//! strictement. Repris de `src/mailCompose.js` de Tafac (`isValidAddress`, `sanitizeDisplayName`,
//! `normalizeRecipientQuery`).
//!
//! Une adresse est une `addr-spec` simple : partie locale en dot-atom (lettres accentuées
//! admises), domaine d'au moins deux étiquettes, longueurs du RFC 5321. Jamais de forme citée ou
//! commentée, ni rien qui puisse finir une ligne d'en-tête ou ouvrir une autre adresse.

use super::text::{
    cap_chars, collapse_spaces, cut_utf16, is_letter, is_mark, is_number, nfc, replace_controls,
    trim_js, utf16_len,
};

/// Longueur maximale d'une adresse, en unités UTF-16.
pub const MAX_ADDRESS_LENGTH: usize = 254;
const MAX_LOCAL_PART_LENGTH: usize = 64;
/// Longueur maximale d'un nom affiché, en points de code.
pub const MAX_DISPLAY_NAME_LENGTH: usize = 128;
/// Destinataires au plus, À, Cc et Cci ensemble.
pub const MAX_RECIPIENTS: usize = 100;

/// Requête de suggestion : longueurs en points de code.
pub const RECIPIENT_QUERY_MIN_LENGTH: usize = 2;
pub const RECIPIENT_QUERY_MAX_LENGTH: usize = 64;
/// Suggestions de destinataires au plus.
pub const MAX_RECIPIENT_SUGGESTIONS: usize = 12;

fn is_atom_char(c: char) -> bool {
    is_letter(c) || is_number(c) || "!#$%&'*+/=?^_`{|}~-".contains(c)
}

/// `LOCAL_PART` : dot-atom, des points seulement entre deux atomes.
fn is_local_part(local: &str) -> bool {
    !local.is_empty()
        && local
            .split('.')
            .all(|atom| !atom.is_empty() && atom.chars().all(is_atom_char))
}

/// `DOMAIN_LABEL` : lettres et chiffres, tirets à l'intérieur seulement, 63 points de code au plus.
fn is_domain_label(label: &str) -> bool {
    let chars: Vec<char> = label.chars().collect();
    let edge = |c: &char| is_letter(*c) || is_number(*c);
    match (chars.first(), chars.last()) {
        (Some(first), Some(last)) => {
            chars.len() <= 63
                && edge(first)
                && edge(last)
                && chars.iter().all(|c| edge(c) || *c == '-')
        }
        _ => false,
    }
}

/// `TOP_LEVEL_DOMAIN` : 2 à 63 lettres, ou `xn--` suivi de 1 à 59 caractères ASCII.
fn is_top_level_domain(label: &str) -> bool {
    let count = label.chars().count();
    let letters = (2..=63).contains(&count) && label.chars().all(is_letter);
    // Octets plutôt que tranches de texte : `abcé` n'a pas de frontière de caractère à 4.
    let bytes = label.as_bytes();
    let punycode = bytes.len() > 4
        && bytes[..4].eq_ignore_ascii_case(b"xn--")
        && (1..=59).contains(&(bytes.len() - 4))
        && bytes[4..]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-');
    letters || punycode
}

/// `isValidAddress`.
#[must_use]
pub fn is_valid_address(email: &str) -> bool {
    let value = nfc(trim_js(email));
    if value.is_empty() || utf16_len(&value) > MAX_ADDRESS_LENGTH {
        return false;
    }
    let Some(at) = value.rfind('@') else {
        return false;
    };
    if at == 0 {
        return false;
    }
    let local = &value[..at];
    let labels: Vec<&str> = value[at + 1..].split('.').collect();
    utf16_len(local) <= MAX_LOCAL_PART_LENGTH
        && is_local_part(local)
        && labels.len() >= 2
        && labels.iter().all(|label| is_domain_label(label))
        && labels.last().is_some_and(|tld| is_top_level_domain(tld))
}

/// `normalizeEmail` : clé d'une adresse, quelle que soit la casse.
#[must_use]
pub fn normalize_email(email: &str) -> String {
    nfc(trim_js(email)).to_lowercase()
}

/// Un nom qui se fait passer pour une adresse : `@`, ses formes pleine chasse et petite
/// (U+FF20, U+FE6B), ou un mot encodé du RFC 2047 (`=?utf-8?b?…?=`), que les clients décodent.
fn looks_like_address(name: &str) -> bool {
    if name.contains(['@', '\u{ff20}', '\u{fe6b}']) {
        return true;
    }
    name.find("=?")
        .is_some_and(|start| name[start + 2..].contains("?="))
}

/// `sanitizeDisplayName` : un nom à montrer à côté d'une adresse, sûr dans un en-tête. Aucun
/// contrôle, aucun de `< > " \`, une espace entre les mots, 128 points de code au plus. Vide quand
/// il ressemble à une adresse ou ne fait que répéter `email`.
#[must_use]
pub fn sanitize_display_name(name: &str, email: &str) -> String {
    let text = replace_controls(&nfc(name), Some(' '));
    let text: String = text
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | '"' | '\\') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let text = collapse_spaces(&text);
    let text = trim_js(cap_chars(trim_js(&text), MAX_DISPLAY_NAME_LENGTH)).to_owned();
    if text.is_empty() || looks_like_address(&text) || text.to_lowercase() == normalize_email(email)
    {
        return String::new();
    }
    text
}

/// `normalizeRecipientQuery` : le texte cherché parmi les destinataires. Lettres, chiffres et
/// `@ . _ + ' -` seulement (aucun joker ni opérateur n'atteint l'annuaire), une espace entre les
/// mots, 64 points de code au plus. Vide quand ce n'est pas une chaîne.
#[must_use]
pub fn normalize_recipient_query(text: &str) -> String {
    let text = nfc(cut_utf16(text, 256));
    let mut kept = String::with_capacity(text.len());
    let mut in_other = false;
    for c in text.chars() {
        let c = if matches!(c, '\u{2018}' | '\u{2019}' | '\u{2bc}') {
            '\''
        } else {
            c
        };
        let allowed = is_letter(c) || is_mark(c) || is_number(c) || "@._+'-".contains(c);
        if allowed {
            kept.push(c);
            in_other = false;
        } else if !in_other {
            kept.push(' ');
            in_other = true;
        }
    }
    let kept = collapse_spaces(&kept);
    trim_js(cap_chars(trim_js(&kept), RECIPIENT_QUERY_MAX_LENGTH)).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adresses_valides_et_refusees() {
        for good in [
            "camille.demo@univ-rennes.fr",
            "élève@exemple.fr",
            "a+b@x.y.example.org",
            "x@exemple.xn--p1ai",
            " camille@exemple.fr ",
            // Un domaine de premier niveau en lettres non ASCII : pas de frontière de caractère
            // à l'octet 4, la vérification du punycode paniquait (revue du 27/11).
            "x@univ.abcé",
            "x@a.éaé",
        ] {
            assert!(is_valid_address(good), "{good}");
        }
        for bad in [
            "",
            "@exemple.fr",
            "camille@exemple",
            "camille@exemple.f",
            "\"camille\"@exemple.fr",
            "camille demo@exemple.fr",
            "camille@exemple..fr",
            "camille@-exemple.fr",
            "camille.@exemple.fr",
            "<camille@exemple.fr>",
            "camille@exem\nple.fr",
            "camille@exemple.12",
            "x@a.xn--é",
        ] {
            assert!(!is_valid_address(bad), "{bad:?}");
        }
        let long_local = format!("{}@exemple.fr", "a".repeat(65));
        assert!(!is_valid_address(&long_local));
        let long = format!("a@{}.fr", "b".repeat(260));
        assert!(!is_valid_address(&long));
    }

    #[test]
    fn noms_affiches() {
        assert_eq!(
            sanitize_display_name("  Camille\n <Demo> \"x\" ", "c@exemple.fr"),
            "Camille Demo x"
        );
        for spoof in [
            "boss@univ.fr",
            "Boss\u{ff20}univ.fr",
            "Boss\u{fe6b}univ.fr",
            "=?utf-8?b?Ym9zc0B1bml2LmZy?=",
        ] {
            assert_eq!(sanitize_display_name(spoof, "c@exemple.fr"), "", "{spoof}");
        }
        assert_eq!(sanitize_display_name("C@Exemple.fr", "c@exemple.fr"), "");
        assert_eq!(
            sanitize_display_name(&"é".repeat(200), "").chars().count(),
            128
        );
    }

    #[test]
    fn requete_de_suggestion() {
        assert_eq!(normalize_recipient_query("  cam’ (mar)*  "), "cam' mar");
        assert_eq!(normalize_recipient_query("a:b\"c"), "a b c");
        assert_eq!(
            normalize_recipient_query(&"x".repeat(300)).chars().count(),
            64
        );
    }
}
