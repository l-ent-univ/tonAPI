//! Suggestions de destinataires de l'éditeur (`/mail/autocomplete`), lues dans
//! l'`AutoCompleteRequest` de Zimbra. Repris de `server/mailAutocomplete.js` de Tafac
//! (`buildZimbraAutoCompleteRequest`, `normalizeZimbraAutoComplete`) et de `src/mailCompose.js`
//! (`cleanRecipientSuggestions`, et la lecture d'une adresse écrite, `parseRecipientText`).
//!
//! `AutoCompleteRequest` cherche dans les contacts de l'étudiant, ses correspondants passés et
//! l'annuaire de l'établissement (la GAL), en une requête. Seuls un nom, une adresse et une
//! nature sortent d'ici : ni identifiant, ni dossier, ni classement.

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::mail::address::{
    MAX_ADDRESS_LENGTH, MAX_RECIPIENT_SUGGESTIONS, RECIPIENT_QUERY_MIN_LENGTH, is_valid_address,
    normalize_email, normalize_recipient_query, sanitize_display_name,
};
use crate::mail::text::{
    cap_chars, collapse_spaces, is_js_space, nfc, replace_controls, trim_js, utf16_len,
};

use super::listing::as_array;

/// Une entrée dont `email` dépasse cette longueur (en unités UTF-16) n'est pas une adresse
/// (l'expansion d'un groupe, ou du bruit) : laissée de côté sans être lue.
pub const MAX_MATCH_EMAIL_LENGTH: usize = 512;

/// Nature d'une suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionKind {
    /// Un contact de l'étudiant.
    Contact,
    /// Un correspondant passé.
    Recent,
    /// L'annuaire de l'établissement.
    Directory,
    /// Une liste de diffusion.
    Group,
}

impl SuggestionKind {
    /// Le nom que la page lit.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            SuggestionKind::Contact => "contact",
            SuggestionKind::Recent => "recent",
            SuggestionKind::Directory => "directory",
            SuggestionKind::Group => "group",
        }
    }

    /// Une nature écrite (`contact`, `recent`, `directory`, `group`) ; toute autre est un contact.
    #[must_use]
    pub fn from_written(kind: &str) -> SuggestionKind {
        match kind {
            "recent" => SuggestionKind::Recent,
            "directory" => SuggestionKind::Directory,
            "group" => SuggestionKind::Group,
            _ => SuggestionKind::Contact,
        }
    }
}

/// Une suggestion de destinataire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipientSuggestion {
    /// Nom affiché, nettoyé (peut être vide).
    pub name: String,
    /// Adresse valide.
    pub email: String,
    /// Nature.
    pub kind: SuggestionKind,
}

/// Une suggestion candidate, avant [`clean_recipient_suggestions`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionCandidate {
    /// Nom tel que reçu.
    pub name: String,
    /// Adresse telle que reçue.
    pub email: String,
    /// Nature écrite.
    pub kind: String,
}

/// Le texte cherché d'après le paramètre `q` (`normalizeRecipientQuery`), ou `None` quand il est
/// trop court pour qu'on demande quoi que ce soit (la route répond alors `{suggestions: []}`).
/// Un `q` répété ou en objet n'est pas une requête : l'appelant passe alors `None`.
#[must_use]
pub fn autocomplete_query(q: Option<&str>) -> Option<String> {
    let query = normalize_recipient_query(q?);
    (utf16_len(&query) >= RECIPIENT_QUERY_MIN_LENGTH).then_some(query)
}

/// `buildZimbraAutoCompleteRequest` : le corps d'`AutoCompleteRequest` (urn:zimbraMail), fixe
/// sauf le texte cherché (déjà passé par [`autocomplete_query`]). `t: account` : les personnes et
/// listes de l'annuaire, pas ses salles ni son matériel ; `includeGal` : l'annuaire avec les
/// contacts.
#[must_use]
pub fn autocomplete_request(query: &str) -> Value {
    json!({ "name": query, "t": "account", "includeGal": 1 })
}

// Lecture d'une adresse écrite (`parseRecipientText`)

/// Un destinataire écrit : valide (`name`, `email`) ou non.
struct Parsed {
    name: String,
    email: String,
    invalid: bool,
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// `splitRecipientText` : coupé aux virgules, points-virgules et fins de ligne, sauf dans un
/// « nom entre guillemets » ou une <adresse>. Une fin de ligne coupe toujours.
fn split_recipient_text(text: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let (mut quoted, mut angled, mut escaped) = (false, false, false);
    for c in text.chars() {
        if c == '\n' || c == '\r' {
            pieces.push(std::mem::take(&mut current));
            quoted = false;
            angled = false;
            continue;
        }
        if escaped {
            escaped = false;
        } else if quoted && c == '\\' {
            escaped = true;
        } else if c == '"' && !angled {
            quoted = !quoted;
        } else if c == '<' && !quoted {
            angled = true;
        } else if c == '>' && !quoted {
            angled = false;
        } else if (c == ',' || c == ';') && !quoted && !angled {
            pieces.push(std::mem::take(&mut current));
            continue;
        }
        current.push(c);
    }
    pieces.push(current);
    pieces
}

fn strip_mailto(text: &str) -> &str {
    match text.get(..7) {
        Some(prefix) if prefix.eq_ignore_ascii_case("mailto:") => &text[7..],
        _ => text,
    }
}

/// `unquoteName` : un nom entre guillemets perd ses guillemets et ses échappements.
fn unquote_name(name: &str) -> String {
    let trimmed = trim_js(name);
    // `"…"` sans condition ; `'…'` sans fin de ligne à l'intérieur (`.` de JavaScript).
    let wrapped =
        |quote: char| trimmed.len() >= 2 && trimmed.starts_with(quote) && trimmed.ends_with(quote);
    let quoted = wrapped('"')
        || (wrapped('\'') && !trimmed[1..trimmed.len() - 1].contains(is_line_terminator));
    let inner = if quoted {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };
    // `\\(.)` → le caractère (pas une fin de ligne).
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(&next) = chars.peek()
            && !is_line_terminator(next)
        {
            out.push(next);
            chars.next();
            continue;
        }
        out.push(c);
    }
    out
}

/// `^(\S+@\S+)\s*\(([^()]*)\)$` : une adresse suivie d'un commentaire entre parenthèses.
fn split_commented(piece: &str) -> Option<(&str, &str)> {
    let body = piece.strip_suffix(')')?;
    let open = body.rfind('(')?;
    let comment = &body[open + 1..];
    if comment.contains(')') {
        return None;
    }
    let address = body[..open].trim_end_matches(is_js_space);
    let at = address.char_indices().skip(1).find(|(_, c)| *c == '@')?.0;
    let valid = !address.contains(is_js_space) && at + 1 < address.len();
    valid.then_some((address, comment))
}

/// `parseRecipientPiece`.
fn parse_piece(piece: &str) -> Vec<Parsed> {
    let trimmed = trim_js(piece);
    if trimmed.is_empty() {
        return Vec::new();
    }
    let invalid = || Parsed {
        name: String::new(),
        email: trimmed.to_owned(),
        invalid: true,
    };
    // `^([\s\S]*?)<([^<>]*)>$` : le dernier `<`, puis rien d'autre qu'une adresse jusqu'au `>`.
    if let Some(body) = trimmed.strip_suffix('>')
        && let Some(open) = body.rfind('<')
        && !body[open + 1..].contains('>')
    {
        let email = strip_mailto(trim_js(&body[open + 1..]));
        return vec![if is_valid_address(email) {
            Parsed {
                name: sanitize_display_name(&unquote_name(&body[..open]), email),
                email: email.to_owned(),
                invalid: false,
            }
        } else {
            invalid()
        }];
    }
    if let Some((address, comment)) = split_commented(trimmed) {
        let email = strip_mailto(address);
        if is_valid_address(email) {
            return vec![Parsed {
                name: sanitize_display_name(comment, email),
                email: email.to_owned(),
                invalid: false,
            }];
        }
    }
    let words: Vec<&str> = trimmed
        .split(is_js_space)
        .filter(|word| !word.is_empty())
        .map(strip_mailto)
        .collect();
    if words.len() > 1 && words.iter().all(|word| is_valid_address(word)) {
        return words
            .into_iter()
            .map(|email| Parsed {
                name: String::new(),
                email: email.to_owned(),
                invalid: false,
            })
            .collect();
    }
    let email = strip_mailto(trimmed);
    vec![if is_valid_address(email) {
        Parsed {
            name: String::new(),
            email: email.to_owned(),
            invalid: false,
        }
    } else {
        invalid()
    }]
}

/// `normalizeRecipient` : un destinataire valide garde son nom nettoyé ; un invalide n'est gardé
/// que s'il reste du texte une fois nettoyé.
fn normalize_parsed(parsed: &Parsed) -> Option<Parsed> {
    let email = nfc(trim_js(&parsed.email));
    if !parsed.invalid && is_valid_address(&email) {
        return Some(Parsed {
            name: sanitize_display_name(&parsed.name, &email),
            email,
            invalid: false,
        });
    }
    let text = collapse_spaces(&replace_controls(&nfc(&parsed.email), Some(' ')));
    let text = trim_js(cap_chars(trim_js(&text), MAX_ADDRESS_LENGTH));
    (!text.is_empty()).then(|| Parsed {
        name: String::new(),
        email: text.to_owned(),
        invalid: true,
    })
}

/// `parseRecipientText` sur un texte court (l'appelant en borne la longueur).
fn parse_recipient_text(text: &str) -> Vec<Parsed> {
    split_recipient_text(text)
        .iter()
        .flat_map(|piece| parse_piece(piece))
        .filter_map(|parsed| normalize_parsed(&parsed))
        .collect()
}

// La réponse

fn text_field<'a>(entry: &'a Value, key: &str) -> &'a str {
    entry.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn is_true(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() == Some(1.0),
        Some(Value::String(s)) => s == "1" || s == "true",
        _ => false,
    }
}

/// `normalizeZimbraAutoComplete` : une `AutoCompleteResponse` en suggestions, dans l'ordre de
/// Zimbra, une fois par adresse, `limit` au plus (au plus 2 × `limit` entrées lues). Une entrée
/// doit donner exactement une adresse valide ; un groupe de contacts (sans adresse à lui) est
/// laissé de côté. Nature : `group` pour une liste, sinon selon `type` (`contact`,
/// `rankingTable` → `recent`, `gal` → `directory`, autre → `contact`).
#[must_use]
pub fn normalize_autocomplete(response: &Value, limit: usize) -> Vec<RecipientSuggestion> {
    let mut candidates = Vec::new();
    for entry in as_array(response.get("match")) {
        if candidates.len() >= limit.saturating_mul(2) {
            break;
        }
        let raw = text_field(entry, "email");
        if raw.is_empty() || utf16_len(raw) > MAX_MATCH_EMAIL_LENGTH {
            continue;
        }
        let mut parsed = parse_recipient_text(raw);
        if parsed.len() != 1 || parsed.first().is_none_or(|address| address.invalid) {
            continue;
        }
        let Some(address) = parsed.pop() else {
            continue;
        };
        let full = text_field(entry, "full");
        let full_name = if full.is_empty() {
            [text_field(entry, "first"), text_field(entry, "last")]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            full.to_owned()
        };
        let name = [
            address.name,
            full_name,
            text_field(entry, "display").to_owned(),
        ]
        .into_iter()
        .find(|name| !name.is_empty())
        .unwrap_or_default();
        let kind = if is_true(entry.get("isGroup")) {
            "group"
        } else {
            match entry.get("type").and_then(Value::as_str) {
                Some("rankingTable") => "recent",
                Some("gal") => "directory",
                _ => "contact",
            }
        };
        candidates.push(SuggestionCandidate {
            name,
            email: address.email,
            kind: kind.to_owned(),
        });
    }
    clean_recipient_suggestions(&candidates, limit)
}

/// `cleanRecipientSuggestions` : des suggestions telles que l'éditeur peut les montrer, d'où
/// qu'elles viennent. Une adresse valide, un nom sûr, une nature connue (sinon `contact`), une
/// fois chacune (sans casse), dans l'ordre, `limit` au plus.
#[must_use]
pub fn clean_recipient_suggestions(
    list: &[SuggestionCandidate],
    limit: usize,
) -> Vec<RecipientSuggestion> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for entry in list {
        if result.len() >= limit {
            break;
        }
        let email = nfc(trim_js(&entry.email));
        let key = normalize_email(&email);
        if !is_valid_address(&email) || seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        result.push(RecipientSuggestion {
            name: sanitize_display_name(&entry.name, &email),
            email,
            kind: SuggestionKind::from_written(&entry.kind),
        });
    }
    result
}

/// La réponse de `/mail/autocomplete` : `{suggestions: [{name, email, kind}]}`.
#[must_use]
pub fn autocomplete_answer(suggestions: &[RecipientSuggestion]) -> Value {
    json!({
        "suggestions": suggestions
            .iter()
            .map(|s| json!({ "name": s.name, "email": s.email, "kind": s.kind.as_str() }))
            .collect::<Vec<_>>(),
    })
}

/// Suggestions au plus par défaut (12).
pub const DEFAULT_LIMIT: usize = MAX_RECIPIENT_SUGGESTIONS;

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> Value {
        json!({
            "canBeCached": true,
            "match": [
                { "email": "\"Camille Martin\" <camille.martin@etudiant.univ-rennes.fr>", "type": "contact", "ranking": "14", "isGroup": false, "id": "1203", "l": "7", "first": "Camille", "last": "Martin", "fileas": "8:Martin Camille" },
                { "email": "<thomas.lemoine@univ-rennes.fr>", "type": "rankingTable", "ranking": "9", "isGroup": false },
                { "email": "\"Scolarité IUT\" <scolarite-iut-lannion@univ-rennes.fr>", "type": "gal", "ranking": "0", "isGroup": false, "full": "Scolarité IUT de Lannion" },
                { "email": "\"MMI 2\" <mmi2-lannion@listes.univ-rennes.fr>", "type": "gal", "isGroup": "1", "exp": true },
                { "type": "contact", "isGroup": true, "display": "Mon groupe TD", "id": "1300" },
                { "email": "CAMILLE.MARTIN@etudiant.univ-rennes.fr", "type": "gal" },
                { "email": "\"Injected\r\nBcc: x@y.fr\" <evil@x.fr>", "type": "gal" },
                { "email": "pas une adresse", "type": "contact" },
                { "email": "a@b.fr, c@d.fr", "type": "contact" },
                { "email": format!("{}@b.fr", "x".repeat(600)), "type": "gal" },
                { "email": "\"boss@univ-rennes.fr\" <someone@else.fr>", "type": "gal" },
                { "email": "\"Odd\" <odd@univ-rennes.fr>", "type": "somethingNew" },
            ],
        })
    }

    fn suggestion(name: &str, email: &str, kind: SuggestionKind) -> RecipientSuggestion {
        RecipientSuggestion {
            name: name.into(),
            email: email.into(),
            kind,
        }
    }

    #[test]
    fn requete_fixe_sauf_le_texte() {
        assert_eq!(
            autocomplete_request("cam"),
            json!({ "name": "cam", "t": "account", "includeGal": 1 })
        );
        assert_eq!(autocomplete_query(Some("thom")).as_deref(), Some("thom"));
        assert_eq!(autocomplete_query(Some("t")), None);
        assert_eq!(autocomplete_query(Some("***")), None);
        assert_eq!(autocomplete_query(None), None);
        assert_eq!(autocomplete_query(Some("l’é")).as_deref(), Some("l'é"));
    }

    #[test]
    fn nom_adresse_nature_et_rien_d_autre() {
        use SuggestionKind::{Contact, Directory, Group, Recent};
        assert_eq!(
            normalize_autocomplete(&response(), DEFAULT_LIMIT),
            [
                suggestion(
                    "Camille Martin",
                    "camille.martin@etudiant.univ-rennes.fr",
                    Contact
                ),
                suggestion("", "thomas.lemoine@univ-rennes.fr", Recent),
                suggestion(
                    "Scolarité IUT",
                    "scolarite-iut-lannion@univ-rennes.fr",
                    Directory
                ),
                suggestion("MMI 2", "mmi2-lannion@listes.univ-rennes.fr", Group),
                // Une fin de ligne (jamais une seule adresse), une liste, du bruit, une entrée
                // trop longue et un groupe de contacts sont laissés de côté ; un nom qui est une
                // adresse est retiré.
                suggestion("", "someone@else.fr", Directory),
                suggestion("Odd", "odd@univ-rennes.fr", Contact),
            ]
        );
        let answer = autocomplete_answer(&normalize_autocomplete(&response(), 2));
        assert_eq!(
            answer,
            json!({ "suggestions": [
                { "name": "Camille Martin", "email": "camille.martin@etudiant.univ-rennes.fr", "kind": "contact" },
                { "name": "", "email": "thomas.lemoine@univ-rennes.fr", "kind": "recent" },
            ] })
        );
    }

    #[test]
    fn nom_de_repli() {
        assert_eq!(
            normalize_autocomplete(
                &json!({ "match": { "email": "lea@univ.fr", "type": "gal", "full": "Léa Moreau" } }),
                DEFAULT_LIMIT
            ),
            [suggestion(
                "Léa Moreau",
                "lea@univ.fr",
                SuggestionKind::Directory
            )]
        );
        assert_eq!(
            normalize_autocomplete(
                &json!({ "match": [{ "email": "lea@univ.fr", "type": "contact", "first": "Léa", "last": "Moreau" }] }),
                DEFAULT_LIMIT
            ),
            [suggestion(
                "Léa Moreau",
                "lea@univ.fr",
                SuggestionKind::Contact
            )]
        );
        assert_eq!(
            normalize_autocomplete(
                &json!({ "match": [{ "email": "lea@univ.fr (Léa M.)", "display": "x" }] }),
                DEFAULT_LIMIT
            ),
            [suggestion("Léa M.", "lea@univ.fr", SuggestionKind::Contact)]
        );
    }

    #[test]
    fn reponse_vide_bizarre_ou_trop_grande() {
        assert!(normalize_autocomplete(&json!({}), DEFAULT_LIMIT).is_empty());
        assert!(normalize_autocomplete(&Value::Null, DEFAULT_LIMIT).is_empty());
        assert!(normalize_autocomplete(&json!({ "match": "x" }), DEFAULT_LIMIT).is_empty());
        let many: Vec<Value> = (0..500)
            .map(|i| json!({ "email": format!("etu{i}@univ.fr"), "type": "gal" }))
            .collect();
        let many = json!({ "match": many });
        assert_eq!(normalize_autocomplete(&many, DEFAULT_LIMIT).len(), 12);
        assert_eq!(normalize_autocomplete(&many, 5).len(), 5);
    }

    #[test]
    fn natures_inconnues_et_proto() {
        for kind in ["__proto__", "constructor", "toString", "hasOwnProperty"] {
            let list = normalize_autocomplete(
                &json!({ "match": [{ "email": "a@univ.fr", "type": kind }] }),
                DEFAULT_LIMIT,
            );
            assert_eq!(list[0].kind, SuggestionKind::Contact, "{kind}");
        }
        assert_eq!(
            SuggestionKind::from_written("__proto__"),
            SuggestionKind::Contact
        );
    }

    #[test]
    fn nom_pleine_chasse_et_controles() {
        let list = normalize_autocomplete(
            &json!({ "match": [
                { "email": "\"Boss＠univ.fr\" <a@univ.fr>" },
                { "email": "\"Ca\u{202e}mille\" <b@univ.fr>" },
            ] }),
            DEFAULT_LIMIT,
        );
        assert_eq!(list[0].name, "");
        assert_eq!(list[1].name, "Ca mille");
    }

    #[test]
    fn lecture_d_une_adresse_ecrite() {
        let one = |text: &str| -> Vec<(String, String, bool)> {
            parse_recipient_text(text)
                .into_iter()
                .map(|p| (p.name, p.email, p.invalid))
                .collect()
        };
        assert_eq!(
            one("\"Martin, Camille\" <c@univ.fr>"),
            [("Martin, Camille".into(), "c@univ.fr".into(), false)]
        );
        assert_eq!(
            one("mailto:c@univ.fr"),
            [(String::new(), "c@univ.fr".into(), false)]
        );
        assert_eq!(one("a@b.fr c@d.fr").len(), 2);
        assert_eq!(
            one("\"Esc \\\"aped\\\"\" <c@univ.fr>"),
            [("Esc aped".into(), "c@univ.fr".into(), false)]
        );
        assert!(one("x <y>")[0].2);
        assert!(one("  ").is_empty());
    }

    #[test]
    fn suggestions_nettoyees() {
        let list = clean_recipient_suggestions(
            &[
                SuggestionCandidate {
                    name: "A".into(),
                    email: " a@univ.fr ".into(),
                    kind: "recent".into(),
                },
                SuggestionCandidate {
                    name: "A bis".into(),
                    email: "A@UNIV.FR".into(),
                    kind: "group".into(),
                },
                SuggestionCandidate {
                    name: "x".into(),
                    email: "pas une adresse".into(),
                    kind: "contact".into(),
                },
                SuggestionCandidate {
                    name: "B".into(),
                    email: "b@univ.fr".into(),
                    kind: "evil".into(),
                },
            ],
            DEFAULT_LIMIT,
        );
        assert_eq!(
            list,
            [
                suggestion("A", "a@univ.fr", SuggestionKind::Recent),
                suggestion("B", "b@univ.fr", SuggestionKind::Contact),
            ]
        );
    }
}
