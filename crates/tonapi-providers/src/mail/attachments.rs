//! Pièces jointes : limites de taille, nom et type d'un fichier. Repris de
//! `src/mailAttachments.js` de Tafac.
//!
//! La limite de Partage (`attSizeLimit` de `GetInfo`) porte sur le mail entier tel qu'il part :
//! les pièces jointes en base64 prennent 4/3 de leur taille, plus un saut de ligne tous les 76
//! caractères. Ce que les fichiers peuvent peser est cela, moins la place du texte et des
//! en-têtes.

use super::text::{
    cap_chars, char_count, collapse_spaces, is_letter, is_number, nfc, replace_controls,
};

/// Limite d'un mail quand Partage ne la donne pas (ou dit « sans limite »).
pub const FALLBACK_MESSAGE_LIMIT_BYTES: u64 = 25 * 1024 * 1024;
/// Jamais plus par le serveur de Tafac, quoi que Partage permette.
pub const HARD_ATTACHMENT_LIMIT_BYTES: u64 = 50 * 1024 * 1024;
// Les mêmes en flottants, pour le calcul de `deriveAttachmentLimits` (valeurs exactes en f64).
const MESSAGE_OVERHEAD_F64: f64 = 256.0 * 1024.0;
const FALLBACK_MESSAGE_LIMIT_F64: f64 = 25.0 * 1024.0 * 1024.0;
/// Pièces jointes d'un mail au plus.
pub const MAX_ATTACHMENTS: usize = 20;
/// Longueur maximale d'un nom de fichier, en points de code.
pub const MAX_ATTACHMENT_NAME_LENGTH: usize = 180;
/// Nom d'un fichier qui n'en a pas.
pub const DEFAULT_ATTACHMENT_NAME: &str = "piece-jointe";

/// Limites des pièces jointes d'un compte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentLimits {
    /// Un fichier au plus.
    pub max_file_bytes: u64,
    /// Toutes les pièces d'un mail ensemble au plus (la même : un fichier seul a tout le mail).
    pub max_total_bytes: u64,
    /// La limite du mail entier retenue.
    pub message_limit_bytes: u64,
    /// La limite vient de Partage (au-dessus de 256 Kio), pas de la valeur de repli.
    pub known: bool,
}

/// `deriveAttachmentLimits` : `message_limit` est `attSizeLimit` en octets (-1 ou 0 : sans
/// limite ; absent : inconnu).
#[must_use]
pub fn derive_attachment_limits(message_limit: Option<f64>) -> AttachmentLimits {
    let known_limit =
        message_limit.filter(|value| value.is_finite() && *value > MESSAGE_OVERHEAD_F64);
    let message_limit_bytes = known_limit.unwrap_or(FALLBACK_MESSAGE_LIMIT_F64);
    let ratio = (4.0 / 3.0) * (78.0 / 76.0);
    let total = ((message_limit_bytes - MESSAGE_OVERHEAD_F64) / ratio).floor();
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "valeur positive et finie, bornée par la limite fixe"
    )]
    let total = (total.max(0.0) as u64).min(HARD_ATTACHMENT_LIMIT_BYTES);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "valeur positive et finie"
    )]
    let message_limit_bytes = message_limit_bytes as u64;
    AttachmentLimits {
        max_file_bytes: total,
        max_total_bytes: total,
        message_limit_bytes,
        known: known_limit.is_some(),
    }
}

/// `sanitizeAttachmentName` : un nom de fichier tel qu'il peut entrer dans un mail (et dans
/// l'envoi à Partage). Sans dossier (tout jusqu'au dernier `/` ou `\`), sans caractère de
/// contrôle ni de direction, sans `" * : < > ? |`, une espace entre les mots, sans point ni espace
/// aux bords, 180 points de code au plus en gardant l'extension.
#[must_use]
pub fn sanitize_attachment_name(name: &str) -> String {
    let text = nfc(name);
    let base = match text.rfind(['/', '\\']) {
        Some(at) => &text[at + 1..],
        None => text.as_str(),
    };
    let text = replace_controls(base, None);
    let text: String = text
        .chars()
        .map(|c| if "\"*:<>?|".contains(c) { '_' } else { c })
        .collect();
    let text = collapse_spaces(&text);
    let text = text
        .trim_matches(|c: char| c == '.' || super::text::is_js_space(c))
        .to_owned();
    let text = if char_count(&text) > MAX_ATTACHMENT_NAME_LENGTH {
        let extension = extension(&text);
        let kept = cap_chars(
            &text,
            MAX_ATTACHMENT_NAME_LENGTH - extension.chars().count(),
        );
        format!(
            "{}{extension}",
            kept.trim_end_matches(super::text::is_js_space)
        )
    } else {
        text
    };
    if text.is_empty() {
        DEFAULT_ATTACHMENT_NAME.to_owned()
    } else {
        text
    }
}

/// `/\.[\p{L}\p{N}]{1,10}$/u` : l'extension d'un nom, point compris, ou rien.
fn extension(text: &str) -> &str {
    let tail: Vec<(usize, char)> = text
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_letter(*c) || is_number(*c))
        .collect();
    let Some(&(start, _)) = tail.last() else {
        return "";
    };
    let dot = start.checked_sub(1);
    // Une extension de plus de 10 caractères : la regex prend les 10 derniers s'ils suivent un
    // point, ce qui n'arrive que lorsqu'il y en a exactement 10 au plus.
    match dot {
        Some(dot) if text.as_bytes()[dot] == b'.' && (1..=10).contains(&tail.len()) => &text[dot..],
        _ => "",
    }
}

/// `normalizeAttachmentType` : `type/sous-type` en minuscules, sans paramètres, de forme
/// `^[a-z0-9][\w.+-]{0,63}/[a-z0-9][\w.+-]{0,127}$` (`\w` ASCII) ; sinon un flux d'octets.
#[must_use]
pub fn normalize_attachment_type(written: &str) -> String {
    let value = written
        .split(';')
        .next()
        .unwrap_or_default()
        .trim_matches(super::text::is_js_space)
        .to_lowercase();
    let token = |part: &str, max: usize| {
        let bytes = part.as_bytes();
        bytes
            .first()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            && bytes.len() <= max + 1
            && bytes[1..]
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'+' | b'-'))
    };
    match value.split_once('/') {
        Some((kind, sub)) if token(kind, 63) && token(sub, 127) => value,
        _ => "application/octet-stream".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limites() {
        let fallback = derive_attachment_limits(None);
        assert!(!fallback.known);
        assert_eq!(fallback.message_limit_bytes, FALLBACK_MESSAGE_LIMIT_BYTES);
        // (25 Mio − 256 Kio) / (4/3 · 78/76), arrondi vers le bas.
        assert_eq!(fallback.max_file_bytes, 18_965_110);
        // Sans limite, ou trop petite : la valeur de repli.
        assert!(!derive_attachment_limits(Some(-1.0)).known);
        assert!(!derive_attachment_limits(Some(256.0 * 1024.0)).known);
        // Jamais plus de 50 Mio.
        let huge = derive_attachment_limits(Some(1e12));
        assert!(huge.known);
        assert_eq!(huge.max_total_bytes, HARD_ATTACHMENT_LIMIT_BYTES);
    }

    #[test]
    fn noms_de_fichier() {
        assert_eq!(
            sanitize_attachment_name("C:\\Users\\x\\rap*port?.pdf"),
            "rap_port_.pdf"
        );
        assert_eq!(sanitize_attachment_name("../../etc/passwd"), "passwd");
        assert_eq!(
            sanitize_attachment_name(" .. rapport\u{202e}fdp.exe .. "),
            "rapportfdp.exe"
        );
        assert_eq!(sanitize_attachment_name(""), DEFAULT_ATTACHMENT_NAME);
        assert_eq!(sanitize_attachment_name("..."), DEFAULT_ATTACHMENT_NAME);
        let long = format!("{}.pdf", "a".repeat(300));
        let cut = sanitize_attachment_name(&long);
        assert_eq!(cut.chars().count(), 180);
        assert_eq!(cut.rsplit('.').next(), Some("pdf"));
        let no_extension = "b".repeat(300);
        assert_eq!(sanitize_attachment_name(&no_extension).chars().count(), 180);
    }

    #[test]
    fn types() {
        assert_eq!(normalize_attachment_type("Image/PNG; name=x"), "image/png");
        assert_eq!(
            normalize_attachment_type("application/vnd.ms-excel"),
            "application/vnd.ms-excel"
        );
        for bad in [
            "",
            "image",
            "image/",
            "/png",
            "imagé/png",
            "image/p\u{0663}ng",
            "text/html\r\nx: y",
        ] {
            assert_eq!(
                normalize_attachment_type(bad),
                "application/octet-stream",
                "{bad:?}"
            );
        }
    }
}
