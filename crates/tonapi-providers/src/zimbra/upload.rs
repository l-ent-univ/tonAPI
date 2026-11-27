//! Envoi d'un fichier au servlet d'upload de Zimbra (`/service/upload?fmt=raw,extended`) : les
//! parties sans requête. Repris de `server/mailUpload.js` de Tafac
//! (`parseZimbraUploadResponse`, `isValidZimbraUploadId`, `buildZimbraUploadDisposition`,
//! `readContentLength`).
//!
//! Le fichier passe en flux, jamais écrit sur disque ni gardé entier : c'est l'affaire de
//! l'appelant, qui lit aussi au plus 64 Kio de la réponse du servlet.

use std::fmt::Write as _;

use serde_json::Value;

use crate::mail::attachments::{normalize_attachment_type, sanitize_attachment_name};
use crate::mail::text::{cut_utf16, is_js_space, trim_js};

use super::listing::{MAX_SAFE_INTEGER, js_number, js_string};

/// Réponse du servlet lue au plus, en unités UTF-16 (l'appelant en lit 64 Kio).
pub const MAX_UPLOAD_RESPONSE_LENGTH: usize = 64 * 1024;
/// Chemin du servlet, sous l'origine du webmail : la réponse sur une ligne, avec l'identifiant, le
/// type, le nom et la taille de chaque fichier.
pub const UPLOAD_PATH: &str = "/service/upload?fmt=raw,extended";

/// Un fichier que le servlet a gardé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    /// Identifiant de pièce jointe (`aid`), qu'un envoi nommera.
    pub aid: String,
    /// Nom, nettoyé (`sanitizeAttachmentName`).
    pub name: String,
    /// Taille en octets, ou `None` si le servlet n'en donne pas de lisible.
    pub size: Option<u64>,
    /// Type, normalisé (`normalizeAttachmentType`).
    pub content_type: String,
}

/// La réponse du servlet : son statut (dans le corps, pas celui de HTTP) et les fichiers gardés
/// (aucun hors statut 200).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadResponse {
    /// `200`, `413` (trop gros), `401` (pas de session, CSRF), `204`…
    pub status: u16,
    /// Fichiers gardés, vérifiés.
    pub uploads: Vec<Upload>,
}

/// `isValidZimbraUploadId` : `^[A-Za-z0-9:._-]{8,200}$`. Jamais de virgule : les identifiants sont
/// joints par `,` dans `attach.aid`.
#[must_use]
pub fn is_valid_upload_id(value: &str) -> bool {
    (8..=200).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'.' | b'_' | b'-'))
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// La forme `^\s*(\d{3})\s*,\s*'(?:[^'\\]|\\.)*'\s*(?:,\s*([\s\S]*?))?\s*$` lue sans expression
/// régulière : le statut et, s'il y en a une, la liste après la seconde virgule (sans espaces de
/// bord).
fn split_upload_response(text: &str) -> Option<(u16, Option<&str>)> {
    let rest = text.trim_start_matches(is_js_space);
    let digits = rest.get(..3)?;
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let status: u16 = digits.parse().ok()?;
    let rest = rest[3..].trim_start_matches(is_js_space);
    let rest = rest.strip_prefix(',')?.trim_start_matches(is_js_space);
    let rest = rest.strip_prefix('\'')?;
    // La chaîne entre apostrophes : `\` suivi d'un caractère (pas une fin de ligne).
    let mut chars = rest.char_indices();
    let end = loop {
        let (index, c) = chars.next()?;
        match c {
            '\'' => break index + 1,
            '\\' => {
                let (_, escaped) = chars.next()?;
                if is_line_terminator(escaped) {
                    return None;
                }
            }
            _ => {}
        }
    };
    let tail = rest[end..].trim_start_matches(is_js_space);
    if tail.is_empty() {
        return Some((status, None));
    }
    let list = tail.strip_prefix(',')?;
    Some((status, Some(trim_js(list))))
}

/// `Number.isSafeInteger(Number(s)) && Number(s) >= 0` : la taille, sinon `None`.
fn upload_size(value: Option<&Value>) -> Option<u64> {
    // `Number(undefined)` : NaN.
    let number = js_number(value?)?;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "entier positif inférieur à 2^53"
    )]
    let size = (number.fract() == 0.0 && (0.0..=MAX_SAFE_INTEGER).contains(&number))
        .then_some(number as u64);
    size
}

/// `parseZimbraUploadResponse` : la réponse du servlet (`fmt=raw,extended`, en `text/html` quoi
/// qu'il arrive), par exemple `200,'null',[{"aid":"…","ct":"…","filename":"…","s":123}]` ou
/// `413,'null'`. Les fichiers sont vérifiés : un identifiant de caractères sûrs, le nom et le type
/// nettoyés comme ceux de la page, une taille entière positive. `None` quand ce n'est pas du tout
/// cette forme (une page HTML, rien, une liste illisible). Lu sur 64 Kio au plus.
#[must_use]
pub fn parse_upload_response(text: &str) -> Option<UploadResponse> {
    let (status, list) = split_upload_response(cut_utf16(text, MAX_UPLOAD_RESPONSE_LENGTH))?;
    if status != 200 {
        return Some(UploadResponse {
            status,
            uploads: Vec::new(),
        });
    }
    let list: Value = serde_json::from_str(list?).ok()?;
    let list = list.as_array()?;
    let uploads = list
        .iter()
        .filter_map(|entry| {
            let aid = entry.get("aid").and_then(Value::as_str)?;
            if !entry.is_object() || !is_valid_upload_id(aid) {
                return None;
            }
            let name = entry
                .get("filename")
                .filter(|name| !name.is_null())
                .and_then(js_string)
                .unwrap_or_default();
            let content_type = entry
                .get("ct")
                .filter(|kind| !kind.is_null())
                .and_then(js_string)
                .unwrap_or_default();
            Some(Upload {
                aid: aid.to_owned(),
                name: sanitize_attachment_name(&name),
                size: upload_size(entry.get("s")),
                content_type: normalize_attachment_type(&content_type),
            })
        })
        .collect();
    Some(UploadResponse { status, uploads })
}

/// `buildZimbraUploadDisposition` : le `Content-Disposition` envoyé à Partage, comme l'écrit le
/// client web de Zimbra : `attachment; filename="…"`, le nom nettoyé, en ASCII seulement : tout
/// caractère hors de 0x20-0x7e, et `& " \ ;`, devient une entité numérique `&#n;`, que le servlet
/// décode.
#[must_use]
pub fn upload_disposition(name: &str) -> String {
    let clean = sanitize_attachment_name(name);
    let mut encoded = String::with_capacity(clean.len());
    for c in clean.chars() {
        let code = u32::from(c);
        if !(0x20..=0x7e).contains(&code) || matches!(c, '&' | '"' | '\\' | ';') {
            let _ = write!(encoded, "&#{code};");
        } else {
            encoded.push(c);
        }
    }
    format!("attachment; filename=\"{encoded}\"")
}

/// `readContentLength` : la longueur déclarée d'une requête, `^[0-9]{1,15}$` une fois les espaces
/// de bord retirées (chiffres ASCII seulement) ; `None` sinon (absente, négative, une liste, autre
/// chose), ce que la route refuse (411).
#[must_use]
pub fn read_content_length(header: Option<&str>) -> Option<u64> {
    let value = trim_js(header.unwrap_or_default());
    if !(1..=15).contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reponse_du_servlet_verifiee() {
        let ok = parse_upload_response(
            "200,'null',[{\"aid\":\"4f6e1c2a-0b7d-4c8e-9a51-2f3e4d5c6b7a:9c1d2e3f-4a5b-6c7d-8e9f-0a1b2c3d4e5f\",\"ct\":\"application/pdf\",\"filename\":\"Rapport final.pdf\",\"s\":48213}]\n",
        );
        assert_eq!(
            ok,
            Some(UploadResponse {
                status: 200,
                uploads: vec![Upload {
                    aid:
                        "4f6e1c2a-0b7d-4c8e-9a51-2f3e4d5c6b7a:9c1d2e3f-4a5b-6c7d-8e9f-0a1b2c3d4e5f"
                            .into(),
                    name: "Rapport final.pdf".into(),
                    size: Some(48213),
                    content_type: "application/pdf".into(),
                }],
            })
        );
        // Les refus n'ont pas de liste.
        assert_eq!(
            parse_upload_response("413,'null'"),
            Some(UploadResponse {
                status: 413,
                uploads: vec![]
            })
        );
        assert_eq!(
            parse_upload_response("401,'null'\r\n"),
            Some(UploadResponse {
                status: 401,
                uploads: vec![]
            })
        );
        // Un identifiant de requête qui contient une apostrophe échappée.
        let escaped = parse_upload_response(
            "200,'a\\'b',[{\"aid\":\"abcdefgh:1234\",\"ct\":\"text/plain\",\"filename\":\"a.txt\",\"s\":3}]",
        )
        .unwrap();
        assert_eq!(escaped.uploads[0].aid, "abcdefgh:1234");
        // Un identifiant inventé, un nom et un type hostiles : nettoyés ou laissés de côté.
        let odd = parse_upload_response(
            "200,'null',[{\"aid\":\"../../etc\",\"ct\":\"text/html; x\",\"filename\":\"a\"},{\"aid\":\"abcdefgh:5678\",\"ct\":\"<script>\",\"filename\":\"..\\\\evil\\u202egpj.exe\",\"s\":-1}]",
        )
        .unwrap();
        assert_eq!(
            odd.uploads,
            [Upload {
                aid: "abcdefgh:5678".into(),
                name: "evilgpj.exe".into(),
                size: None,
                content_type: "application/octet-stream".into(),
            }]
        );
        // Pas la forme du servlet.
        assert_eq!(
            parse_upload_response("<html><body>Error</body></html>"),
            None
        );
        assert_eq!(parse_upload_response(""), None);
        assert_eq!(parse_upload_response("200,'null',[{"), None);
        assert_eq!(parse_upload_response("200,'null',{}"), None);
        assert_eq!(parse_upload_response("200,'null'"), None);
        assert_eq!(parse_upload_response("200,'null' x"), None);
        // Chiffres Unicode dans le statut, échappement suivi d'une fin de ligne : refusés.
        assert_eq!(parse_upload_response("٢٠٠,'null'"), None);
        assert_eq!(parse_upload_response("200,'a\\\nb'"), None);
        assert_eq!(parse_upload_response("200,'ouvert"), None);
    }

    #[test]
    fn tailles_lues_comme_javascript() {
        let one = |s: &str| {
            parse_upload_response(&format!(
                "200,'null',[{{\"aid\":\"abcdefgh:1\",\"filename\":\"a\"{s}}}]"
            ))
            .unwrap()
            .uploads[0]
                .size
        };
        assert_eq!(one(""), None);
        assert_eq!(one(",\"s\":\"12\""), Some(12));
        assert_eq!(one(",\"s\":null"), Some(0));
        assert_eq!(one(",\"s\":1.5"), None);
        assert_eq!(one(",\"s\":9007199254740992"), None);
        assert_eq!(one(",\"s\":\"١٢\""), None);
        // Sans nom ni type : les valeurs par défaut.
        let bare =
            parse_upload_response("200,'null',[{\"aid\":\"abcdefgh:1\"},[1],\"x\",null]").unwrap();
        assert_eq!(bare.uploads.len(), 1);
        assert_eq!(bare.uploads[0].name, "piece-jointe");
        assert_eq!(bare.uploads[0].content_type, "application/octet-stream");
    }

    #[test]
    fn reponse_lue_sur_64_kio() {
        let long = format!(
            "200,'null',[{{\"aid\":\"abcdefgh:1\"}}]{}",
            " ".repeat(70 * 1024)
        );
        assert!(parse_upload_response(&long).is_some());
        let cut = format!("200,'{}',[]", "x".repeat(70 * 1024));
        assert_eq!(parse_upload_response(&cut), None);
    }

    #[test]
    fn identifiants_d_upload() {
        assert!(is_valid_upload_id("abcdefgh:1234"));
        assert!(is_valid_upload_id(&"a".repeat(200)));
        for bad in [
            "short",
            "abcdefgh,1234",
            "abcdefgh 1234",
            "abcdéfgh:1",
            "../../etc",
            "١٢٣٤٥٦٧٨٩",
            &"a".repeat(201),
        ] {
            assert!(!is_valid_upload_id(bad), "{bad}");
        }
    }

    #[test]
    fn disposition_en_entites_pour_partage() {
        assert_eq!(
            upload_disposition("Été & co.pdf"),
            "attachment; filename=\"&#201;t&#233; &#38; co.pdf\""
        );
        assert_eq!(
            upload_disposition("a\\b;c.txt"),
            "attachment; filename=\"b&#59;c.txt\""
        );
        // ASCII seulement : tient dans tout en-tête.
        for name in ["日本語 😀.png", "a＂; filename=evil.html＼", "x\r\ny"] {
            let header = upload_disposition(name);
            assert!(
                header.bytes().all(|b| (0x20..=0x7e).contains(&b)),
                "{header}"
            );
            assert_eq!(header.matches('"').count(), 2, "{header}");
        }
        assert_eq!(
            upload_disposition(""),
            "attachment; filename=\"piece-jointe\""
        );
    }

    #[test]
    fn longueur_declaree() {
        assert_eq!(read_content_length(Some("1234")), Some(1234));
        assert_eq!(read_content_length(Some(" 0 ")), Some(0));
        assert_eq!(
            read_content_length(Some("999999999999999")),
            Some(999_999_999_999_999)
        );
        for value in [
            None,
            Some(""),
            Some("-1"),
            Some("1e6"),
            Some("12, 12"),
            Some("0x10"),
            Some("9999999999999999"),
            Some("١٢٣"),
            Some("+5"),
        ] {
            assert_eq!(read_content_length(value), None, "{value:?}");
        }
    }
}
