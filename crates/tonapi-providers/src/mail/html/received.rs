//! Le mail reçu (`server/mailMessage.js` de Tafac). La page le montre dans un cadre sans script
//! mais de **sa propre origine** et n'a aucun nettoyage à elle : ce passage est le seul rempart.
//!
//! - balises d'une liste (plus `style`), le reste retiré, son texte gardé (sauf `script`,
//!   `textarea`, `option`, `noscript`, `title`, `xml`, `select`, `button`) ;
//! - liens : `#`, http(s), `mailto:`, `tel:` seulement, ouverts dans un onglet, `noopener` ;
//!   **W6** : pas de lien vers les hôtes de Tafac, ni d'http(s) qui ne s'écrit pas `schéma://` ;
//! - images : `cid:` vers la partie du message (formats matriciels seulement), distantes mises
//!   de côté dans `data-remote-src` (« Afficher »), `data:image/` gardées, le reste retiré ;
//! - CSS (`style`, `<style>`) : chaque `url()` réécrite (`rewrite_css_urls`, linéaire, W1) ;
//! - imbrication limitée (W2).

use std::fmt::Write as _;

use url::Url;

use super::sanitize::{Attributes, Exclude, Policy, sanitize};
use crate::mail::text::{escape_html, is_js_space, trim_js};

/// Un mail reçu nettoyé : son HTML et le nombre d'images distantes mises de côté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailHtml {
    /// Le HTML que la page montre.
    pub html: String,
    /// Les images distantes retenues (balises, fonds, `url()` du CSS).
    pub remote_images: usize,
}

/// Le CSS réécrit (`rewriteCssUrls`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssRewrite {
    /// Sans les URL distantes (`none` à leur place).
    pub held: String,
    /// Avec elles, pour « Afficher » ; `None` s'il n'y en avait pas.
    pub revealed: Option<String>,
    /// Combien il y en avait.
    pub remote: usize,
}

const ALLOWED_TAGS: &[&str] = &[
    "a",
    "abbr",
    "address",
    "article",
    "aside",
    "b",
    "bdi",
    "bdo",
    "big",
    "blockquote",
    "br",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "font",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "li",
    "main",
    "mark",
    "nav",
    "ol",
    "p",
    "pre",
    "q",
    "s",
    "samp",
    "section",
    "small",
    "span",
    "strike",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "tt",
    "u",
    "ul",
    "var",
    "wbr",
];

const GLOBAL_ATTRIBUTES: &[&str] = &[
    "align",
    "valign",
    "dir",
    "lang",
    "title",
    "class",
    "id",
    "style",
    "width",
    "height",
    "bgcolor",
    "color",
    "border",
    "cellpadding",
    "cellspacing",
    "colspan",
    "rowspan",
    "nowrap",
    "face",
    "size",
    "start",
    "type",
    "reversed",
    "span",
    "headers",
    "scope",
    "abbr",
    "summary",
    "cite",
    "datetime",
    "hspace",
    "vspace",
    "rules",
    "frame",
    "background",
    "data-remote-background",
    "data-remote-style",
];

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len()
        && text.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

fn is_remote_url(value: &str) -> bool {
    starts_with_ignore_case(value, "http://") || starts_with_ignore_case(value, "https://")
}

/// `COLOR_PATTERN` : `^#?[0-9a-z]{1,20}$` sans tenir compte de la casse.
fn is_plain_color(value: &str) -> bool {
    let body = value.strip_prefix('#').unwrap_or(value);
    (1..=20).contains(&body.len()) && body.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// `decodeURIComponent`, `None` quand il lèverait une erreur.
pub(crate) fn decode_uri_component(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = (*bytes.get(index + 1)? as char).to_digit(16)?;
            let low = (*bytes.get(index + 2)? as char).to_digit(16)?;
            out.push(u8::try_from(high * 16 + low).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `normalizeCid` : un Content-ID tel que le HTML ou une partie l'écrit (`cid:`, chevrons,
/// encodage `%`), en minuscules.
#[must_use]
pub fn normalize_cid(value: &str) -> String {
    let mut cid = trim_js(value);
    if starts_with_ignore_case(cid, "cid:") {
        cid = &cid[4..];
    }
    let cid = cid.strip_prefix('<').unwrap_or(cid);
    let cid = cid.strip_suffix('>').unwrap_or(cid);
    decode_uri_component(cid)
        .unwrap_or_else(|| cid.to_owned())
        .to_lowercase()
}

fn host_key(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// Un lien http(s) qui peut rester (W6) : écrit `http://` ou `https://` (deux barres, aucune
/// barre inverse), lisible, et vers un hôte qui n'est pas l'un de ceux de Tafac (`own_hosts`) :
/// depuis le cadre de même origine, un tel lien partirait avec `Sec-Fetch-Site: same-origin`.
pub(crate) fn http_link_allowed(href: &str, own_hosts: &[&str]) -> bool {
    if !is_remote_url(href) || href.contains('\\') {
        return false;
    }
    let Ok(url) = Url::parse(href) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host_key(host);
    !own_hosts.iter().any(|own| host_key(own) == host)
}

// ── CSS ───────────────────────────────────────────────────────────────────

/// Une unité d'identifiant CSS au début de `bytes[at..]` : `[A-Za-z0-9_-]`, `\` + 1 à 6
/// chiffres hexadécimaux + une espace facultative (`\r\n` comptant pour une, comme le
/// navigateur), ou `\` + un caractère qui n'est ni hexadécimal ni une fin de ligne. Sa longueur,
/// 0 sinon.
fn ident_unit(text: &str, at: usize) -> usize {
    let bytes = text.as_bytes();
    let Some(&first) = bytes.get(at) else {
        return 0;
    };
    if first.is_ascii_alphanumeric() || first == b'_' || first == b'-' {
        return 1;
    }
    if first != b'\\' {
        return 0;
    }
    match bytes.get(at + 1) {
        None | Some(b'\n' | b'\r' | b'\x0c') => 0,
        Some(b) if b.is_ascii_hexdigit() => {
            let mut end = at + 1;
            while end < bytes.len() && end < at + 7 && bytes[end].is_ascii_hexdigit() {
                end += 1;
            }
            if bytes.get(end..end + 2) == Some(b"\r\n") {
                end += 2;
            } else if matches!(bytes.get(end), Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')) {
                end += 1;
            }
            end - at
        }
        Some(_) => 1 + text[at + 1..].chars().next().map_or(1, char::len_utf8),
    }
}

/// Une suite d'unités à partir de `at` : sa fin et si elle contient un échappement.
fn ident_run(text: &str, at: usize) -> (usize, bool) {
    let mut end = at;
    let mut escaped = false;
    loop {
        let length = ident_unit(text, end);
        if length == 0 {
            return (end, escaped);
        }
        if text.as_bytes()[end] == b'\\' {
            escaped = true;
        }
        end += length;
    }
}

/// `decodeCssEscapes` : les échappements CSS décodés (point de code invalide : U+FFFD ; une
/// barre inverse avant une fin de ligne ou à la fin : rien).
fn decode_css_escapes(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            let c = value[index..].chars().next().unwrap_or('\u{fffd}');
            out.push(c);
            index += c.len_utf8();
            continue;
        }
        match bytes.get(index + 1) {
            None => index += 1,
            Some(b) if b.is_ascii_hexdigit() => {
                let mut end = index + 1;
                while end < bytes.len() && end < index + 7 && bytes[end].is_ascii_hexdigit() {
                    end += 1;
                }
                let code = u32::from_str_radix(&value[index + 1..end], 16).unwrap_or(0);
                let decoded = if code > 0 && code <= 0x10_ffff && !(0xd800..=0xdfff).contains(&code)
                {
                    char::from_u32(code).unwrap_or('\u{fffd}')
                } else {
                    '\u{fffd}'
                };
                out.push(decoded);
                if bytes.get(end..end + 2) == Some(b"\r\n") {
                    end += 2;
                } else if matches!(bytes.get(end), Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')) {
                    end += 1;
                }
                index = end;
            }
            Some(b'\r') if bytes.get(index + 2) == Some(&b'\n') => index += 3,
            Some(b'\n' | b'\r' | b'\x0c') => index += 2,
            Some(_) => {
                let c = value[index + 1..].chars().next().unwrap_or('\u{fffd}');
                out.push(c);
                index += 1 + c.len_utf8();
            }
        }
    }
    out
}

fn plain_css_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-')
}

fn push_char(text: &str, index: &mut usize, out: &mut String) {
    if let Some(c) = text[*index..].chars().next() {
        out.push(c);
        *index += c.len_utf8();
    } else {
        *index += 1;
    }
}

/// Étape 1 : les noms de fonction écrits avec des échappements, décodés (`u\72l(` devient
/// `url(`) ; un nom qui, décodé, n'est pas `[a-z-]+` devient `x-invalid`.
fn decode_function_names(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut index = 0;
    while index < css.len() {
        if ident_unit(css, index) == 0 {
            push_char(css, &mut index, &mut out);
            continue;
        }
        let (end, escaped) = ident_run(css, index);
        let run = &css[index..end];
        if escaped && css.as_bytes().get(end) == Some(&b'(') {
            let decoded = decode_css_escapes(run);
            out.push_str(if plain_css_name(&decoded) {
                &decoded
            } else {
                "x-invalid"
            });
        } else {
            out.push_str(run);
        }
        index = end;
    }
    out
}

/// 2. Les mots-clés `@` écrits avec des échappements, décodés de même (`@\69mport`).
fn decode_at_keywords(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut index = 0;
    while index < css.len() {
        if css.as_bytes()[index] != b'@' {
            push_char(css, &mut index, &mut out);
            continue;
        }
        let (end, escaped) = ident_run(css, index + 1);
        if escaped {
            let decoded = decode_css_escapes(&css[index + 1..end]);
            out.push('@');
            out.push_str(if plain_css_name(&decoded) {
                &decoded
            } else {
                "x-invalid"
            });
        } else {
            out.push_str(&css[index..end]);
        }
        index = end;
    }
    out
}

/// 3. `@import…;` retiré (`/@import\b[^;{}]*;?/gi`).
fn drop_imports(css: &str) -> String {
    let bytes = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut index = 0;
    while index < bytes.len() {
        if starts_with_ignore_case(&css[index..], "@import")
            && !bytes
                .get(index + 7)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            let mut end = index + 7;
            while end < bytes.len() && !matches!(bytes[end], b';' | b'{' | b'}') {
                end += 1;
            }
            if bytes.get(end) == Some(&b';') {
                end += 1;
            }
            index = end;
            continue;
        }
        push_char(css, &mut index, &mut out);
    }
    out
}

/// La fonction `image-set(`, `image(` ou `src(` (avec `-webkit-` facultatif) qui commence à
/// `at` : l'indice de sa parenthèse ouvrante.
fn string_url_function(css: &str, at: usize) -> Option<usize> {
    let names = ["image-set(", "image(", "src("];
    let rest = &css[at..];
    if starts_with_ignore_case(rest, "-webkit-") {
        for name in names {
            if starts_with_ignore_case(&rest[8..], name) {
                return Some(at + 8 + name.len() - 1);
            }
        }
    }
    let boundary = at == 0
        || !css
            .as_bytes()
            .get(at - 1)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_');
    if boundary {
        for name in names {
            if starts_with_ignore_case(rest, name) {
                return Some(at + name.len() - 1);
            }
        }
    }
    None
}

/// Étape 4 : `image-set()`, `image()` et `src()`, qui prennent des chaînes pour des URL,
/// remplacées par `none` jusqu'à leur parenthèse fermante (ou la fin).
fn drop_string_url_functions(css: &str) -> String {
    let bytes = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut index = 0;
    while index < bytes.len() {
        if let Some(open) = string_url_function(css, index) {
            let mut depth = 0usize;
            let mut end = open;
            while end < bytes.len() {
                match bytes[end] {
                    b'(' => depth += 1,
                    b')' => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                end += 1;
            }
            out.push_str("none");
            index = end + 1;
            continue;
        }
        push_char(css, &mut index, &mut out);
    }
    out
}

/// `cssUrl` : `url("…")`, chaque caractère hors de `[A-Za-z0-9_\-.:/?#=&%+,;~@!$*]` écrit
/// `\hex ` (le format que la page relit pour « Afficher »).
fn css_url(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 7);
    out.push_str("url(\"");
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || "_-.:/?#=&%+,;~@!$*".contains(c) {
            out.push(c);
        } else {
            let _ = write!(out, "\\{:x} ", u32::from(c));
        }
    }
    out.push_str("\")");
    out
}

fn skip_js_spaces(text: &str, mut index: usize) -> usize {
    while let Some(c) = text.get(index..).and_then(|rest| rest.chars().next()) {
        if !is_js_space(c) {
            break;
        }
        index += c.len_utf8();
    }
    index
}

/// L'`url(` qui commence à `at` (`url(` déjà vu) : la fin de l'appel et sa valeur quand il se lit
/// (`url( "…" )`, `url('…')`, `url(nu)`), `None` sinon.
#[allow(clippy::single_match_else)]
fn read_url_call(css: &str, at: usize) -> (usize, Option<&str>) {
    let bytes = css.as_bytes();
    let open_end = at + 4;
    let start = skip_js_spaces(css, open_end);
    let parsed = match bytes.get(start) {
        Some(&quote @ (b'"' | b'\'')) => {
            let mut index = start + 1;
            let mut close = None;
            while index < bytes.len() {
                match bytes[index] {
                    b if b == quote => {
                        close = Some(index);
                        break;
                    }
                    b'\\' => {
                        let Some(c) = css.get(index + 1..).and_then(|rest| rest.chars().next())
                        else {
                            break;
                        };
                        index += 1 + c.len_utf8();
                    }
                    b'\n' => break,
                    _ => index += css[index..].chars().next().map_or(1, char::len_utf8),
                }
            }
            close.map(|close| (&css[start + 1..close], close + 1))
        }
        _ => {
            let mut index = start;
            while let Some(c) = css.get(index..).and_then(|rest| rest.chars().next()) {
                if matches!(c, '"' | '\'' | '(' | ')' | '\\') || is_js_space(c) {
                    break;
                }
                index += c.len_utf8();
            }
            Some((&css[start..index], index))
        }
    };
    if let Some((value, after)) = parsed {
        let close = skip_js_spaces(css, after);
        if bytes.get(close) == Some(&b')') {
            return (close + 1, Some(value));
        }
    }
    let end = bytes[open_end..]
        .iter()
        .position(|b| *b == b')')
        .map_or(bytes.len(), |offset| open_end + offset + 1);
    (end, None)
}

/// `rewriteCssUrls` : le CSS d'un mail, chaque `url()` réécrite. `data:image/` gardée, `cid:`
/// vers la partie du message (inconnue : `none`), http(s) retenue (`none`, gardée à part pour
/// « Afficher »), tout le reste `none` ; noms de fonction et mots-clés `@` échappés décodés,
/// `@import` retiré, `image-set()`, `image()` et `src()` remplacés par `none`.
///
/// Un balayage linéaire (W1) : les expressions de Node étaient quadratiques sur une longue suite
/// de lettres. Écart voulu : après un échappement hexadécimal, `\r\n` compte pour une espace,
/// comme dans le navigateur (Node laissait `\75\r\nrl(` passer pour autre chose que `url(`).
pub fn rewrite_css_urls(
    css: &str,
    resolve_cid: &mut dyn FnMut(&str) -> Option<String>,
) -> CssRewrite {
    enum Part {
        Text(String),
        Remote(String),
    }
    let Some(normalized) = normalize_css(css) else {
        return CssRewrite {
            held: String::new(),
            revealed: None,
            remote: 0,
        };
    };
    let mut parts: Vec<Part> = Vec::new();
    let mut remote = 0;
    let mut text = String::with_capacity(normalized.len());
    let mut index = 0;
    while index < normalized.len() {
        if !starts_with_ignore_case(&normalized[index..], "url(") {
            push_char(&normalized, &mut index, &mut text);
            continue;
        }
        let (end, value) = read_url_call(&normalized, index);
        index = end;
        let Some(value) = value else {
            text.push_str("none");
            continue;
        };
        let decoded = decode_css_escapes(value);
        let value = trim_js(&decoded);
        if starts_with_ignore_case(value, "data:image/") {
            text.push_str(&css_url(value));
        } else if starts_with_ignore_case(value, "cid:") {
            match resolve_cid(&normalize_cid(value)) {
                Some(resolved) => text.push_str(&css_url(&resolved)),
                None => text.push_str("none"),
            }
        } else if is_remote_url(value) {
            remote += 1;
            parts.push(Part::Text(std::mem::take(&mut text)));
            parts.push(Part::Remote(css_url(value)));
        } else {
            text.push_str("none");
        }
    }
    parts.push(Part::Text(text));
    let join = |reveal: bool| -> String {
        parts
            .iter()
            .map(|part| match part {
                Part::Text(text) => text.as_str(),
                Part::Remote(url) if reveal => url.as_str(),
                Part::Remote(_) => "none",
            })
            .collect()
    };
    let held = join(false);
    let revealed = (remote > 0).then(|| join(true));
    if has_escaped_name(&held) || revealed.as_deref().is_some_and(has_escaped_name) {
        return CssRewrite {
            held: String::new(),
            revealed: None,
            remote: 0,
        };
    }
    CssRewrite {
        held,
        revealed,
        remote,
    }
}

/// Combien de fois au plus les étapes 1 à 4 sont reprises : un CSS qui change encore après est
/// retiré en entier.
const NORMALIZE_ROUNDS: usize = 8;

/// Étapes 1 à 4, reprises jusqu'à ce que le texte ne change plus : retirer `@import…;` ou une
/// fonction recolle le texte de part et d'autre, ce qui peut former un nouveau nom échappé
/// (`\75@import;rl(` devient `\75rl(`, lu `url(`) ou un nouvel `@import` (`@im@import;port`).
/// `None` : encore changeant après `NORMALIZE_ROUNDS` tours, ou un nom échappé resté devant `(`
/// ou après `@`.
fn normalize_css(css: &str) -> Option<String> {
    let mut current = css.to_owned();
    for _ in 0..NORMALIZE_ROUNDS {
        let next = drop_string_url_functions(&drop_imports(&decode_at_keywords(
            &decode_function_names(&current),
        )));
        if next == current {
            return (!has_escaped_name(&next)).then_some(next);
        }
        current = next;
    }
    None
}

/// Un nom écrit avec un échappement juste devant `(`, ou après `@` : ce que le navigateur
/// décoderait en `url(`, `image(` ou `@import`.
fn has_escaped_name(css: &str) -> bool {
    let bytes = css.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let after_at = index > 0 && bytes[index - 1] == b'@';
        if ident_unit(css, index) == 0 {
            index += 1;
            continue;
        }
        let (end, escaped) = ident_run(css, index);
        if escaped && (after_at || bytes.get(end) == Some(&b'(')) {
            return true;
        }
        index = end.max(index + 1);
    }
    false
}

/// `</style` (toutes casses) écrit `<\/style` : le CSS réécrit ne ferme pas son élément.
fn neutralize_style_end(css: &str) -> String {
    let bytes = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'<'
            && bytes.get(index + 1) == Some(&b'/')
            && bytes
                .get(index + 2..index + 7)
                .is_some_and(|name| name.eq_ignore_ascii_case(b"style"))
        {
            out.push_str("<\\/");
            index += 2;
            continue;
        }
        push_char(css, &mut index, &mut out);
    }
    out
}

// ── Le nettoyage ──────────────────────────────────────────────────────────

struct ReceivedPolicy<'a> {
    own_hosts: &'a [&'a str],
    resolve_cid: &'a mut dyn FnMut(&str) -> Option<String>,
    remote: usize,
}

impl ReceivedPolicy<'_> {
    /// `holdRemoteBackground` puis `cleanStyleAttribute`, sur toutes les balises.
    fn every_tag(&mut self, mut attributes: Attributes) -> Attributes {
        let background = attributes.remove("background").unwrap_or_default();
        attributes.remove("data-remote-background");
        let background = trim_js(&background);
        if is_remote_url(background) {
            self.remote += 1;
            attributes.set("data-remote-background", background);
        } else if starts_with_ignore_case(background, "cid:")
            && let Some(resolved) = (self.resolve_cid)(&normalize_cid(background))
        {
            attributes.set("background", resolved);
        }
        attributes.remove("data-remote-style");
        if let Some(style) = attributes.get("style").map(str::to_owned) {
            let rewritten = rewrite_css_urls(&style, &mut *self.resolve_cid);
            self.remote += rewritten.remote;
            attributes.set("style", rewritten.held);
            if let Some(revealed) = rewritten.revealed {
                attributes.set("data-remote-style", revealed);
            }
        }
        attributes
    }
}

impl Policy for ReceivedPolicy<'_> {
    fn transform(&mut self, name: &str, mut attributes: Attributes) -> (String, Attributes) {
        let mut tag = name.to_owned();
        match name {
            "body" => {
                let bgcolor = attributes.remove("bgcolor").unwrap_or_default();
                let text = attributes.remove("text").unwrap_or_default();
                let mut styles: Vec<String> = Vec::new();
                if is_plain_color(&bgcolor) {
                    styles.push(format!("background-color:{bgcolor}"));
                }
                if is_plain_color(&text) {
                    styles.push(format!("color:{text}"));
                }
                if let Some(style) = attributes.get("style").filter(|style| !style.is_empty()) {
                    styles.push(style.to_owned());
                }
                if !styles.is_empty() {
                    attributes.set("style", styles.join(";"));
                }
                attributes.set("class", "tafac-mail-body");
                "div".clone_into(&mut tag);
            }
            "a" => {
                let href = attributes.remove("href").unwrap_or_default();
                let href = trim_js(&href);
                let target = if href.starts_with('#') {
                    "_self"
                } else {
                    "_blank"
                };
                let kept = href.starts_with('#')
                    || starts_with_ignore_case(href, "mailto:")
                    || starts_with_ignore_case(href, "tel:")
                    || ((starts_with_ignore_case(href, "http:")
                        || starts_with_ignore_case(href, "https:"))
                        && http_link_allowed(href, self.own_hosts));
                if kept {
                    attributes.set("href", href);
                }
                attributes.set("target", target);
                attributes.set("rel", "noopener noreferrer");
            }
            "img" => {
                let src = attributes.remove("src").unwrap_or_default();
                attributes.remove("data-remote-src");
                let src = trim_js(&src);
                if starts_with_ignore_case(src, "cid:") {
                    if let Some(resolved) = (self.resolve_cid)(&normalize_cid(src)) {
                        attributes.set("src", resolved);
                    }
                } else if is_remote_url(src) {
                    self.remote += 1;
                    attributes.set("data-remote-src", src);
                } else if starts_with_ignore_case(src, "data:image/") {
                    attributes.set("src", src);
                }
            }
            _ => {}
        }
        let attributes = self.every_tag(attributes);
        (tag, attributes)
    }

    fn tag_allowed(&self, name: &str) -> bool {
        ALLOWED_TAGS.contains(&name)
    }

    fn non_text(&self, name: &str) -> bool {
        matches!(
            name,
            "script" | "textarea" | "option" | "noscript" | "title" | "xml" | "select" | "button"
        )
    }

    fn attribute_allowed(&self, tag: &str, attribute: &str) -> bool {
        GLOBAL_ATTRIBUTES.contains(&attribute)
            || (tag == "a" && matches!(attribute, "href" | "name" | "target" | "rel"))
            || (tag == "img" && matches!(attribute, "src" | "alt" | "data-remote-src"))
    }

    fn scheme_checked(&self, attribute: &str) -> bool {
        matches!(attribute, "href" | "src" | "cite" | "background")
    }

    fn allowed_schemes(&self, tag: &str) -> &'static [&'static str] {
        if tag == "img" {
            &["data"]
        } else {
            &["http", "https", "mailto", "tel"]
        }
    }

    fn allow_protocol_relative(&self) -> bool {
        false
    }

    fn exclude(&self, tag: &str, attributes: &Attributes) -> Exclude {
        if tag == "img"
            && attributes.text("src").is_empty()
            && attributes.text("data-remote-src").is_empty()
        {
            Exclude::Always
        } else {
            Exclude::Never
        }
    }

    fn finish_style(&mut self, css: &str) -> Option<(String, Option<String>)> {
        let rewritten = rewrite_css_urls(css, &mut *self.resolve_cid);
        self.remote += rewritten.remote;
        Some((neutralize_style_end(&rewritten.held), rewritten.revealed))
    }
}

/// `sanitizeMailHtml` : le HTML d'un mail reçu, nettoyé pour la page. `own_hosts` : les hôtes de
/// Tafac, vers lesquels aucun lien ne reste (W6) ; `resolve_cid` : l'adresse de l'image du
/// message qui porte ce Content-ID (déjà normalisé), ou `None`.
pub fn sanitize_mail_html(
    html: &str,
    own_hosts: &[&str],
    resolve_cid: &mut dyn FnMut(&str) -> Option<String>,
) -> MailHtml {
    let mut policy = ReceivedPolicy {
        own_hosts,
        resolve_cid,
        remote: 0,
    };
    let html = sanitize(html, &mut policy);
    MailHtml {
        html,
        remote_images: policy.remote,
    }
}

/// `mailTextToHtml` : un mail en texte, échappé, ses liens http(s) cliquables (la ponctuation
/// finale laissée dehors), dans `<div class="tafac-mail-text">`. Comme dans le HTML (W6), un lien
/// vers un hôte de Tafac reste du texte.
#[must_use]
pub fn mail_text_to_html(text: &str, own_hosts: &[&str]) -> String {
    let bytes = text.as_bytes();
    let mut html = String::with_capacity(text.len() + 32);
    let mut last = 0;
    let mut index = 0;
    while index < bytes.len() {
        let boundary = index == 0
            || !bytes
                .get(index - 1)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_');
        let scheme = if !boundary {
            0
        } else if starts_with_ignore_case(&text[index..], "https://") {
            8
        } else if starts_with_ignore_case(&text[index..], "http://") {
            7
        } else {
            0
        };
        if scheme == 0 {
            index += text[index..].chars().next().map_or(1, char::len_utf8);
            continue;
        }
        let mut end = index + scheme;
        while let Some(c) = text.get(end..).and_then(|rest| rest.chars().next()) {
            if is_js_space(c) || matches!(c, '<' | '>' | '"' | '\'') {
                break;
            }
            end += c.len_utf8();
        }
        if end == index + scheme {
            index += 1;
            continue;
        }
        let url = text[index..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}']);
        html.push_str(&escape_html(&text[last..index]));
        if http_link_allowed(url, own_hosts) {
            let escaped = escape_html(url);
            let _ = write!(
                html,
                "<a href=\"{escaped}\" target=\"_blank\" rel=\"noopener noreferrer\">{escaped}</a>"
            );
        } else {
            html.push_str(&escape_html(url));
        }
        last = index + url.len();
        index = end;
    }
    html.push_str(&escape_html(&text[last..]));
    format!("<div class=\"tafac-mail-text\">{html}</div>")
}
