//! Règles de texte communes à la messagerie : caractères de contrôle, noms reçus, longueurs,
//! échappement et décodage HTML. Reprises de `src/mailText.js` de Tafac, à l'identique : la page
//! de Tafac et ses tests comptent sur ces sorties.
//!
//! Deux unités de longueur coexistent, comme en JavaScript :
//! - les **unités UTF-16** (`.length` et `.slice` en JavaScript) : [`utf16_len`], [`cut_utf16`] ;
//! - les **points de code** (`Array.from`) : [`cap_chars`].
//!
//! `len()` en Rust compte des octets : ne s'en servir pour aucune limite de ces règles.

use unicode_general_category::{GeneralCategory, get_general_category};
use unicode_normalization::UnicodeNormalization;

/// Lettre (`\p{L}`).
#[must_use]
pub fn is_letter(c: char) -> bool {
    matches!(
        get_general_category(c),
        GeneralCategory::UppercaseLetter
            | GeneralCategory::LowercaseLetter
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::OtherLetter
    )
}

/// Marque combinante (`\p{M}`).
#[must_use]
pub fn is_mark(c: char) -> bool {
    matches!(
        get_general_category(c),
        GeneralCategory::NonspacingMark
            | GeneralCategory::SpacingMark
            | GeneralCategory::EnclosingMark
    )
}

/// Chiffre ou nombre (`\p{N}`).
#[must_use]
pub fn is_number(c: char) -> bool {
    matches!(
        get_general_category(c),
        GeneralCategory::DecimalNumber
            | GeneralCategory::LetterNumber
            | GeneralCategory::OtherNumber
    )
}

/// `CONTROL_CHARACTERS` : `[\p{Cc}\p{Cf}\p{Co}\p{Cn}  ]`. Contrôles (CR, LF,
/// tabulation, NUL), formats (renversements bidi, ZWJ), usage privé, non attribués, séparateurs de
/// ligne et de paragraphe.
#[must_use]
pub fn is_control(c: char) -> bool {
    matches!(c, '\u{2028}' | '\u{2029}')
        || matches!(
            get_general_category(c),
            GeneralCategory::Control
                | GeneralCategory::Format
                | GeneralCategory::PrivateUse
                | GeneralCategory::Unassigned
        )
}

/// `DIRECTION_CHARACTERS` : contrôles et caractères qui changent le sens de ce qui suit
/// (`rapport\u{202e}fdp.exe` se lirait `rapportexe.pdf`). Les liants (ZWJ) restent.
#[must_use]
pub fn is_direction(c: char) -> bool {
    matches!(
        c,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            | '\u{2028}'
            | '\u{2029}'
    ) || get_general_category(c) == GeneralCategory::Control
}

/// Espace au sens de `\s` en JavaScript (WhiteSpace et LineTerminator d'ECMAScript), qui diffère
/// de [`char::is_whitespace`] : U+FEFF en est, U+0085 n'en est pas.
#[must_use]
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `text.trim()` de JavaScript.
#[must_use]
pub fn trim_js(text: &str) -> &str {
    text.trim_matches(is_js_space)
}

/// `.replace(/\s+/g, ' ')` : chaque suite d'espaces devient une espace.
#[must_use]
pub fn collapse_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if is_js_space(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// `.replace(CONTROL_CHARACTERS, replacement)` : chaque caractère de contrôle remplacé (ou
/// retiré avec `None`).
#[must_use]
pub fn replace_controls(text: &str, replacement: Option<char>) -> String {
    text.chars()
        .filter_map(|c| if is_control(c) { replacement } else { Some(c) })
        .collect()
}

/// Forme normale NFC.
#[must_use]
pub fn nfc(text: &str) -> String {
    text.nfc().collect()
}

/// Forme normale NFKD.
#[must_use]
pub fn nfkd(text: &str) -> String {
    text.nfkd().collect()
}

/// `cleanReceivedName` : un nom tel qu'il arrive dans un mail reçu (expéditeur, fichier), sans
/// caractère de contrôle ni de direction, espaces de bord retirées.
#[must_use]
pub fn clean_received_name(name: &str) -> String {
    let kept: String = name.chars().filter(|&c| !is_direction(c)).collect();
    trim_js(&kept).to_owned()
}

/// Longueur en unités UTF-16 (`.length` de JavaScript).
#[must_use]
pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// `text.slice(0, max)` de JavaScript : au plus `max` unités UTF-16. Un caractère qui
/// chevaucherait la limite est laissé dehors (JavaScript en garderait une demi-paire, qu'il
/// remplacerait ensuite).
#[must_use]
pub fn cut_utf16(text: &str, max: usize) -> &str {
    let mut units = 0;
    for (index, c) in text.char_indices() {
        units += c.len_utf16();
        if units > max {
            return &text[..index];
        }
    }
    text
}

/// `capLength` : au plus `max` points de code, sans couper une paire de substitution.
#[must_use]
pub fn cap_chars(text: &str, max: usize) -> &str {
    match text.char_indices().nth(max) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

/// Nombre de points de code.
#[must_use]
pub fn char_count(text: &str) -> usize {
    text.chars().count()
}

/// `escapeHtml` : `& < > " '` échappés, pour un texte ou une valeur d'attribut entre guillemets.
#[must_use]
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// `decodeEntities` : références numériques (hors de l'espace Unicode, 0 ou substitut :
/// retirées) et les quelques références nommées des mails et des portails ; une autre reste telle
/// qu'elle est écrite. Une seule passe : `&amp;lt;` donne `&lt;`.
///
/// Forme reconnue : `&(#x[0-9a-f]+|#[0-9]+|[a-z]+[0-9]*);`, sans tenir compte de la casse.
#[must_use]
pub fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some((consumed, decoded)) = read_reference(after) {
            match decoded {
                Reference::Char(c) => out.push(c),
                Reference::Dropped => {}
                Reference::Kept => out.push_str(&rest[at..=at + consumed]),
            }
            rest = &after[consumed..];
        } else {
            out.push('&');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

enum Reference {
    Char(char),
    Dropped,
    Kept,
}

/// Une référence au début de `text` (après `&`) : octets lus jusqu'au `;` compris, et ce qu'elle
/// donne. `None` si le texte n'a pas la forme d'une référence.
fn read_reference(text: &str) -> Option<(usize, Reference)> {
    let bytes = text.as_bytes();
    if bytes.first() == Some(&b'#') {
        let hex = matches!(bytes.get(1), Some(b'x' | b'X'));
        let start = if hex { 2 } else { 1 };
        let digits = bytes[start..]
            .iter()
            .take_while(|b| {
                if hex {
                    b.is_ascii_hexdigit()
                } else {
                    b.is_ascii_digit()
                }
            })
            .count();
        let end = start + digits;
        if digits == 0 || bytes.get(end) != Some(&b';') {
            return None;
        }
        let written = &text[start..end];
        let code = if hex {
            u32::from_str_radix(written.trim_start_matches('0'), 16)
                .ok()
                .or_else(|| written.bytes().all(|b| b == b'0').then_some(0))
        } else {
            written
                .trim_start_matches('0')
                .parse::<u32>()
                .ok()
                .or_else(|| written.bytes().all(|b| b == b'0').then_some(0))
        };
        // Un nombre trop grand pour u32 est hors de l'espace Unicode : retiré, comme 0.
        let decoded = code
            .filter(|&code| code > 0)
            .and_then(char::from_u32)
            .map_or(Reference::Dropped, Reference::Char);
        return Some((end + 1, decoded));
    }
    let letters = bytes.iter().take_while(|b| b.is_ascii_alphabetic()).count();
    if letters == 0 {
        return None;
    }
    let digits = bytes[letters..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    let end = letters + digits;
    if bytes.get(end) != Some(&b';') {
        return None;
    }
    let name = text[..end].to_ascii_lowercase();
    let decoded = match name.as_str() {
        "amp" => Reference::Char('&'),
        "lt" => Reference::Char('<'),
        "gt" => Reference::Char('>'),
        "quot" => Reference::Char('"'),
        "apos" => Reference::Char('\''),
        "nbsp" => Reference::Char('\u{a0}'),
        _ => Reference::Kept,
    };
    Some((end + 1, decoded))
}

/// `cleanTextLines` : lignes de texte comme une signature les garde. NFC, fins de ligne Unix,
/// tabulation en espace, sans caractère de contrôle, sans espace en fin de ligne, sans ligne vide
/// au début ni à la fin, jamais plus d'une ligne vide de suite.
#[must_use]
pub fn clean_text_lines(text: &str) -> String {
    let text = nfc(text).replace("\r\n", "\n").replace('\r', "\n");
    let text = text.replace('\t', " ");
    let lines: Vec<String> = text
        .split('\n')
        .map(|line| {
            let cleaned = replace_controls(line, None);
            cleaned.trim_end_matches(is_js_space).to_owned()
        })
        .collect();
    let joined = lines.join("\n");
    // `\n{3,}` → `\n\n`, puis les `\n` des bords.
    let mut out = String::with_capacity(joined.len());
    let mut newlines = 0;
    for c in joined.chars() {
        if c == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push('\n');
            }
        } else {
            newlines = 0;
            out.push(c);
        }
    }
    out.trim_matches('\n').to_owned()
}

/// Longueur maximale d'un objet, en points de code.
pub const MAX_SUBJECT_LENGTH: usize = 255;

/// `sanitizeSubject` : l'objet d'un mail, sur une ligne. Coupé d'abord à 1020 unités UTF-16, NFC,
/// caractères de contrôle (CR, LF, renversements bidi) en espaces, une espace entre les mots,
/// 255 points de code au plus.
#[must_use]
pub fn sanitize_subject(value: &str) -> String {
    let text = replace_controls(&nfc(cut_utf16(value, MAX_SUBJECT_LENGTH * 4)), Some(' '));
    let text = collapse_spaces(&text);
    trim_js(cap_chars(trim_js(&text), MAX_SUBJECT_LENGTH)).to_owned()
}

/// Texte d'une valeur JSON quand c'en est un, comme `typeof x === 'string'`.
#[must_use]
pub fn json_str(value: Option<&serde_json::Value>) -> Option<&str> {
    value.and_then(serde_json::Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controles_et_direction() {
        // Contrôle, format (bidi, ZWJ), usage privé, non attribué, séparateurs.
        for c in [
            '\n', '\0', '\u{202e}', '\u{200d}', '\u{e000}', '\u{0378}', '\u{2028}',
        ] {
            assert!(is_control(c), "{c:?}");
        }
        assert!(!is_control('a'));
        assert!(!is_control('é'));
        // Les liants restent dans un nom reçu.
        assert!(!is_direction('\u{200d}'));
        assert!(is_direction('\u{202e}'));
        assert_eq!(
            clean_received_name("  rapport\u{202e}fdp.exe\u{200d} "),
            "rapportfdp.exe\u{200d}"
        );
    }

    #[test]
    fn longueurs_en_unites_et_en_points_de_code() {
        assert_eq!(utf16_len("a😀"), 3);
        assert_eq!(cut_utf16("a😀b", 2), "a");
        assert_eq!(cut_utf16("a😀b", 3), "a😀");
        assert_eq!(cap_chars("a😀b", 2), "a😀");
        assert_eq!(cap_chars("ab", 5), "ab");
    }

    #[test]
    fn espaces_de_javascript() {
        assert_eq!(collapse_spaces("a \u{a0}\t b\u{feff}c"), "a b c");
        assert_eq!(trim_js("\u{feff} a \u{3000}"), "a");
        // U+0085 n'est pas une espace pour JavaScript.
        assert_eq!(trim_js("\u{85}a"), "\u{85}a");
    }

    #[test]
    fn entites() {
        assert_eq!(
            decode_entities("&amp;lt; &LT; &#x6a;&#106; &eacute; &#0; &#x110000; &"),
            "&lt; < jj &eacute;   &"
        );
        assert_eq!(decode_entities("&nbsp;&#39;&apos;"), "\u{a0}''");
        assert_eq!(decode_entities("&#99999999999999999999;x"), "x");
        assert_eq!(decode_entities("&#xd800;"), "");
        assert_eq!(
            escape_html("<a href=\"x\">'&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
    }

    #[test]
    fn objet_sur_une_ligne() {
        assert_eq!(
            sanitize_subject(" Re:\r\nBcc: x\u{202e} \t ok "),
            "Re: Bcc: x ok"
        );
        assert_eq!(sanitize_subject(&"é".repeat(400)).chars().count(), 255);
    }

    #[test]
    fn lignes_d_une_signature() {
        assert_eq!(
            clean_text_lines("\n\nCamille  \r\n\tDemo\u{202e}\r\r\r\rFin\n\n"),
            "Camille\n Demo\n\nFin"
        );
    }
}
