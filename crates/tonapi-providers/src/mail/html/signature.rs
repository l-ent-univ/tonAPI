//! Les signatures du webmail (`server/mailSignature.js` de Tafac) : le HTML de quelqu'un, jamais
//! envoyé ni montré tel que Zimbra le donne. Liste plus étroite que celle de l'éditeur (texte,
//! sauts de ligne, gras et compagnie, quelques styles, liens http(s) et `mailto:` ; ni image, ni
//! tableau, ni classe), bords vides retirés par un balayage (W2b).

use super::composer::{Piece, is_color, pieces, read_attribute, tidy_lines};
use super::sanitize::{Attributes, Policy, sanitize};
use crate::mail::text::{
    cap_chars, clean_text_lines, collapse_spaces, cut_utf16, decode_entities, escape_html,
    is_js_space, trim_js, utf16_len,
};

/// Au-delà, le HTML d'une signature cède la place à son texte (unités UTF-16).
pub const MAX_SIGNATURE_HTML_LENGTH: usize = 20_000;
/// Longueur maximale du texte d'une signature, en points de code.
pub const MAX_SIGNATURE_TEXT_LENGTH: usize = 4000;
/// Les balises d'une signature.
pub const SIGNATURE_TAGS: &[&str] = &[
    "div", "p", "br", "span", "b", "strong", "i", "em", "u", "s", "a", "ul", "ol", "li",
];

const STYLED_TAGS: &[&str] = &["div", "p", "span", "li", "b", "strong", "i", "em", "u", "s"];

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len()
        && text.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

fn safe_href(href: &str) -> bool {
    starts_with_ignore_case(href, "http://")
        || starts_with_ignore_case(href, "https://")
        || starts_with_ignore_case(href, "mailto:")
}

/// `^(?:[89]|[12]\d|3[0-2])(?:\.\d+)?(?:px|pt)$`.
fn is_font_size(value: &str) -> bool {
    let Some(number) = value
        .strip_suffix("px")
        .or_else(|| value.strip_suffix("pt"))
    else {
        return false;
    };
    let (whole, fraction) = match number.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (number, None),
    };
    let whole_ok = match whole.as_bytes() {
        [b'8' | b'9'] | [b'3', b'0'..=b'2'] => true,
        [b'1' | b'2', d] => d.is_ascii_digit(),
        _ => false,
    };
    whole_ok
        && fraction.is_none_or(|fraction| {
            !fraction.is_empty() && fraction.bytes().all(|b| b.is_ascii_digit())
        })
}

fn inline_style_allowed(prop: &str, value: &str) -> bool {
    match prop {
        "color" => is_color(value, true),
        "font-weight" => {
            matches!(value, "bold" | "bolder" | "normal")
                || (value.len() == 3
                    && value.ends_with("00")
                    && (b'1'..=b'9').contains(&value.as_bytes()[0]))
        }
        "font-style" => matches!(value, "italic" | "normal"),
        "text-decoration" => matches!(value, "underline" | "line-through" | "none"),
        "font-size" => is_font_size(value),
        _ => false,
    }
}

struct SignaturePolicy;

impl Policy for SignaturePolicy {
    fn transform(&mut self, name: &str, attributes: Attributes) -> (String, Attributes) {
        match name {
            "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th" | "center"
            | "blockquote" | "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "section"
            | "header" | "footer" | "address" => ("div".to_owned(), attributes),
            "font" => {
                let mut span = Attributes::default();
                let color = attributes.text("color");
                if is_color(color, true) {
                    span.set("style", format!("color:{color}"));
                }
                ("span".to_owned(), span)
            }
            "a" => {
                let mut link = Attributes::default();
                let href = trim_js(attributes.text("href"));
                if safe_href(href) {
                    link.set("href", href);
                }
                ("a".to_owned(), link)
            }
            _ => (name.to_owned(), attributes),
        }
    }

    fn tag_allowed(&self, name: &str) -> bool {
        SIGNATURE_TAGS.contains(&name)
    }

    fn non_text(&self, name: &str) -> bool {
        matches!(
            name,
            "script"
                | "style"
                | "textarea"
                | "option"
                | "noscript"
                | "title"
                | "head"
                | "template"
                | "svg"
                | "math"
                | "iframe"
                | "object"
                | "select"
                | "button"
        )
    }

    fn attribute_allowed(&self, tag: &str, attribute: &str) -> bool {
        (tag == "a" && attribute == "href") || (attribute == "style" && STYLED_TAGS.contains(&tag))
    }

    fn scheme_checked(&self, attribute: &str) -> bool {
        attribute == "href"
    }

    fn allowed_schemes(&self, _tag: &str) -> &'static [&'static str] {
        &["http", "https", "mailto"]
    }

    fn allow_protocol_relative(&self) -> bool {
        false
    }

    fn style_allowed(&self, tag: &str, prop: &str, value: &str) -> Option<bool> {
        STYLED_TAGS
            .contains(&tag)
            .then(|| inline_style_allowed(prop, value))
    }
}

// ── Bords vides ───────────────────────────────────────────────────────────

/// Une espace JavaScript, `&nbsp;` ou `<br />` au début de `text[at..]` : sa longueur.
fn blank_at(text: &str, at: usize) -> usize {
    let rest = &text[at..];
    if let Some(c) = rest.chars().next()
        && is_js_space(c)
    {
        return c.len_utf8();
    }
    if rest.starts_with("&nbsp;") || rest.starts_with("<br />") {
        return 6;
    }
    0
}

/// La même chose finissant à `end`.
fn blank_before(text: &str, end: usize) -> usize {
    let before = &text[..end];
    if let Some(c) = before.chars().next_back()
        && is_js_space(c)
    {
        return c.len_utf8();
    }
    if before.ends_with("&nbsp;") || before.ends_with("<br />") {
        return 6;
    }
    0
}

/// Une unité vide au début : un blanc, ou `<div>`/`<p>` ne contenant que des blancs.
fn edge_unit_at(text: &str, at: usize) -> usize {
    let blank = blank_at(text, at);
    if blank > 0 {
        return blank;
    }
    for (open, close) in [("<div>", "</div>"), ("<p>", "</p>")] {
        if text[at..].starts_with(open) {
            let mut index = at + open.len();
            loop {
                let length = blank_at(text, index);
                if length == 0 {
                    break;
                }
                index += length;
            }
            if text[index..].starts_with(close) {
                return index + close.len() - at;
            }
        }
    }
    0
}

/// La même chose finissant à `end`.
fn edge_unit_before(text: &str, end: usize) -> usize {
    let blank = blank_before(text, end);
    if blank > 0 {
        return blank;
    }
    for (open, close) in [("<div>", "</div>"), ("<p>", "</p>")] {
        if text[..end].ends_with(close) {
            let mut index = end - close.len();
            loop {
                let length = blank_before(text, index);
                if length == 0 {
                    break;
                }
                index -= length;
            }
            if text[..index].ends_with(open) {
                return end - (index - open.len());
            }
        }
    }
    0
}

/// `EDGE_BLANKS` sans expression : les lignes vides du début et de la fin retirées, en temps
/// linéaire (W2b).
fn trim_edge_blanks(text: &str) -> &str {
    let mut start = 0;
    loop {
        let length = edge_unit_at(text, start);
        if length == 0 {
            break;
        }
        start += length;
    }
    if start == text.len() {
        return "";
    }
    let mut end = text.len();
    while end > start {
        let length = edge_unit_before(text, end);
        if length == 0 || end - length < start {
            break;
        }
        end -= length;
    }
    &text[start..end]
}

/// Le HTML d'une signature nettoyé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureHtml {
    /// Le HTML gardé.
    pub html: String,
    /// Elle avait des images, retirées (la page le dit).
    pub has_images: bool,
}

/// `sanitizeSignatureHtml` : le HTML d'une signature du webmail, coupé d'abord à 100 000 unités
/// UTF-16, nettoyé, ses bords vides retirés.
#[must_use]
pub fn sanitize_signature_html(html: &str) -> SignatureHtml {
    let source = cut_utf16(html, MAX_SIGNATURE_HTML_LENGTH * 5);
    let clean = sanitize(source, &mut SignaturePolicy);
    let bytes = source.as_bytes();
    let has_images = (0..bytes.len()).any(|at| {
        bytes[at] == b'<'
            && bytes
                .get(at + 1..at + 4)
                .is_some_and(|name| name.eq_ignore_ascii_case(b"img"))
            && !bytes
                .get(at + 4)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
    });
    SignatureHtml {
        html: trim_edge_blanks(&clean).to_owned(),
        has_images,
    }
}

/// `signatureHtmlToText` : le HTML de `sanitize_signature_html` en texte. Un bloc ou un `<br>`
/// par ligne, un élément de liste « - », un lien « texte (adresse) » sauf si son texte est
/// l'adresse.
#[must_use]
pub fn signature_html_to_text(clean_html: &str) -> String {
    let mut out = String::new();
    let mut link: Option<(String, String)> = None;
    for piece in pieces(clean_html, false) {
        match piece {
            Piece::Text(raw) => {
                let text = collapse_spaces(&decode_entities(raw));
                out.push_str(&text);
                if let Some((_, link_text)) = link.as_mut() {
                    link_text.push_str(&text);
                }
            }
            Piece::Tag {
                closing,
                name,
                attributes,
            } => match name.as_str() {
                "br" => out.push('\n'),
                "a" => {
                    if !closing {
                        let href =
                            decode_entities(read_attribute(attributes, "href").unwrap_or_default());
                        link = Some((href, String::new()));
                    } else if let Some((href, text)) = link.take() {
                        let target = if starts_with_ignore_case(&href, "mailto:") {
                            &href[7..]
                        } else {
                            href.as_str()
                        };
                        let shown = trim_js(&text);
                        if !href.is_empty() && target != shown && href != shown {
                            out.push_str(" (");
                            out.push_str(target);
                            out.push(')');
                        }
                    }
                }
                "div" | "p" | "ul" | "ol" | "li" => {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    if name == "li" && !closing {
                        out.push_str("- ");
                    }
                }
                _ => {}
            },
        }
    }
    let lines: Vec<String> = out
        .split('\n')
        .map(|line| trim_js(&collapse_spaces(line)).to_owned())
        .collect();
    tidy_lines(&lines.join("\n"))
}

/// `plainSignatureToHtml` : une signature en texte, une ligne par `<div>`, une ligne vide gardée.
#[must_use]
pub fn plain_signature_to_html(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            if line.is_empty() {
                "<div><br /></div>".to_owned()
            } else {
                format!("<div>{}</div>", escape_html(line))
            }
        })
        .collect()
}

fn clean_signature_text(text: &str) -> String {
    cap_chars(&clean_text_lines(text), MAX_SIGNATURE_TEXT_LENGTH).to_owned()
}

/// Le contenu d'une signature (`readSignatureContent`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SignatureContent {
    /// Son HTML nettoyé.
    pub html: String,
    /// Son texte.
    pub text: String,
    /// Elle avait des images, retirées.
    pub has_images: bool,
}

/// `readSignatureContent` : le contenu d'une signature (HTML et/ou texte, comme le webmail les
/// garde). Le HTML nettoyé et son texte ; un HTML de plus de 20 000 unités ou sans texte cède la
/// place au texte (nettoyé, 4000 points de code au plus).
#[must_use]
pub fn read_signature_content(html: Option<&str>, text: Option<&str>) -> SignatureContent {
    if let Some(html) = html
        && !trim_js(html).is_empty()
    {
        let clean = sanitize_signature_html(html);
        let from_html = clean_signature_text(&signature_html_to_text(&clean.html));
        if utf16_len(&clean.html) <= MAX_SIGNATURE_HTML_LENGTH && !from_html.is_empty() {
            return SignatureContent {
                html: clean.html,
                text: from_html,
                has_images: clean.has_images,
            };
        }
        let fallback = match text {
            Some(text) if !trim_js(text).is_empty() => clean_signature_text(text),
            _ => from_html,
        };
        return SignatureContent {
            html: plain_signature_to_html(&fallback),
            text: fallback,
            has_images: clean.has_images,
        };
    }
    let plain = clean_signature_text(text.unwrap_or_default());
    SignatureContent {
        html: if plain.is_empty() {
            String::new()
        } else {
            plain_signature_to_html(&plain)
        },
        text: plain,
        has_images: false,
    }
}

struct FramePolicy;

impl Policy for FramePolicy {
    fn transform(&mut self, name: &str, mut attributes: Attributes) -> (String, Attributes) {
        let base = match name {
            "p" => "margin:0",
            "ul" | "ol" => "margin:0.25em 0;padding-left:1.5em",
            "a" => "color:#1a73e8",
            _ => return (name.to_owned(), attributes),
        };
        let style = match attributes.get("style").filter(|style| !style.is_empty()) {
            Some(own) => format!("{base};{own}"),
            None => base.to_owned(),
        };
        attributes.set("style", style);
        (name.to_owned(), attributes)
    }

    fn tag_allowed(&self, name: &str) -> bool {
        SIGNATURE_TAGS.contains(&name)
    }

    fn non_text(&self, name: &str) -> bool {
        matches!(name, "script" | "style" | "textarea" | "option" | "xmp")
    }

    fn attribute_allowed(&self, _tag: &str, _attribute: &str) -> bool {
        true
    }
}

/// Une signature prête pour le mail.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FramedSignature {
    /// Son HTML dans son cadre, styles en ligne.
    pub html: String,
    /// Son texte, après « -- » s'il n'en commence pas par un.
    pub text: String,
}

/// `/^--\s*(?:\n|$)/` : le texte commence par « -- » seul sur sa ligne.
fn starts_with_dashes(text: &str) -> bool {
    let Some(rest) = text.strip_prefix("--") else {
        return false;
    };
    let spaces: String = rest.chars().take_while(|c| is_js_space(*c)).collect();
    spaces.contains('\n') || spaces.len() == rest.len()
}

/// `frameSignature` : une signature (de `read_signature_content`) pour le mail. Son HTML (ou son
/// texte en HTML) dans son cadre, styles en ligne (paragraphes, listes, liens) ; son texte après
/// « -- » (la ligne à laquelle les clients reconnaissent une signature). La passe garde tous les
/// attributs : `html` doit venir de `sanitize_signature_html`.
#[must_use]
pub fn frame_signature(html: &str, text: &str) -> FramedSignature {
    if html.is_empty() && text.is_empty() {
        return FramedSignature::default();
    }
    let source = if html.is_empty() {
        plain_signature_to_html(text)
    } else {
        html.to_owned()
    };
    let styled = sanitize(&source, &mut FramePolicy);
    FramedSignature {
        html: format!(
            "<div style=\"margin:14px 0 6px;font-size:14px;line-height:1.45;color:#1d1d1f\">{styled}</div>"
        ),
        text: if starts_with_dashes(text) {
            text.to_owned()
        } else {
            format!("-- \n{text}")
        },
    }
}
