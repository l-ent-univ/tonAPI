//! Le HTML de l'éditeur (`server/mailSend.js` de Tafac) : à l'envoi, au brouillon et quand un
//! brouillon revient dans l'éditeur (c'est le seul HTML qui entre dans la page principale, par
//! TipTap).
//!
//! `sanitize_composer_html` est la passe stricte. Les suivantes (`select_inline_images`,
//! `inline_composer_styles`, `build_mail_html`) gardent tous les attributs : elles ne sont sûres
//! que sur sa sortie, et doivent la suivre. `read_draft_images` la précède (elle est suivie
//! d'elle dans `zimbra::compose`).

use serde_json::Value;

use super::sanitize::{Attributes, Exclude, Policy, sanitize};
use crate::mail::text::{
    cap_chars, collapse_spaces, cut_utf16, decode_entities, escape_html, is_js_space,
    replace_controls, trim_js,
};

/// Longueur maximale du HTML de l'éditeur, en unités UTF-16 (un long fil cité compris).
pub const MAX_COMPOSER_HTML_LENGTH: usize = 1_500_000;
const MAX_IMAGE_ALT_LENGTH: usize = 200;

/// La place de la signature dans le texte de l'éditeur.
pub const SIGNATURE_PLACEHOLDER: &str = "<div data-mail-signature></div>";

const COMPOSER_TAGS: &[&str] = &[
    "p",
    "br",
    "strong",
    "b",
    "em",
    "i",
    "u",
    "s",
    "ul",
    "ol",
    "li",
    "h2",
    "h3",
    "blockquote",
    "a",
    "span",
    "mark",
    "div",
    "table",
    "tbody",
    "tr",
    "th",
    "td",
    "img",
];

const DEFAULT_FONT: &str =
    "-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif";
const HIGHLIGHT_TEXT: &str = "#1f1f1f";

fn element_style(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "p" | "li" => "margin:0",
        "h2" => "margin:0.2em 0;font-size:1.5em;font-weight:700;line-height:1.25",
        "h3" => "margin:0.2em 0;font-size:1.2em;font-weight:600;line-height:1.3",
        "ul" | "ol" => "margin:0.25em 0;padding-left:1.5em",
        "blockquote" => {
            "margin:0.4em 0 0;padding:0 0 0 12px;border-left:2px solid #d9d2cc;color:#6b625b"
        }
        "a" => "color:#1a73e8",
        "table" => {
            "margin:4px 0 12px;padding:6px 0;border-collapse:separate;border-spacing:0;border:1px solid #e8e1da;border-radius:12px;background:#f8f5f1;font-size:14px;line-height:1.45"
        }
        "th" => {
            "padding:4px 18px 4px 14px;text-align:left;vertical-align:top;font-weight:500;color:#6b625b;white-space:nowrap"
        }
        "td" => {
            "padding:4px 14px 4px 0;text-align:left;vertical-align:top;font-weight:600;color:#1d1d1f"
        }
        _ => return None,
    })
}

const WRAPPER_STYLE: &str = "font-family:-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif;font-size:16px;line-height:1.5;color:#1d1d1f";

// ── Valeurs permises ──────────────────────────────────────────────────────

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len()
        && text.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

fn is_ascii_digits(text: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit())
}

/// `INLINE_CID_PATTERN` : `^tafac-img-[0-9a-f-]{8,64}@tafac\.fr$`, la référence d'une image du
/// texte que la page crée.
#[must_use]
pub fn is_inline_cid(cid: &str) -> bool {
    cid.strip_prefix("tafac-img-")
        .and_then(|rest| rest.strip_suffix("@tafac.fr"))
        .is_some_and(|middle| {
            (8..=64).contains(&middle.len())
                && middle
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
        })
}

/// Avance sur les espaces JavaScript.
fn skip_spaces(text: &str, mut index: usize) -> usize {
    while let Some(c) = text.get(index..).and_then(|rest| rest.chars().next()) {
        if !is_js_space(c) {
            break;
        }
        index += c.len_utf8();
    }
    index
}

/// `^\d{1,3}` puis les espaces : la fin, ou `None`.
fn read_byte_number(text: &str, index: usize) -> Option<usize> {
    let digits = text
        .get(index..)?
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    (1..=3).contains(&digits).then_some(index + digits)
}

/// `COLOR` : `#` et 3 à 8 chiffres hexadécimaux, `rgb()`/`rgba()` (et, pour les signatures,
/// un nom `[a-z]{3,20}`), sans tenir compte de la casse.
pub(crate) fn is_color(value: &str, names: bool) -> bool {
    if let Some(hex) = value.strip_prefix('#') {
        return (3..=8).contains(&hex.len()) && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    if names && (3..=20).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_alphabetic()) {
        return true;
    }
    let mut index = if starts_with_ignore_case(value, "rgba(") {
        5
    } else if starts_with_ignore_case(value, "rgb(") {
        4
    } else {
        return false;
    };
    for position in 0..3 {
        index = skip_spaces(value, index);
        let Some(end) = read_byte_number(value, index) else {
            return false;
        };
        index = skip_spaces(value, end);
        if position < 2 {
            if value.as_bytes().get(index) != Some(&b',') {
                return false;
            }
            index += 1;
        }
    }
    if value.as_bytes().get(index) == Some(&b',') {
        index = skip_spaces(value, index + 1);
        let bytes = value.as_bytes();
        let start = index;
        if bytes.get(index) == Some(&b'0') {
            index += 1;
        }
        if bytes.get(index) == Some(&b'.') && bytes.get(index + 1).is_some_and(u8::is_ascii_digit) {
            index += 1;
            while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
            }
        } else if index == start {
            if bytes.get(index) == Some(&b'1') {
                index += 1;
            } else {
                return false;
            }
        }
        index = skip_spaces(value, index);
    }
    index + 1 == value.len() && value.ends_with(')')
}

fn is_font_family(value: &str) -> bool {
    if matches!(value, "Georgia, serif" | "ui-monospace, Menlo, monospace") {
        return true;
    }
    let rest = value.strip_prefix('"').unwrap_or(value);
    let Some(rest) = rest
        .strip_prefix("Collab Sans")
        .or_else(|| rest.strip_prefix("Inter Variable"))
    else {
        return false;
    };
    let rest = rest.strip_prefix('"').unwrap_or(rest);
    rest == ", system-ui, sans-serif"
}

fn composer_style_allowed(tag: &str, prop: &str, value: &str) -> Option<bool> {
    Some(match tag {
        "span" => match prop {
            "color" | "background-color" => is_color(value, false),
            "font-size" => matches!(value, "10px" | "13px" | "16px" | "18px" | "24px" | "32px"),
            "font-family" => is_font_family(value),
            _ => false,
        },
        "mark" => match prop {
            "background-color" => is_color(value, false),
            "color" => value == "inherit",
            _ => false,
        },
        "p" | "h2" | "h3" => {
            prop == "margin-left"
                && matches!(
                    value,
                    "32px" | "64px" | "96px" | "128px" | "160px" | "192px"
                )
        }
        _ => return None,
    })
}

/// `cleanImageAttributes` : une image du texte n'est qu'une référence à une partie du mail.
/// Le texte de `alt` est décodé une fois, par la lecture (W15 : Node le décodait deux fois).
fn clean_image_attributes(attributes: &Attributes) -> Attributes {
    let cid = trim_js(attributes.text("data-inline-cid")).to_lowercase();
    let mut clean = Attributes::default();
    if !is_inline_cid(&cid) {
        return clean;
    }
    let alt = collapse_spaces(&replace_controls(attributes.text("alt"), Some(' ')));
    let alt = trim_js(cap_chars(trim_js(&alt), MAX_IMAGE_ALT_LENGTH));
    clean.set("data-inline-cid", cid);
    if !alt.is_empty() {
        clean.set("alt", alt);
    }
    for size in ["width", "height"] {
        if let Some(value) = attributes.get(size)
            && is_ascii_digits(value, 1, 4)
        {
            clean.set(size, value);
        }
    }
    clean
}

// ── La passe stricte ──────────────────────────────────────────────────────

struct ComposerPolicy;

impl Policy for ComposerPolicy {
    fn transform(&mut self, name: &str, attributes: Attributes) -> (String, Attributes) {
        let mut clean = Attributes::default();
        match name {
            "div" if attributes.has("data-mail-signature") => clean.set("data-mail-signature", ""),
            "table" if attributes.has("data-mail-summary") => clean.set("data-mail-summary", ""),
            "div" | "table" => {}
            "img" => clean = clean_image_attributes(&attributes),
            "ol" => {
                if let Some(start) = attributes.get("start")
                    && is_ascii_digits(start, 1, 4)
                    && start != "1"
                {
                    clean.set("start", start);
                }
            }
            "a" => {
                let href = trim_js(attributes.text("href"));
                if starts_with_ignore_case(href, "http://")
                    || starts_with_ignore_case(href, "https://")
                    || starts_with_ignore_case(href, "mailto:")
                {
                    clean.set("href", href);
                }
            }
            _ => clean = attributes,
        }
        (name.to_owned(), clean)
    }

    fn tag_allowed(&self, name: &str) -> bool {
        COMPOSER_TAGS.contains(&name)
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
        )
    }

    fn attribute_allowed(&self, tag: &str, attribute: &str) -> bool {
        match tag {
            "a" => attribute == "href",
            "span" | "mark" | "p" | "h2" | "h3" => attribute == "style",
            "ol" => attribute == "start",
            "div" => attribute == "data-mail-signature",
            "table" => attribute == "data-mail-summary",
            "img" => matches!(attribute, "data-inline-cid" | "alt" | "width" | "height"),
            _ => false,
        }
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
        composer_style_allowed(tag, prop, value)
    }

    fn exclude(&self, tag: &str, attributes: &Attributes) -> Exclude {
        match tag {
            "div" if attributes.has("data-mail-signature") => Exclude::IfText,
            "img" if attributes.text("data-inline-cid").is_empty() => Exclude::Always,
            _ => Exclude::Never,
        }
    }
}

/// `sanitizeComposerHtml` : le HTML de l'éditeur réduit à ce que l'éditeur écrit (paragraphes,
/// titres 2 et 3, listes, citations, liens http(s) et `mailto:`, gras et compagnie, couleurs,
/// tailles et polices du menu, retraits, tableau de résumé, images par référence), coupé à
/// 1 500 000 unités UTF-16. Le premier emplacement de signature vide reste, les autres `div`
/// cèdent la place à leur contenu.
#[must_use]
pub fn sanitize_composer_html(html: &str) -> String {
    const TOKEN: &str = "<tafac-signature>";
    let clean = sanitize(
        cut_utf16(html, MAX_COMPOSER_HTML_LENGTH),
        &mut ComposerPolicy,
    );
    let first = [
        SIGNATURE_PLACEHOLDER,
        "<div data-mail-signature=\"\"></div>",
    ]
    .iter()
    .filter_map(|pattern| clean.find(pattern).map(|at| (at, pattern.len())))
    .min();
    let clean = match first {
        Some((at, length)) => format!("{}{TOKEN}{}", &clean[..at], &clean[at + length..]),
        None => clean,
    };
    clean
        .replace("<div data-mail-signature=\"\">", "")
        .replace("<div data-mail-signature>", "")
        .replace("<div>", "")
        .replace("</div>", "")
        .replacen(TOKEN, SIGNATURE_PLACEHOLDER, 1)
}

// ── Les passes qui suivent (tous attributs permis) ───────────────────────

/// Une passe qui garde tous les attributs, sur les balises de l'éditeur.
trait Permissive {
    fn transform(&mut self, name: &str, attributes: Attributes) -> Attributes;
    fn exclude(&self, tag: &str, attributes: &Attributes) -> Exclude;
    fn image_schemes(&self) -> &'static [&'static str] {
        &["http", "https", "ftp", "mailto", "tel"]
    }
}

struct Pass<'a, T: Permissive>(&'a mut T);

impl<T: Permissive> Policy for Pass<'_, T> {
    fn transform(&mut self, name: &str, attributes: Attributes) -> (String, Attributes) {
        (name.to_owned(), self.0.transform(name, attributes))
    }
    fn tag_allowed(&self, name: &str) -> bool {
        COMPOSER_TAGS.contains(&name)
    }
    fn non_text(&self, name: &str) -> bool {
        matches!(name, "script" | "style" | "textarea" | "option" | "xmp")
    }
    fn attribute_allowed(&self, _tag: &str, _attribute: &str) -> bool {
        true
    }
    fn allowed_schemes(&self, tag: &str) -> &'static [&'static str] {
        if tag == "img" {
            self.0.image_schemes()
        } else {
            &["http", "https", "ftp", "mailto", "tel"]
        }
    }
    fn exclude(&self, tag: &str, attributes: &Attributes) -> Exclude {
        self.0.exclude(tag, attributes)
    }
}

struct SelectImages<'a> {
    references: &'a [Value],
    images: Vec<Value>,
}

impl Permissive for SelectImages<'_> {
    fn transform(&mut self, name: &str, attributes: Attributes) -> Attributes {
        if name != "img" {
            return attributes;
        }
        let cid = attributes.get("data-inline-cid");
        let reference = cid.and_then(|cid| {
            self.references
                .iter()
                .rev()
                .find(|entry| entry.get("cid").and_then(Value::as_str) == Some(cid))
        });
        let Some(reference) = reference else {
            return Attributes::default();
        };
        let reference_cid = reference.get("cid");
        if self
            .images
            .iter()
            .any(|image| image.get("cid") == reference_cid)
        {
            return Attributes::default();
        }
        let mut image = reference.clone();
        if let Value::Object(map) = &mut image {
            map.insert(
                "alt".to_owned(),
                Value::String(attributes.text("alt").to_owned()),
            );
        }
        self.images.push(image);
        attributes
    }

    fn exclude(&self, tag: &str, attributes: &Attributes) -> Exclude {
        if tag == "img" && attributes.text("data-inline-cid").is_empty() {
            Exclude::Always
        } else {
            Exclude::Never
        }
    }
}

/// `selectInlineImages` : les images que le mail prend, d'après le HTML de
/// `sanitize_composer_html` et les références de la requête (objets JSON avec leur `cid`). Une
/// image dont le `cid` n'est pas listé part, la seconde d'un même `cid` aussi. Rend le HTML et les
/// références gardées, dans l'ordre du texte, chacune avec son `alt` (pris dans le texte).
#[must_use]
pub fn select_inline_images(clean_html: &str, references: &[Value]) -> (String, Vec<Value>) {
    let mut select = SelectImages {
        references,
        images: Vec::new(),
    };
    let html = sanitize(clean_html, &mut Pass(&mut select));
    (html, select.images)
}

/// Une image du texte telle que le mail la montre : son `src` (`cid:…`, ou une URL `data:` pour
/// une copie qui ne part pas), sa taille et son texte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineImageSource {
    /// Le Content-ID de la référence (`data-inline-cid`).
    pub cid: String,
    /// `cid:…` ou `data:…`.
    pub src: String,
    /// Largeur affichée, en pixels.
    pub width: u32,
    /// Hauteur affichée, en pixels.
    pub height: u32,
    /// Texte de remplacement.
    pub alt: String,
}

struct InlineStyles<'a> {
    images: &'a [InlineImageSource],
}

/// `/font-family:\s*"?(?:Collab Sans|Inter Variable)"?, system-ui, sans-serif/` : la première,
/// remplacée par la police par défaut.
fn replace_editor_font(style: &str) -> String {
    let mut from = 0;
    while let Some(offset) = style[from..].find("font-family:") {
        let start = from + offset;
        let mut index = skip_spaces(style, start + "font-family:".len());
        if style.as_bytes().get(index) == Some(&b'"') {
            index += 1;
        }
        let rest = &style[index..];
        let name = ["Collab Sans", "Inter Variable"]
            .into_iter()
            .find(|name| rest.starts_with(name));
        if let Some(name) = name {
            index += name.len();
            if style.as_bytes().get(index) == Some(&b'"') {
                index += 1;
            }
            let tail = ", system-ui, sans-serif";
            if style[index..].starts_with(tail) {
                return format!(
                    "{}font-family:{DEFAULT_FONT}{}",
                    &style[..start],
                    &style[index + tail.len()..]
                );
            }
        }
        from = start + 1;
    }
    style.to_owned()
}

/// `/;?\s*color:\s*inherit/` : la première, retirée.
fn drop_inherit_color(style: &str) -> String {
    let mut from = 0;
    while let Some(offset) = style[from..].find("color:") {
        let at = from + offset;
        let after = skip_spaces(style, at + "color:".len());
        if style[after..].starts_with("inherit") {
            let mut start = at;
            while let Some(c) = style[..start].chars().next_back() {
                if !is_js_space(c) {
                    break;
                }
                start -= c.len_utf8();
            }
            if style[..start].ends_with(';') {
                start -= 1;
            }
            return format!("{}{}", &style[..start], &style[after + "inherit".len()..]);
        }
        from = at + 1;
    }
    style.to_owned()
}

/// `/margin-left:\s*(\d+)px/` : les chiffres du premier retrait.
fn read_indent(style: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(offset) = style[from..].find("margin-left:") {
        let at = from + offset;
        let start = skip_spaces(style, at + "margin-left:".len());
        let digits = style[start..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        if digits > 0 && style[start + digits..].starts_with("px") {
            return Some(&style[start..start + digits]);
        }
        from = at + 1;
    }
    None
}

impl Permissive for InlineStyles<'_> {
    fn transform(&mut self, name: &str, mut attributes: Attributes) -> Attributes {
        if name == "img" {
            let cid = attributes.text("data-inline-cid");
            let mut tag = Attributes::default();
            if let Some(image) = self.images.iter().rev().find(|image| image.cid == cid) {
                tag.set("src", image.src.clone());
                tag.set("width", image.width.to_string());
                tag.set("height", image.height.to_string());
                tag.set("alt", image.alt.clone());
                tag.set(
                    "style",
                    format!(
                        "display:block;width:{}px;max-width:100%;height:auto;margin:0 0 4px;border:0;outline:none;border-radius:16px",
                        image.width
                    ),
                );
            }
            return tag;
        }
        if name == "table" {
            let mut table = Attributes::default();
            table.set("cellpadding", "0");
            table.set("cellspacing", "0");
            table.set("style", element_style("table").unwrap_or_default());
            return table;
        }
        if name == "span" && !attributes.text("style").is_empty() {
            let mut span = Attributes::default();
            span.set("style", replace_editor_font(attributes.text("style")));
            return span;
        }
        if name == "mark" {
            let own = drop_inherit_color(attributes.text("style"));
            let style = if own.is_empty() {
                format!("color:{HIGHLIGHT_TEXT}")
            } else {
                format!("{own};color:{HIGHLIGHT_TEXT}")
            };
            let mut mark = Attributes::default();
            mark.set("style", style);
            return mark;
        }
        let Some(base) = element_style(name) else {
            return attributes;
        };
        let style = match read_indent(attributes.text("style")) {
            Some(indent) if matches!(name, "p" | "h2" | "h3") => {
                format!("{base};margin-left:{indent}px")
            }
            _ => base.to_owned(),
        };
        attributes.set("style", style);
        attributes
    }

    fn exclude(&self, tag: &str, attributes: &Attributes) -> Exclude {
        if tag == "img" && attributes.text("src").is_empty() {
            Exclude::Always
        } else {
            Exclude::Never
        }
    }

    fn image_schemes(&self) -> &'static [&'static str] {
        &["cid", "data"]
    }
}

/// `inlineComposerStyles` : le HTML de `sanitize_composer_html` avec ses styles en ligne (les
/// clients de mail lisent mal les `<style>`) ; un paragraphe vide garde sa hauteur (`<br />`) ;
/// la police de l'éditeur devient celle du système. Une image absente de `images` part.
#[must_use]
pub fn inline_composer_styles(clean_html: &str, images: &[InlineImageSource]) -> String {
    let styled = sanitize(clean_html, &mut Pass(&mut InlineStyles { images }));
    // `<p style="…"></p>` → `<p style="…"><br /></p>`.
    let mut out = String::with_capacity(styled.len());
    let mut rest = styled.as_str();
    while let Some(at) = rest.find("<p style=\"") {
        let after = at + "<p style=\"".len();
        let Some(quote) = rest[after..].find('"') else {
            break;
        };
        let end = after + quote;
        if rest[end..].starts_with("\"></p>") {
            out.push_str(&rest[..end]);
            out.push_str("\"><br /></p>");
            rest = &rest[end + "\"></p>".len()..];
        } else {
            out.push_str(&rest[..after]);
            rest = &rest[after..];
        }
    }
    out.push_str(rest);
    out
}

/// `buildMailHtml` : la partie HTML du mail, le texte dans son cadre, la signature à la place de
/// son emplacement (insérée telle quelle, sans motif de remplacement). Un brouillon garde
/// l'emplacement (`keep_placeholder`).
#[must_use]
pub fn build_mail_html(
    clean_html: &str,
    signature_html: &str,
    keep_placeholder: bool,
    images: &[InlineImageSource],
) -> String {
    let styled = inline_composer_styles(clean_html, images);
    let body = if keep_placeholder {
        styled
    } else {
        let first = [
            SIGNATURE_PLACEHOLDER,
            "<div data-mail-signature=\"\"></div>",
        ]
        .iter()
        .filter_map(|pattern| styled.find(pattern).map(|at| (at, pattern.len())))
        .min();
        match first {
            Some((at, length)) => format!(
                "{}{signature_html}{}",
                &styled[..at],
                &styled[at + length..]
            ),
            None => styled,
        }
    };
    format!(
        "<html><head><meta charset=\"utf-8\"></head><body><div style=\"{WRAPPER_STYLE}\">{body}</div></body></html>"
    )
}

// ── Le texte ──────────────────────────────────────────────────────────────

/// Un morceau du HTML nettoyé, tel que l'expression de Tafac le découpe.
pub(crate) enum Piece<'a> {
    Tag {
        closing: bool,
        name: String,
        attributes: &'a str,
    },
    Text(&'a str),
}

/// `/<(\/?)([a-z0-9-]+)((?:\s+[a-z-]+(?:="[^"]*")?)*)\s*\/?>|([^<]+)/gi` (sans `-` dans le nom
/// pour les signatures) : un `<` qui n'ouvre pas une balise est sauté.
pub(crate) fn pieces(html: &str, dash_in_name: bool) -> Vec<Piece<'_>> {
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'<' {
            let end = html[index..].find('<').map_or(html.len(), |at| index + at);
            out.push(Piece::Text(&html[index..end]));
            index = end;
            continue;
        }
        match read_tag(html, index, dash_in_name) {
            Some((piece, end)) => {
                out.push(piece);
                index = end;
            }
            None => index += 1,
        }
    }
    out
}

fn read_tag(html: &str, at: usize, dash_in_name: bool) -> Option<(Piece<'_>, usize)> {
    let bytes = html.as_bytes();
    let mut index = at + 1;
    let closing = bytes.get(index) == Some(&b'/');
    if closing {
        index += 1;
    }
    let name_start = index;
    while bytes
        .get(index)
        .is_some_and(|b| b.is_ascii_alphanumeric() || (dash_in_name && *b == b'-'))
    {
        index += 1;
    }
    if index == name_start {
        return None;
    }
    let name = html[name_start..index].to_ascii_lowercase();
    let attributes_start = index;
    loop {
        let spaced = skip_spaces(html, index);
        if spaced == index {
            break;
        }
        let name_end = spaced
            + bytes[spaced..]
                .iter()
                .take_while(|b| b.is_ascii_alphabetic() || **b == b'-')
                .count();
        if name_end == spaced {
            break;
        }
        let mut end = name_end;
        if bytes.get(end..end + 2) == Some(b"=\"")
            && let Some(close) = html[end + 2..].find('"')
        {
            end = end + 2 + close + 1;
        }
        index = end;
    }
    let attributes = &html[attributes_start..index];
    let mut end = skip_spaces(html, index);
    if bytes.get(end) == Some(&b'/') {
        end += 1;
    }
    if bytes.get(end) != Some(&b'>') {
        return None;
    }
    Some((
        Piece::Tag {
            closing,
            name,
            attributes,
        },
        end + 1,
    ))
}

/// `/name="([^"]*)"/.exec(attributes)` : la valeur du premier attribut de ce nom.
pub(crate) fn read_attribute<'a>(attributes: &'a str, name: &str) -> Option<&'a str> {
    let pattern = format!("{name}=\"");
    let at = attributes.find(&pattern)? + pattern.len();
    let end = attributes[at..].find('"')?;
    Some(&attributes[at..at + end])
}

/// `\s+$` retiré.
fn trim_end_js(text: &str) -> &str {
    text.trim_end_matches(is_js_space)
}

/// `.replace(/\n{3,}/g, '\n\n').replace(/^\n+|\n+$/g, '')`.
pub(crate) fn tidy_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut newlines = 0;
    for c in text.chars() {
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

struct Line {
    head: String,
    text: String,
}

struct TextWriter {
    lines: Vec<String>,
    line: Option<Line>,
    quote_depth: usize,
    lists: Vec<(bool, i64)>,
    marker: Option<String>,
}

impl TextWriter {
    fn prefix(&self) -> String {
        "> ".repeat(self.quote_depth)
    }

    fn open(&mut self) {
        if self.line.is_some() {
            return;
        }
        let indent = if self.lists.len() > 1 {
            "  ".repeat(self.lists.len() - 1)
        } else {
            String::new()
        };
        let marker = self.marker.take().unwrap_or_else(|| {
            if self.lists.is_empty() {
                String::new()
            } else {
                "  ".to_owned()
            }
        });
        self.line = Some(Line {
            head: format!("{}{indent}{marker}", self.prefix()),
            text: String::new(),
        });
    }

    fn close(&mut self, keep_empty: bool) {
        if let Some(line) = self.line.take() {
            let full = format!("{}{}", line.head, trim_end_js(&line.text));
            self.lines.push(trim_end_js(&full).to_owned());
        } else if keep_empty {
            let prefix = self.prefix();
            self.lines.push(trim_end_js(&prefix).to_owned());
        }
    }

    fn line_text(&mut self) -> &mut String {
        self.open();
        &mut self
            .line
            .get_or_insert_with(|| Line {
                head: String::new(),
                text: String::new(),
            })
            .text
    }
}

/// `composerHtmlToText` : la partie texte du mail, d'après le HTML de `sanitize_composer_html`.
/// Un paragraphe ou un titre par ligne, listes en « - » ou « 1. », citations en « > », un lien
/// « texte (adresse) » sauf si son texte est l'adresse, le tableau de résumé en lignes « libellé :
/// valeur » à part, une image « [Image : texte] », la signature (`signature_text`) à sa place.
#[must_use]
#[allow(clippy::too_many_lines, clippy::similar_names)]
pub fn composer_html_to_text(clean_html: &str, signature_text: &str) -> String {
    let mut writer = TextWriter {
        lines: Vec::new(),
        line: None,
        quote_depth: 0,
        lists: Vec::new(),
        marker: None,
    };
    let mut link: Option<(String, String)> = None;
    let mut block_empty = false;
    for piece in pieces(clean_html, true) {
        let (closing, tag, attributes) = match piece {
            Piece::Text(raw) => {
                let text = collapse_spaces(&decode_entities(raw));
                if trim_js(&text).is_empty() && writer.line.is_none() {
                    continue;
                }
                let line = writer.line_text();
                if line.is_empty() {
                    line.push_str(text.trim_start_matches(is_js_space));
                } else {
                    line.push_str(&text);
                }
                block_empty = false;
                if let Some((_, link_text)) = link.as_mut() {
                    link_text.push_str(&text);
                }
                continue;
            }
            Piece::Tag {
                closing,
                name,
                attributes,
            } => (closing, name, attributes),
        };
        match tag.as_str() {
            "br" => {
                writer.open();
                writer.close(false);
            }
            "img" => {
                writer.close(false);
                let alt = decode_entities(read_attribute(attributes, "alt").unwrap_or_default());
                let alt = trim_js(&alt);
                let image = if alt.is_empty() {
                    "[Image]".to_owned()
                } else {
                    format!("[Image : {alt}]")
                };
                let line = format!("{}{image}", writer.prefix());
                writer.lines.push(line);
            }
            "table" => {
                writer.close(false);
                let blank = trim_end_js(&writer.prefix()).to_owned();
                if writer.lines.last().is_some_and(|last| *last != blank) {
                    writer.lines.push(blank);
                }
            }
            "tr" => writer.close(false),
            "th" if closing => {
                if let Some(line) = writer.line.as_mut() {
                    line.text = format!("{} : ", trim_end_js(&line.text));
                }
            }
            "div" if !closing && attributes.contains("data-mail-signature") => {
                writer.close(false);
                if !signature_text.is_empty() {
                    if writer.lines.last().is_some_and(|last| !last.is_empty()) {
                        writer.lines.push(String::new());
                    }
                    let prefix = writer.prefix();
                    for line in signature_text.split('\n') {
                        writer.lines.push(format!("{prefix}{line}"));
                    }
                }
            }
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
                        let addition = format!(" ({target})");
                        writer.line_text().push_str(&addition);
                    }
                }
            }
            "p" | "h2" | "h3" => {
                if closing {
                    writer.close(block_empty);
                    block_empty = false;
                } else {
                    writer.close(false);
                    block_empty = true;
                }
            }
            "ul" | "ol" => {
                writer.close(false);
                if closing {
                    writer.lists.pop();
                } else {
                    let start = read_attribute(attributes, "start")
                        .filter(|start| {
                            !start.is_empty() && start.bytes().all(|b| b.is_ascii_digit())
                        })
                        .and_then(|start| start.parse::<i64>().ok())
                        .unwrap_or(1);
                    writer.lists.push((tag == "ol", start - 1));
                }
            }
            "li" => {
                writer.close(false);
                if !closing && let Some((ordered, count)) = writer.lists.last_mut() {
                    *count += 1;
                    writer.marker = Some(if *ordered {
                        format!("{count}. ")
                    } else {
                        "- ".to_owned()
                    });
                }
            }
            "blockquote" => {
                writer.close(false);
                if closing {
                    writer.quote_depth = writer.quote_depth.saturating_sub(1);
                } else {
                    writer.quote_depth += 1;
                }
            }
            _ => {}
        }
    }
    writer.close(false);
    tidy_lines(&writer.lines.join("\n"))
}

// ── Un brouillon, de retour dans l'éditeur ───────────────────────────────

/// `plainTextToComposerHtml` : un texte en paragraphes de l'éditeur (un brouillon écrit en texte
/// ailleurs).
#[must_use]
pub fn plain_text_to_composer_html(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(text.len() + 16);
    for line in text.split('\n') {
        out.push_str("<p>");
        out.push_str(&escape_html(line));
        out.push_str("</p>");
    }
    out
}

/// Ce que `read_draft_images` rend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadDraftImages {
    /// Le HTML, chaque image de l'éditeur en `img[data-inline-cid]`, les autres sans source.
    pub html: String,
    /// Les Content-ID montrés, dans l'ordre.
    pub used: Vec<String>,
    /// Les images de l'éditeur revenues sans leur partie.
    pub dropped: usize,
}

struct DraftImages<'a> {
    cids: &'a [String],
    used: Vec<String>,
    dropped: usize,
}

impl Policy for DraftImages<'_> {
    fn transform(&mut self, name: &str, attributes: Attributes) -> (String, Attributes) {
        if name != "img" {
            return (name.to_owned(), attributes);
        }
        let src = trim_js(attributes.text("src"));
        let is_cid = starts_with_ignore_case(src, "cid:");
        let cid = if is_cid { &src[4..] } else { src };
        let cid = cid.strip_prefix('<').unwrap_or(cid);
        let cid = cid.strip_suffix('>').unwrap_or(cid).to_lowercase();
        if !is_cid || !self.cids.contains(&cid) || self.used.contains(&cid) {
            if starts_with_ignore_case(src, "cid:tafac-img-") {
                self.dropped += 1;
            }
            return (name.to_owned(), Attributes::default());
        }
        let mut clean = Attributes::default();
        clean.set("data-inline-cid", cid.clone());
        for key in ["alt", "width", "height"] {
            if let Some(value) = attributes.get(key)
                && !value.is_empty()
            {
                clean.set(key, value);
            }
        }
        self.used.push(cid);
        (name.to_owned(), clean)
    }

    fn tag_allowed(&self, _name: &str) -> bool {
        true
    }

    fn non_text(&self, name: &str) -> bool {
        matches!(name, "script" | "style" | "textarea" | "option" | "xmp")
    }

    fn attribute_allowed(&self, _tag: &str, _attribute: &str) -> bool {
        true
    }
}

/// `readDraftImages` : le HTML d'un brouillon, ses images comme l'éditeur les tient. Une
/// `<img src="cid:…">` d'une de ses parties (`cids`, une fois chacune) devient une
/// `img[data-inline-cid]`, toute autre image perd sa source. À faire suivre de
/// `sanitize_composer_html`.
#[must_use]
pub fn read_draft_images(html: &str, cids: &[String]) -> ReadDraftImages {
    let mut policy = DraftImages {
        cids,
        used: Vec::new(),
        dropped: 0,
    };
    let html = sanitize(html, &mut policy);
    ReadDraftImages {
        html,
        used: policy.used,
        dropped: policy.dropped,
    }
}
