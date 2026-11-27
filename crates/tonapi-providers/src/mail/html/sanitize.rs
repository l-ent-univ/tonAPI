//! Le moteur des nettoyages : sanitize-html (2.18) en mode `discard`, sur les événements de
//! `tokenizer`. Chaque nettoyage (`Policy`) dit quelles balises et quels attributs passent et
//! comment ils sont transformés ; le moteur réécrit une sortie canonique au format de
//! sanitize-html, dont les passes suivantes et la page dépendent :
//!
//! - texte échappé (`& < >`), attributs entre guillemets doubles (`& < > "` échappés) ;
//! - `<br />`, `<img … />` (les balises « auto-fermantes » de sanitize-html), un attribut vide
//!   écrit sans valeur (`<div data-mail-signature>`), sauf `alt=""` ;
//! - balises équilibrées : une balise refusée disparaît, son texte reste (sauf les balises « sans
//!   texte », dont le contenu part avec elles).
//!
//! Ajouts à sanitize-html : une limite d'imbrication (`NESTING_LIMIT`, W2), au-delà de laquelle
//! une balise est traitée comme refusée ; le texte d'un `<style>` ou `<script>` refusé est
//! toujours échappé, jamais recopié brut.

use super::style::{StyleNode, parse_style};
use super::tokenizer::{self, Handler};
use crate::mail::text::trim_js;

/// Profondeur d'imbrication au-delà de laquelle une balise est retirée (son texte gardé).
pub const NESTING_LIMIT: usize = 100;

/// Profondeur des `<iframe>` refusés dont le contenu est relu comme du HTML (sanitize-html).
const MAX_IFRAME_FALLBACK_DEPTH: usize = 8;

/// Les attributs d'une balise, dans l'ordre d'un objet JavaScript : une valeur remplacée garde sa
/// place, une nouvelle va à la fin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Attributes(pub(crate) Vec<(String, String)>);

impl Attributes {
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub(crate) fn has(&self, name: &str) -> bool {
        self.0.iter().any(|(key, _)| key == name)
    }

    pub(crate) fn set(&mut self, name: &str, value: impl Into<String>) {
        let value = value.into();
        if let Some(entry) = self.0.iter_mut().find(|(key, _)| key == name) {
            entry.1 = value;
        } else {
            self.0.push((name.to_owned(), value));
        }
    }

    pub(crate) fn remove(&mut self, name: &str) -> Option<String> {
        let at = self.0.iter().position(|(key, _)| key == name)?;
        Some(self.0.remove(at).1)
    }

    /// La valeur, vide si absente (`String(attribs.x ?? '')`).
    pub(crate) fn text(&self, name: &str) -> &str {
        self.get(name).unwrap_or_default()
    }
}

/// Ce qu'un `exclusiveFilter` décide d'un élément, une fois ses attributs connus.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Exclude {
    Never,
    /// Retiré avec son contenu.
    Always,
    /// Retiré avec son contenu s'il contient du texte (autre que des espaces).
    IfText,
}

/// Un nettoyage : les options de sanitize-html qu'il donne.
pub(crate) trait Policy {
    /// `transformTags` : la transformation propre à la balise, puis celle de `*`.
    fn transform(&mut self, name: &str, attributes: Attributes) -> (String, Attributes) {
        (name.to_owned(), attributes)
    }
    /// `allowedTags`.
    fn tag_allowed(&self, name: &str) -> bool;
    /// `nonTextTags` : refusées, leur texte part avec elles.
    fn non_text(&self, name: &str) -> bool;
    /// `allowedAttributes` (`false` : tous).
    fn attribute_allowed(&self, tag: &str, attribute: &str) -> bool;
    /// `allowedSchemesAppliedToAttributes`.
    fn scheme_checked(&self, attribute: &str) -> bool {
        matches!(
            attribute,
            "href"
                | "src"
                | "cite"
                | "action"
                | "formaction"
                | "data"
                | "xlink:href"
                | "poster"
                | "background"
                | "ping"
                | "longdesc"
                | "usemap"
                | "codebase"
                | "classid"
                | "archive"
                | "profile"
                | "manifest"
                | "itemid"
                | "dynsrc"
                | "lowsrc"
        )
    }
    /// `allowedSchemesByTag`, sinon `allowedSchemes`.
    fn allowed_schemes(&self, _tag: &str) -> &'static [&'static str] {
        &["http", "https", "ftp", "mailto", "tel"]
    }
    /// `allowProtocolRelative`.
    fn allow_protocol_relative(&self) -> bool {
        true
    }
    /// `allowedStyles` : `None` quand la balise n'a pas de liste (tout est gardé, commentaires
    /// compris), sinon si la déclaration passe.
    fn style_allowed(&self, _tag: &str, _prop: &str, _value: &str) -> Option<bool> {
        None
    }
    /// `exclusiveFilter`, décidé sur la balise d'origine et ses attributs gardés.
    fn exclude(&self, _tag: &str, _attributes: &Attributes) -> Exclude {
        Exclude::Never
    }
    /// Le contenu brut d'un `<style>` gardé, réécrit : `(css, attribut data-remote-css)`.
    /// `None` : laissé tel quel.
    fn finish_style(&mut self, _css: &str) -> Option<(String, Option<String>)> {
        None
    }
}

/// Nettoie `html` selon `policy`.
pub(crate) fn sanitize<P: Policy>(html: &str, policy: &mut P) -> String {
    run(html, policy, 0, 0)
}

fn run<P: Policy>(html: &str, policy: &mut P, base_depth: usize, iframe_depth: usize) -> String {
    let mut engine = Engine {
        source: html,
        policy,
        result: String::with_capacity(html.len()),
        stack: Vec::new(),
        base_depth,
        iframe_depth,
        skip_text: false,
        skip_text_depth: 0,
        raw_text_end: None,
        skip_raw_text: false,
    };
    tokenizer::parse(html, &mut engine);
    engine.result
}

/// `escapeHtml` de sanitize-html : `& < >`, et `"` dans un attribut.
pub(crate) fn escape(text: &str, quote: bool, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if quote => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// Les balises que sanitize-html écrit `<x />`, sans fermante.
fn self_closing(name: &str) -> bool {
    matches!(
        name,
        "img" | "br" | "hr" | "area" | "base" | "basefont" | "input" | "link" | "meta" | "col"
    )
}

#[allow(clippy::too_many_lines)]
/// `nonBooleanAttributes` de sanitize-html : vides, ils sont retirés (sauf `alt`).
fn non_boolean(name: &str) -> bool {
    matches!(
        name,
        "abbr"
            | "accept"
            | "accept-charset"
            | "accesskey"
            | "action"
            | "allow"
            | "alt"
            | "as"
            | "autocapitalize"
            | "autocomplete"
            | "blocking"
            | "charset"
            | "cite"
            | "class"
            | "color"
            | "cols"
            | "colspan"
            | "content"
            | "contenteditable"
            | "coords"
            | "crossorigin"
            | "data"
            | "datetime"
            | "decoding"
            | "dir"
            | "dirname"
            | "download"
            | "draggable"
            | "enctype"
            | "enterkeyhint"
            | "fetchpriority"
            | "for"
            | "form"
            | "formaction"
            | "formenctype"
            | "formmethod"
            | "formtarget"
            | "headers"
            | "height"
            | "hidden"
            | "high"
            | "href"
            | "hreflang"
            | "http-equiv"
            | "id"
            | "imagesizes"
            | "imagesrcset"
            | "inputmode"
            | "integrity"
            | "is"
            | "itemid"
            | "itemprop"
            | "itemref"
            | "itemtype"
            | "kind"
            | "label"
            | "lang"
            | "list"
            | "loading"
            | "low"
            | "max"
            | "maxlength"
            | "media"
            | "method"
            | "min"
            | "minlength"
            | "name"
            | "nonce"
            | "optimum"
            | "pattern"
            | "ping"
            | "placeholder"
            | "popover"
            | "popovertarget"
            | "popovertargetaction"
            | "poster"
            | "preload"
            | "referrerpolicy"
            | "rel"
            | "rows"
            | "rowspan"
            | "sandbox"
            | "scope"
            | "shape"
            | "size"
            | "sizes"
            | "slot"
            | "span"
            | "spellcheck"
            | "src"
            | "srcdoc"
            | "srclang"
            | "srcset"
            | "start"
            | "step"
            | "style"
            | "tabindex"
            | "target"
            | "title"
            | "translate"
            | "type"
            | "usemap"
            | "value"
            | "width"
            | "wrap"
    ) || (name.starts_with("on") && is_event_handler(name))
}

fn is_event_handler(name: &str) -> bool {
    matches!(
        &name[2..],
        "auxclick"
            | "afterprint"
            | "beforematch"
            | "beforeprint"
            | "beforeunload"
            | "beforetoggle"
            | "blur"
            | "cancel"
            | "canplay"
            | "canplaythrough"
            | "change"
            | "click"
            | "close"
            | "contextlost"
            | "contextmenu"
            | "contextrestored"
            | "copy"
            | "cuechange"
            | "cut"
            | "dblclick"
            | "drag"
            | "dragend"
            | "dragenter"
            | "dragleave"
            | "dragover"
            | "dragstart"
            | "drop"
            | "durationchange"
            | "emptied"
            | "ended"
            | "error"
            | "focus"
            | "formdata"
            | "hashchange"
            | "input"
            | "invalid"
            | "keydown"
            | "keypress"
            | "keyup"
            | "languagechange"
            | "load"
            | "loadeddata"
            | "loadedmetadata"
            | "loadstart"
            | "message"
            | "messageerror"
            | "mousedown"
            | "mouseenter"
            | "mouseleave"
            | "mousemove"
            | "mouseout"
            | "mouseover"
            | "mouseup"
            | "offline"
            | "online"
            | "pagehide"
            | "pageshow"
            | "paste"
            | "pause"
            | "play"
            | "playing"
            | "popstate"
            | "progress"
            | "ratechange"
            | "reset"
            | "resize"
            | "rejectionhandled"
            | "scroll"
            | "scrollend"
            | "securitypolicyviolation"
            | "seeked"
            | "seeking"
            | "select"
            | "slotchange"
            | "stalled"
            | "storage"
            | "submit"
            | "suspend"
            | "timeupdate"
            | "toggle"
            | "unhandledrejection"
            | "unload"
            | "volumechange"
            | "waiting"
            | "wheel"
    )
}

/// `VALID_HTML_ATTRIBUTE_NAME` : `^[^\0\t\n\f\r /<=>]+$`.
fn valid_attribute_name(name: &str) -> bool {
    !name.is_empty()
        && !name.bytes().any(|b| {
            matches!(
                b,
                0 | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ' | b'/' | b'<' | b'=' | b'>'
            )
        })
}

/// `cleanHref` de launder : sans les caractères de U+0000 à U+0020, sans les commentaires
/// `<!-- … -->` (retirés de gauche à droite, comme sa boucle, mais en une passe).
pub(crate) fn clean_href(href: &str) -> String {
    let stripped: Vec<u8> = href.bytes().filter(|b| *b > 0x20).collect();
    let mut out: Vec<u8> = Vec::with_capacity(stripped.len());
    let mut position = 0;
    loop {
        // Le premier `<!--` du texte courant (`out` puis le reste) commence au plus trois octets
        // avant la jonction.
        let joint = out.len().saturating_sub(3);
        let mut first: Option<(bool, usize)> = None;
        for start in joint..out.len() {
            let mut matched = true;
            for (offset, expected) in b"<!--".iter().enumerate() {
                let at = start + offset;
                let byte = if at < out.len() {
                    out.get(at).copied()
                } else {
                    stripped.get(position + at - out.len()).copied()
                };
                if byte != Some(*expected) {
                    matched = false;
                    break;
                }
            }
            if matched {
                first = Some((true, start));
                break;
            }
        }
        if first.is_none()
            && let Some(offset) = find(&stripped[position..], b"<!--")
        {
            first = Some((false, position + offset));
        }
        let Some((in_out, start)) = first else { break };
        // `-->` cherché après les quatre caractères de `<!--`.
        let search_from = if in_out {
            position + (start + 4 - out.len())
        } else {
            start + 4
        };
        let Some(offset) = find(stripped.get(search_from..).unwrap_or_default(), b"-->") else {
            break;
        };
        let comment_end = search_from + offset + 3;
        if in_out {
            out.truncate(start);
        } else {
            out.extend_from_slice(&stripped[position..start]);
        }
        position = comment_end;
    }
    out.extend_from_slice(stripped.get(position..).unwrap_or_default());
    String::from_utf8_lossy(&out).into_owned()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// `naughtyHref` de launder : vrai pour un lien à refuser (schéma hors de la liste, ou `//`
/// quand ils ne sont pas permis). Testé après `clean_href`.
pub(crate) fn naughty_href(href: &str, allowed: &[&str], allow_protocol_relative: bool) -> bool {
    let href = clean_href(href);
    let bytes = href.as_bytes();
    let scheme_length = if bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        1 + bytes[1..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
            .count()
    } else {
        0
    };
    if scheme_length > 0 && bytes.get(scheme_length) == Some(&b':') {
        let scheme = href[..scheme_length].to_ascii_lowercase();
        return !allowed.contains(&scheme.as_str());
    }
    if bytes.len() >= 2 && matches!(bytes[0], b'/' | b'\\') && matches!(bytes[1], b'/' | b'\\') {
        return !allow_protocol_relative;
    }
    false
}

fn local_part(name: &str) -> String {
    let lower = name.to_lowercase();
    match lower.rfind(':') {
        Some(at) => lower[at + 1..].to_owned(),
        None => lower,
    }
}

struct Frame {
    tag: String,
    name: String,
    skip: bool,
    tag_position: usize,
    has_text: bool,
    exclude: Exclude,
    /// Un `<style>` gardé : où commence son contenu.
    style_body: Option<usize>,
}

struct Engine<'a, 'p, P: Policy> {
    source: &'a str,
    policy: &'p mut P,
    result: String,
    stack: Vec<Frame>,
    base_depth: usize,
    iframe_depth: usize,
    skip_text: bool,
    skip_text_depth: usize,
    raw_text_end: Option<usize>,
    skip_raw_text: bool,
}

impl<P: Policy> Engine<'_, '_, P> {
    fn depth(&self) -> usize {
        self.base_depth + self.stack.len()
    }

    fn update_raw_text_region(&mut self, start: usize) {
        if let Some(end) = self.raw_text_end
            && start >= end
        {
            self.raw_text_end = None;
            self.skip_raw_text = false;
        }
    }

    /// `animatesUrlAttribute` : une animation SVG qui viserait un attribut de lien.
    fn animates_url_attribute(&self, name: &str, attributes: &Attributes) -> bool {
        let local = local_part(name);
        if !matches!(
            local.as_str(),
            "animate" | "animatecolor" | "animatemotion" | "animatetransform" | "set"
        ) {
            return false;
        }
        attributes.0.iter().any(|(key, value)| {
            if local_part(key) != "attributename" {
                return false;
            }
            let target = trim_js(value).to_lowercase();
            let local_name = target.rsplit(':').next().unwrap_or_default();
            local_name == "href"
                || self.policy.scheme_checked(&target)
                || self.policy.scheme_checked(local_name)
        })
    }

    /// Les attributs gardés, écrits dans `self.result`.
    fn write_attributes(&mut self, tag: &str, attributes: Attributes) -> Attributes {
        let mut kept = Attributes::default();
        for (name, value) in attributes.0 {
            if !valid_attribute_name(&name) {
                continue;
            }
            if value.is_empty() && name != "alt" && non_boolean(&name) {
                continue;
            }
            if !self.policy.attribute_allowed(tag, &name) {
                continue;
            }
            if self.policy.scheme_checked(&name)
                && naughty_href(
                    &value,
                    self.policy.allowed_schemes(tag),
                    self.policy.allow_protocol_relative(),
                )
            {
                continue;
            }
            let written = if name == "style" {
                let Some(nodes) = parse_style(tag, &value) else {
                    continue;
                };
                let mut style = String::new();
                for node in &nodes {
                    let keep = match node {
                        StyleNode::Declaration { prop, value, .. } => {
                            self.policy.style_allowed(tag, prop, value).unwrap_or(true)
                        }
                        StyleNode::Other => self.policy.style_allowed(tag, "", "").is_none(),
                    };
                    if keep {
                        if !style.is_empty() {
                            style.push(';');
                        }
                        node.write(&mut style);
                    }
                }
                if style.is_empty() {
                    continue;
                }
                style
            } else {
                value.clone()
            };
            self.result.push(' ');
            self.result.push_str(&name);
            if !written.is_empty() {
                self.result.push_str("=\"");
                escape(&written, true, &mut self.result);
                self.result.push('"');
            } else if name == "alt" {
                self.result.push_str("=\"\"");
            }
            kept.0.push((name, value));
        }
        kept
    }
}

impl<P: Policy> Handler for Engine<'_, '_, P> {
    fn open_tag(
        &mut self,
        name: &str,
        attributes: Vec<(String, String)>,
        start: usize,
        end: usize,
    ) {
        self.update_raw_text_region(start);
        if self.skip_raw_text {
            return;
        }
        if self.skip_text {
            self.skip_text_depth += 1;
            return;
        }
        let tag_position = self.result.len();
        let (tag, attributes) = self.policy.transform(name, Attributes(attributes));
        let skip = !self.policy.tag_allowed(&tag)
            || self.animates_url_attribute(&tag, &attributes)
            || self.depth() >= NESTING_LIMIT;
        if skip && self.policy.non_text(&tag) {
            self.skip_text = true;
            self.skip_text_depth = 1;
            if name.eq_ignore_ascii_case("noscript") {
                self.raw_text_end = Some(tokenizer::find_raw_text_end(
                    self.source,
                    "noscript",
                    end + 1,
                ));
            }
        }
        let mut frame = Frame {
            tag: name.to_owned(),
            name: tag.clone(),
            skip,
            tag_position,
            has_text: false,
            exclude: Exclude::Never,
            style_body: None,
        };
        if skip {
            self.stack.push(frame);
            return;
        }
        self.result.push('<');
        self.result.push_str(&tag);
        let kept = self.write_attributes(&tag, attributes);
        frame.exclude = self.policy.exclude(name, &kept);
        if self_closing(&tag) {
            self.result.push_str(" />");
        } else {
            self.result.push('>');
            if tag == "style" {
                frame.style_body = Some(self.result.len());
            }
        }
        self.stack.push(frame);
    }

    fn text(&mut self, text: &str, start: usize) {
        self.update_raw_text_region(start);
        if self.skip_text || self.skip_raw_text {
            return;
        }
        let top = self
            .stack
            .last()
            .map(|frame| (frame.tag.clone(), frame.skip));
        match top {
            Some((tag, false))
                if (tag == "script" || tag == "style") && self.policy.tag_allowed(&tag) =>
            {
                self.result.push_str(text);
            }
            Some((tag, false)) if tag == "textarea" && self.policy.tag_allowed(&tag) => {
                escape(text, false, &mut self.result);
            }
            Some((tag, false)) if tag == "xmp" && self.policy.tag_allowed(&tag) => {
                for c in text.chars() {
                    match c {
                        '<' => self.result.push_str("&lt;"),
                        '>' => self.result.push_str("&gt;"),
                        _ => self.result.push(c),
                    }
                }
            }
            Some((tag, _))
                if tag == "iframe"
                    && !self.policy.tag_allowed(&tag)
                    && self.iframe_depth < MAX_IFRAME_FALLBACK_DEPTH =>
            {
                let depth = self.depth();
                let nested = run(text, &mut *self.policy, depth, self.iframe_depth + 1);
                self.result.push_str(&nested);
            }
            _ => escape(text, false, &mut self.result),
        }
        if let Some(frame) = self.stack.last_mut()
            && !frame.has_text
            && !trim_js(text).is_empty()
        {
            frame.has_text = true;
        }
    }

    fn close_tag(&mut self, name: &str, _implied: bool, start: usize) {
        self.update_raw_text_region(start);
        if self.skip_raw_text {
            if self.stack.last().is_none_or(|frame| frame.tag != name) {
                return;
            }
        } else if self.skip_text {
            self.skip_text_depth = self.skip_text_depth.saturating_sub(1);
            if self.skip_text_depth == 0 {
                self.skip_text = false;
                if self.raw_text_end.is_some() {
                    self.skip_raw_text = true;
                }
            } else {
                return;
            }
        }
        let Some(frame) = self.stack.pop() else {
            return;
        };
        if frame.tag != name {
            self.stack.push(frame);
            return;
        }
        if !frame.skip {
            let excluded = match frame.exclude {
                Exclude::Never => false,
                Exclude::Always => true,
                Exclude::IfText => frame.has_text,
            };
            if excluded {
                self.result.truncate(frame.tag_position);
                return;
            }
        }
        if frame.has_text
            && let Some(parent) = self.stack.last_mut()
        {
            parent.has_text = true;
        }
        if frame.skip || self_closing(&frame.name) {
            return;
        }
        if let Some(body) = frame.style_body
            && body <= self.result.len()
        {
            let css = self.result[body..].to_owned();
            if let Some((held, revealed)) = self.policy.finish_style(&css) {
                self.result.truncate(body - 1);
                if let Some(revealed) = revealed {
                    self.result.push_str(" data-remote-css=\"");
                    self.result
                        .push_str(&crate::mail::text::escape_html(&revealed));
                    self.result.push('"');
                }
                self.result.push('>');
                self.result.push_str(&held);
            }
        }
        self.result.push_str("</");
        self.result.push_str(&frame.name);
        self.result.push('>');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liens_douteux() {
        let schemes = &["http", "https", "mailto", "tel"];
        for href in [
            "javascript:alert(1)",
            "java\tscript:alert(1)",
            "\u{1}javascript:alert(1)",
            "java<!-- x -->script:alert(1)",
            "JAVASCRIPT:x",
            "data:text/html,x",
        ] {
            assert!(naughty_href(href, schemes, false), "{href:?}");
        }
        assert!(naughty_href("//evil.example", schemes, false));
        assert!(naughty_href("\\\\evil.example", schemes, false));
        assert!(!naughty_href("//evil.example", schemes, true));
        for href in [
            "https://x.fr",
            "mailto:a@b.fr",
            "/relatif",
            "page.html",
            "#top",
        ] {
            assert!(!naughty_href(href, schemes, false), "{href:?}");
        }
        assert_eq!(clean_href("a<!--x-->b<!--y-->c"), "abc");
        assert_eq!(clean_href("<!<!--x-->--y-->z"), "z");
        assert_eq!(clean_href("a<!--x"), "a<!--x");
    }
}
