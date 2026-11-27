//! Lecture de HTML à la manière de htmlparser2 (v12, celui de sanitize-html), dont les sorties de
//! Tafac dépendent : éléments de texte brut (`script`, `style`, `iframe`, `xmp`…) et RCDATA
//! (`title`, `textarea`), commentaires, doctype, instructions, attributs avec ou sans valeur et
//! guillemets, références décodées, balises fermantes égarées, fermetures implicites (`<p>` dans
//! `<p>`, `<li>` dans `<li>`…), éléments vides, contexte SVG et MathML.
//!
//! La sûreté ne repose pas sur cette lecture : les nettoyages réécrivent une sortie canonique
//! (texte échappé, attributs entre guillemets, balises de la liste équilibrées). Elle suit
//! htmlparser2 pour que la sortie soit celle de Tafac, jusque dans ses bizarreries de fin de
//! texte.
//!
//! Tout est linéaire : la pile garde un compte par nom, une balise fermante inconnue ne la
//! parcourt pas.

use std::collections::{HashMap, HashSet};

use super::entities::ENTITIES;

/// Ce que la lecture rapporte, dans l'ordre du texte. `start` est l'indice de début de
/// l'événement tel que htmlparser2 le tient (`parser.startIndex`), en octets.
pub(crate) trait Handler {
    /// Une balise ouvrante, ses attributs (noms en minuscules, le premier d'un nom répété
    /// gardé). `end` : l'indice du `>` qui la ferme.
    fn open_tag(&mut self, name: &str, attributes: Vec<(String, String)>, start: usize, end: usize);
    /// Du texte, références décodées (sauf dans les éléments de texte brut).
    fn text(&mut self, text: &str, start: usize);
    /// La fin d'un élément, écrite (`implied` faux) ou déduite.
    fn close_tag(&mut self, name: &str, implied: bool, start: usize);
}

/// Lit `html` en entier et rapporte ses événements à `handler`.
pub(crate) fn parse<H: Handler>(html: &str, handler: &mut H) {
    let mut parser = Parser::new(html, handler);
    parser.run();
}

/// Où commence l'élément de texte brut que le navigateur termine : le premier `</nom` suivi
/// d'une espace, de `/` ou de `>`, à partir de `from` (sans tenir compte de la casse).
/// `usize::MAX` s'il n'y en a pas.
pub(crate) fn find_raw_text_end(html: &str, name: &str, from: usize) -> usize {
    let bytes = html.as_bytes();
    let name = name.as_bytes();
    let mut index = from;
    while index + 2 + name.len() < bytes.len() {
        if bytes[index] == b'<'
            && bytes[index + 1] == b'/'
            && bytes[index + 2..index + 2 + name.len()].eq_ignore_ascii_case(name)
            && matches!(
                bytes[index + 2 + name.len()],
                b'\t' | b'\n' | b'\x0c' | b'\r' | b' ' | b'/' | b'>'
            )
        {
            return index;
        }
        index += 1;
    }
    usize::MAX
}

// ── Références ─────────────────────────────────────────────────────────────

/// Où se trouve une référence : le texte décode aussi les références historiques suivies d'une
/// lettre (`&copy2` donne `©2`), un attribut non (`&notit` y reste tel quel).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntityMode {
    Text,
    Attribute,
}

/// Remplacements des références numériques de 128 à 159 (Windows-1252), 0 : aucun.
const C1_REPLACEMENTS: [u32; 32] = [
    8364, 0, 8218, 402, 8222, 8230, 8224, 8225, 710, 8240, 352, 8249, 338, 0, 381, 0, 0, 8216,
    8217, 8220, 8221, 8226, 8211, 8212, 732, 8482, 353, 8250, 339, 0, 382, 376,
];

fn replace_code_point(code: u32) -> char {
    if code == 0 || (0xd800..=0xdfff).contains(&code) || code > 0x10_ffff {
        return '\u{fffd}';
    }
    let code = if (128..=159).contains(&code) {
        match C1_REPLACEMENTS.get((code - 128) as usize) {
            Some(&replacement) if replacement != 0 => replacement,
            _ => code,
        }
    } else {
        code
    };
    char::from_u32(code).unwrap_or('\u{fffd}')
}

fn lookup_entity(name: &str) -> Option<(&'static str, bool)> {
    ENTITIES
        .binary_search_by(|(entry, _, _)| (*entry).cmp(name))
        .ok()
        .and_then(|index| ENTITIES.get(index))
        .map(|(_, value, legacy)| (*value, *legacy))
}

/// Le nom de référence le plus long du standard (`CounterClockwiseContourIntegral`).
const MAX_ENTITY_NAME: usize = 32;

/// Décode la référence qui commence à `at`, juste après un `&` : octets lus après le `&` et
/// texte obtenu, ou `None` quand ce n'est pas une référence (le `&` reste du texte).
pub(crate) fn decode_reference(src: &str, at: usize, mode: EntityMode) -> Option<(usize, String)> {
    let bytes = src.as_bytes();
    if bytes.get(at) == Some(&b'#') {
        let hex = matches!(bytes.get(at + 1), Some(b'x' | b'X'));
        let digits_start = at + 1 + usize::from(hex);
        let mut code: u32 = 0;
        let mut index = digits_start;
        while let Some(&b) = bytes.get(index) {
            let digit = if hex {
                (b as char).to_digit(16)
            } else {
                (b as char).to_digit(10)
            };
            let Some(digit) = digit else { break };
            // Au-delà de l'espace Unicode, la valeur ne compte plus : U+FFFD.
            code = code
                .saturating_mul(if hex { 16 } else { 10 })
                .saturating_add(digit)
                .min(0x11_0000);
            index += 1;
        }
        if index == digits_start {
            return None;
        }
        if bytes.get(index) == Some(&b';') {
            index += 1;
        }
        return Some((index - at, replace_code_point(code).to_string()));
    }
    let run = bytes[at.min(bytes.len())..]
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric())
        .count();
    if run == 0 {
        return None;
    }
    if run <= MAX_ENTITY_NAME
        && bytes.get(at + run) == Some(&b';')
        && let Some((value, _)) = lookup_entity(&src[at..at + run])
    {
        return Some((run + 1, value.to_owned()));
    }
    for length in (1..=run.min(MAX_ENTITY_NAME)).rev() {
        if let Some((value, true)) = lookup_entity(&src[at..at + length]) {
            if mode == EntityMode::Attribute
                && bytes
                    .get(at + length)
                    .is_some_and(|b| *b == b'=' || b.is_ascii_alphanumeric())
            {
                return None;
            }
            return Some((length, value.to_owned()));
        }
    }
    None
}

// ── Tables de htmlparser2 ─────────────────────────────────────────────────

fn open_implies_close(name: &str) -> &'static [&'static str] {
    const P: &[&str] = &["p"];
    const HEADINGS: &[&str] = &["h1", "h2", "h3", "h4", "h5", "h6", "p"];
    const FORM: &[&str] = &[
        "input", "option", "optgroup", "select", "button", "datalist", "textarea",
    ];
    match name {
        "tr" => &["tr", "th", "td"],
        "th" => &["th"],
        "td" => &["thead", "th", "td"],
        "body" => &["head", "link", "script"],
        "a" => &["a"],
        "li" => &["li"],
        "p" | "address" | "article" | "aside" | "blockquote" | "details" | "div" | "dl"
        | "fieldset" | "figcaption" | "figure" | "footer" | "form" | "header" | "hr" | "main"
        | "nav" | "ol" | "pre" | "section" | "table" | "ul" => P,
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => HEADINGS,
        "select" | "input" | "output" | "button" | "datalist" | "textarea" => FORM,
        "option" => &["option"],
        "optgroup" => &["optgroup", "option"],
        "dd" | "dt" => &["dd", "dt"],
        "rt" | "rp" => &["rt", "rp"],
        "tbody" | "tfoot" => &["thead", "tbody"],
        _ => &[],
    }
}

pub(crate) fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "basefont"
            | "br"
            | "col"
            | "command"
            | "embed"
            | "frame"
            | "hr"
            | "img"
            | "input"
            | "isindex"
            | "keygen"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn is_html_integration(name: &str) -> bool {
    matches!(
        name,
        "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml" | "foreignObject" | "desc" | "title"
    )
}

fn svg_adjusted(name: &str) -> Option<&'static str> {
    Some(match name {
        "altglyph" => "altGlyph",
        "altglyphdef" => "altGlyphDef",
        "altglyphitem" => "altGlyphItem",
        "animatecolor" => "animateColor",
        "animatemotion" => "animateMotion",
        "animatetransform" => "animateTransform",
        "clippath" => "clipPath",
        "feblend" => "feBlend",
        "fecolormatrix" => "feColorMatrix",
        "fecomponenttransfer" => "feComponentTransfer",
        "fecomposite" => "feComposite",
        "feconvolvematrix" => "feConvolveMatrix",
        "fediffuselighting" => "feDiffuseLighting",
        "fedisplacementmap" => "feDisplacementMap",
        "fedistantlight" => "feDistantLight",
        "fedropshadow" => "feDropShadow",
        "feflood" => "feFlood",
        "fefunca" => "feFuncA",
        "fefuncb" => "feFuncB",
        "fefuncg" => "feFuncG",
        "fefuncr" => "feFuncR",
        "fegaussianblur" => "feGaussianBlur",
        "feimage" => "feImage",
        "femerge" => "feMerge",
        "femergenode" => "feMergeNode",
        "femorphology" => "feMorphology",
        "feoffset" => "feOffset",
        "fepointlight" => "fePointLight",
        "fespecularlighting" => "feSpecularLighting",
        "fespotlight" => "feSpotLight",
        "fetile" => "feTile",
        "feturbulence" => "feTurbulence",
        "foreignobject" => "foreignObject",
        "glyphref" => "glyphRef",
        "lineargradient" => "linearGradient",
        "radialgradient" => "radialGradient",
        "textpath" => "textPath",
        _ => return None,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Foreign {
    None,
    Svg,
    MathMl,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Text,
    BeforeTagName,
    InTagName,
    InSelfClosingTag,
    BeforeClosingTagName,
    InClosingTagName,
    AfterClosingTagName,
    BeforeAttributeName,
    InAttributeName,
    AfterAttributeName,
    BeforeAttributeValue,
    InAttributeValueDq,
    InAttributeValueSq,
    InAttributeValueNq,
    BeforeDeclaration,
    InDeclaration,
    BeforeComment,
    CdataSequence,
    DeclarationSequence,
    InSpecialComment,
    InCommentLike,
    SpecialStartSequence,
    InSpecialTag,
    InPlainText,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sequence {
    Empty,
    CdataEnd,
    CommentEnd,
    Doctype,
    Iframe,
    Noembed,
    Noframes,
    Plaintext,
    Script,
    Style,
    Title,
    Textarea,
    Xmp,
}

impl Sequence {
    fn bytes(self) -> &'static [u8] {
        match self {
            Self::Empty => b"",
            Self::CdataEnd => b"]]>",
            Self::CommentEnd => b"--!>",
            Self::Doctype => b"doctype",
            Self::Iframe => b"</iframe",
            Self::Noembed => b"</noembed",
            Self::Noframes => b"</noframes",
            Self::Plaintext => b"</plaintext",
            Self::Script => b"</script",
            Self::Style => b"</style",
            Self::Title => b"</title",
            Self::Textarea => b"</textarea",
            Self::Xmp => b"</xmp",
        }
    }

    fn special_start(first: u8) -> Option<Self> {
        match first {
            b'i' => Some(Self::Iframe),
            b'n' => Some(Self::Noembed),
            b'p' => Some(Self::Plaintext),
            b's' => Some(Self::Script),
            b't' => Some(Self::Title),
            b'x' => Some(Self::Xmp),
            _ => None,
        }
    }
}

const CDATA_START: &[u8] = b"CDATA[";

fn is_whitespace(c: u8) -> bool {
    matches!(c, b' ' | b'\n' | b'\t' | b'\x0c' | b'\r')
}

fn is_end_of_tag_section(c: u8) -> bool {
    c == b'/' || c == b'>' || is_whitespace(c)
}

/// `sectionStart` à -1 dans htmlparser2.
const NO_SECTION: usize = usize::MAX;

struct Parser<'a, 'h, H: Handler> {
    src: &'a str,
    bytes: &'a [u8],
    handler: &'h mut H,
    // Lecture
    state: State,
    section_start: usize,
    index: usize,
    is_special: bool,
    sequence: Sequence,
    sequence_index: usize,
    // Arbre
    start_index: usize,
    end_index: usize,
    open_tag_start: usize,
    tagname: String,
    attribname: String,
    attribvalue: String,
    attribs: Option<Vec<(String, String)>>,
    attrib_names: HashSet<String>,
    stack: Vec<String>,
    counts: HashMap<String, usize>,
    foreign: Vec<Foreign>,
}

impl<'a, 'h, H: Handler> Parser<'a, 'h, H> {
    fn new(src: &'a str, handler: &'h mut H) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            handler,
            state: State::Text,
            section_start: 0,
            index: 0,
            is_special: false,
            sequence: Sequence::Empty,
            sequence_index: 0,
            start_index: 0,
            end_index: 0,
            open_tag_start: 0,
            tagname: String::new(),
            attribname: String::new(),
            attribvalue: String::new(),
            attribs: None,
            attrib_names: HashSet::new(),
            stack: Vec::new(),
            counts: HashMap::new(),
            foreign: vec![Foreign::None],
        }
    }

    fn slice(&self, start: usize, end: usize) -> &'a str {
        self.src.get(start..end).unwrap_or_default()
    }

    fn in_foreign(&self) -> bool {
        self.foreign
            .last()
            .is_some_and(|context| *context != Foreign::None)
    }

    fn run(&mut self) {
        while self.index < self.bytes.len() {
            let c = self.bytes[self.index];
            match self.state {
                State::Text => self.state_text(c),
                State::InPlainText => self.index = self.bytes.len() - 1,
                State::SpecialStartSequence => self.state_special_start_sequence(c),
                State::InSpecialTag => self.state_in_special_tag(c),
                State::CdataSequence => self.state_cdata_sequence(c),
                State::DeclarationSequence => self.state_declaration_sequence(c),
                State::InAttributeValueDq => self.state_in_attribute_value(c, b'"'),
                State::InAttributeValueSq => self.state_in_attribute_value(c, b'\''),
                State::InAttributeValueNq => self.state_in_attribute_value_nq(c),
                State::InAttributeName => self.state_in_attribute_name(c),
                State::InCommentLike => self.state_in_comment_like(c),
                State::InSpecialComment => self.state_in_special_comment(c),
                State::BeforeAttributeName => self.state_before_attribute_name(c),
                State::InTagName => self.state_in_tag_name(c),
                State::InClosingTagName => self.state_in_closing_tag_name(c),
                State::BeforeTagName => self.state_before_tag_name(c),
                State::AfterAttributeName => self.state_after_attribute_name(c),
                State::BeforeAttributeValue => self.state_before_attribute_value(c),
                State::BeforeClosingTagName => self.state_before_closing_tag_name(c),
                State::AfterClosingTagName => self.state_after_closing_tag_name(c),
                State::InSelfClosingTag => self.state_in_self_closing_tag(c),
                State::InDeclaration => self.state_in_declaration(c),
                State::BeforeDeclaration => self.state_before_declaration(c),
                State::BeforeComment => self.state_before_comment(c),
            }
            self.index += 1;
        }
        self.finish();
    }

    // ── États ──────────────────────────────────────────────────────────────

    fn state_text(&mut self, c: u8) {
        if c == b'<' {
            if self.index > self.section_start {
                self.ontext(self.section_start, self.index);
            }
            self.state = State::BeforeTagName;
            self.section_start = self.index;
        } else if c == b'&' {
            self.text_entity();
        }
    }

    /// Une référence dans du texte (ou un élément RCDATA) : le texte d'avant, puis elle.
    fn text_entity(&mut self) {
        let entity_start = self.index;
        if let Some((length, decoded)) =
            decode_reference(self.src, entity_start + 1, EntityMode::Text)
        {
            if self.section_start < entity_start {
                self.ontext(self.section_start, entity_start);
            }
            self.section_start = entity_start + 1 + length;
            self.index = self.section_start - 1;
            self.end_index = self.section_start - 1;
            self.handler.text(&decoded, self.start_index);
            self.start_index = self.section_start;
        }
    }

    fn attribute_entity(&mut self) {
        let entity_start = self.index;
        if let Some((length, decoded)) =
            decode_reference(self.src, entity_start + 1, EntityMode::Attribute)
        {
            if self.section_start < entity_start {
                let data = self.slice(self.section_start, entity_start);
                self.attribvalue.push_str(data);
            }
            self.section_start = entity_start + 1 + length;
            self.index = self.section_start - 1;
            self.attribvalue.push_str(&decoded);
        }
    }

    fn enter_tag_body(&mut self) {
        if self.sequence == Sequence::Plaintext {
            self.sequence = Sequence::Empty;
            self.state = State::InPlainText;
        } else if self.is_special {
            self.state = State::InSpecialTag;
            self.sequence_index = 0;
        } else {
            self.state = State::Text;
        }
    }

    fn state_special_start_sequence(&mut self, c: u8) {
        let lower = c | 0x20;
        let sequence = self.sequence.bytes();
        if self.sequence_index < sequence.len() {
            if sequence.get(self.sequence_index) == Some(&lower) {
                self.sequence_index += 1;
                return;
            }
            if self.sequence_index == 3 {
                if self.sequence == Sequence::Script && lower == b't' {
                    self.sequence = Sequence::Style;
                    self.sequence_index = 4;
                    return;
                }
                if self.sequence == Sequence::Title && lower == b'e' {
                    self.sequence = Sequence::Textarea;
                    self.sequence_index = 4;
                    return;
                }
            } else if self.sequence_index == 4
                && self.sequence == Sequence::Noembed
                && lower == b'f'
            {
                self.sequence = Sequence::Noframes;
                self.sequence_index = 5;
                return;
            }
        } else if is_end_of_tag_section(c) {
            self.sequence_index = 0;
            self.state = State::InTagName;
            self.state_in_tag_name(c);
            return;
        }
        self.is_special = false;
        self.sequence = Sequence::Empty;
        self.sequence_index = 0;
        self.state = State::InTagName;
        self.state_in_tag_name(c);
    }

    fn state_cdata_sequence(&mut self, c: u8) {
        if CDATA_START.get(self.sequence_index) == Some(&c) {
            self.sequence_index += 1;
            if self.sequence_index == CDATA_START.len() {
                self.state = State::InCommentLike;
                self.sequence = Sequence::CdataEnd;
                self.sequence_index = 0;
                self.section_start = self.index + 1;
            }
        } else {
            self.sequence_index = 0;
            self.state = State::InSpecialComment;
            self.state_in_special_comment(c);
        }
    }

    /// Avance jusqu'au prochain `target` : vrai s'il est trouvé (l'indice y est), faux sinon
    /// (l'indice est au dernier octet).
    fn fast_forward_to(&mut self, target: u8) -> bool {
        while self.index + 1 < self.bytes.len() {
            self.index += 1;
            if self.bytes[self.index] == target {
                return true;
            }
        }
        self.index = self.bytes.len().saturating_sub(1);
        false
    }

    fn emit_comment(&mut self, _offset: usize) {
        self.oncomment();
        self.sequence_index = 0;
        self.section_start = self.index + 1;
        self.state = State::Text;
    }

    fn state_in_comment_like(&mut self, c: u8) {
        let sequence = self.sequence.bytes();
        let comment = self.sequence == Sequence::CommentEnd;
        if comment
            && self.sequence_index <= 1
            && self.index == self.section_start.wrapping_add(self.sequence_index)
            && c == b'>'
        {
            self.emit_comment(self.sequence_index);
        } else if comment && self.sequence_index == 2 && c == b'>' {
            self.emit_comment(2);
        } else if comment && self.sequence_index == sequence.len() - 1 && c != b'>' {
            self.sequence_index = usize::from(c == b'-');
        } else if sequence.get(self.sequence_index) == Some(&c) {
            self.sequence_index += 1;
            if self.sequence_index == sequence.len() {
                if self.sequence == Sequence::CdataEnd {
                    self.oncdata(self.section_start, self.index - 2);
                } else {
                    self.oncomment();
                }
                self.sequence_index = 0;
                self.section_start = self.index + 1;
                self.state = State::Text;
            }
        } else if self.sequence_index == 0 {
            if let Some(&first) = sequence.first()
                && self.fast_forward_to(first)
            {
                self.sequence_index = 1;
            }
        } else if sequence.get(self.sequence_index - 1) != Some(&c) {
            self.sequence_index = 0;
        }
    }

    fn state_in_special_tag(&mut self, c: u8) {
        let sequence = self.sequence.bytes();
        if self.sequence_index == sequence.len() {
            if is_end_of_tag_section(c) {
                let end_of_text = self.index - sequence.len();
                if self.section_start < end_of_text {
                    self.ontext(self.section_start, end_of_text);
                }
                self.is_special = false;
                self.section_start = end_of_text + 2;
                self.state_in_closing_tag_name(c);
                return;
            }
            self.sequence_index = 0;
        }
        if sequence.get(self.sequence_index) == Some(&(c | 0x20)) {
            self.sequence_index += 1;
        } else if self.sequence_index == 0 {
            if matches!(self.sequence, Sequence::Title | Sequence::Textarea) {
                if c == b'&' {
                    self.text_entity();
                }
            } else if self.fast_forward_to(b'<') {
                self.sequence_index = 1;
            }
        } else {
            self.sequence_index = usize::from(c == b'<');
        }
    }

    fn state_before_tag_name(&mut self, c: u8) {
        if c == b'!' {
            self.state = State::BeforeDeclaration;
            self.section_start = self.index + 1;
        } else if c == b'?' {
            self.state = State::InSpecialComment;
            self.section_start = self.index;
        } else if c.is_ascii_alphabetic() {
            self.section_start = self.index;
            let special = if self.in_foreign() {
                None
            } else {
                Sequence::special_start(c | 0x20)
            };
            if let Some(sequence) = special {
                self.is_special = true;
                self.sequence = sequence;
                self.sequence_index = 3;
                self.state = State::SpecialStartSequence;
            } else {
                self.state = State::InTagName;
            }
        } else if c == b'/' {
            self.state = State::BeforeClosingTagName;
        } else {
            self.state = State::Text;
            self.state_text(c);
        }
    }

    fn state_in_tag_name(&mut self, c: u8) {
        if is_end_of_tag_section(c) {
            self.onopentagname(self.section_start, self.index);
            self.section_start = NO_SECTION;
            self.state = State::BeforeAttributeName;
            self.state_before_attribute_name(c);
        }
    }

    fn state_before_closing_tag_name(&mut self, c: u8) {
        if is_whitespace(c) {
            self.state = State::InSpecialComment;
            self.section_start = self.index;
        } else if c == b'>' {
            self.state = State::Text;
            self.section_start = self.index + 1;
        } else {
            self.state = if c.is_ascii_alphabetic() {
                State::InClosingTagName
            } else {
                State::InSpecialComment
            };
            self.section_start = self.index;
        }
    }

    fn state_in_closing_tag_name(&mut self, c: u8) {
        if is_end_of_tag_section(c) {
            self.onclosetag(self.section_start, self.index);
            self.section_start = NO_SECTION;
            self.state = State::AfterClosingTagName;
            self.state_after_closing_tag_name(c);
        }
    }

    fn state_after_closing_tag_name(&mut self, c: u8) {
        if c == b'>' || self.fast_forward_to(b'>') {
            self.state = State::Text;
            self.section_start = self.index + 1;
        }
    }

    fn state_before_attribute_name(&mut self, c: u8) {
        if c == b'>' {
            self.onopentagend(self.index);
            self.enter_tag_body();
            self.section_start = self.index + 1;
        } else if c == b'/' {
            self.state = State::InSelfClosingTag;
        } else if !is_whitespace(c) {
            self.state = State::InAttributeName;
            self.section_start = self.index;
        }
    }

    fn state_in_self_closing_tag(&mut self, c: u8) {
        if c == b'>' {
            self.onselfclosingtag(self.index);
            self.section_start = self.index + 1;
            self.enter_tag_body();
        } else if !is_whitespace(c) {
            self.state = State::BeforeAttributeName;
            self.state_before_attribute_name(c);
        }
    }

    fn state_in_attribute_name(&mut self, c: u8) {
        if c == b'=' || is_end_of_tag_section(c) {
            self.onattribname(self.section_start, self.index);
            self.section_start = self.index;
            self.state = State::AfterAttributeName;
            self.state_after_attribute_name(c);
        }
    }

    fn state_after_attribute_name(&mut self, c: u8) {
        if c == b'=' {
            self.state = State::BeforeAttributeValue;
        } else if c == b'/' || c == b'>' {
            self.onattribend();
            self.section_start = NO_SECTION;
            self.state = State::BeforeAttributeName;
            self.state_before_attribute_name(c);
        } else if !is_whitespace(c) {
            self.onattribend();
            self.state = State::InAttributeName;
            self.section_start = self.index;
        }
    }

    fn state_before_attribute_value(&mut self, c: u8) {
        if c == b'"' {
            self.state = State::InAttributeValueDq;
            self.section_start = self.index + 1;
        } else if c == b'\'' {
            self.state = State::InAttributeValueSq;
            self.section_start = self.index + 1;
        } else if !is_whitespace(c) {
            self.section_start = self.index;
            self.state = State::InAttributeValueNq;
            self.state_in_attribute_value_nq(c);
        }
    }

    fn state_in_attribute_value(&mut self, c: u8, quote: u8) {
        if c == quote {
            self.onattribdata(self.section_start, self.index);
            self.section_start = NO_SECTION;
            self.onattribend();
            self.state = State::BeforeAttributeName;
        } else if c == b'&' {
            self.attribute_entity();
        }
    }

    fn state_in_attribute_value_nq(&mut self, c: u8) {
        if is_whitespace(c) || c == b'>' {
            self.onattribdata(self.section_start, self.index);
            self.section_start = NO_SECTION;
            self.onattribend();
            self.state = State::BeforeAttributeName;
            self.state_before_attribute_name(c);
        } else if c == b'&' {
            self.attribute_entity();
        }
    }

    fn state_before_declaration(&mut self, c: u8) {
        if c == b'[' {
            self.state = State::CdataSequence;
            self.sequence_index = 0;
        } else if c | 0x20 == b'd' {
            self.state = State::DeclarationSequence;
            self.sequence = Sequence::Doctype;
            self.sequence_index = 1;
        } else if c == b'>' {
            self.oncomment();
            self.state = State::Text;
            self.section_start = self.index + 1;
        } else if c == b'-' {
            self.state = State::BeforeComment;
        } else {
            self.state = State::InSpecialComment;
        }
    }

    fn state_declaration_sequence(&mut self, c: u8) {
        let sequence = Sequence::Doctype.bytes();
        if self.sequence_index == sequence.len() {
            self.state = State::InDeclaration;
            self.state_in_declaration(c);
        } else if sequence.get(self.sequence_index) == Some(&(c | 0x20)) {
            self.sequence_index += 1;
        } else if c == b'>' {
            self.oncomment();
            self.state = State::Text;
            self.section_start = self.index + 1;
        } else {
            self.state = State::InSpecialComment;
        }
    }

    fn state_in_declaration(&mut self, c: u8) {
        if c == b'>' || self.fast_forward_to(b'>') {
            // Un doctype : rien à rapporter.
            self.end_index = self.index;
            self.start_index = self.index + 1;
            self.state = State::Text;
            self.section_start = self.index + 1;
        }
    }

    fn state_before_comment(&mut self, c: u8) {
        if c == b'-' {
            self.state = State::InCommentLike;
            self.sequence = Sequence::CommentEnd;
            self.sequence_index = 0;
            self.section_start = self.index + 1;
        } else if c == b'>' {
            self.oncomment();
            self.state = State::Text;
            self.section_start = self.index + 1;
        } else {
            self.state = State::InSpecialComment;
        }
    }

    fn state_in_special_comment(&mut self, c: u8) {
        if c == b'>' || self.fast_forward_to(b'>') {
            self.oncomment();
            self.state = State::Text;
            self.section_start = self.index + 1;
        }
    }

    /// La fin du texte (`finish` et `handleTrailingData`).
    fn finish(&mut self) {
        let end = self.bytes.len();
        // Ce qui reste d'un texte, d'un élément de texte brut.
        if (matches!(self.state, State::Text | State::InPlainText)
            || (self.state == State::InSpecialTag && self.sequence_index == 0))
            && self.section_start != NO_SECTION
            && self.section_start < end
        {
            self.ontext(self.section_start, end);
            self.section_start = end;
        }
        match self.state {
            // Commentaires, doctype, CDATA non fermés : rien.
            State::InCommentLike
            | State::BeforeDeclaration
            | State::InSpecialComment
            | State::BeforeComment
            | State::CdataSequence
            | State::DeclarationSequence
            | State::InDeclaration
            // Une balise interrompue : ignorée.
            | State::InTagName
            | State::BeforeAttributeName
            | State::BeforeAttributeValue
            | State::AfterAttributeName
            | State::InAttributeName
            | State::InAttributeValueSq
            | State::InAttributeValueDq
            | State::InAttributeValueNq
            | State::InClosingTagName => {}
            _ => {
                if self.section_start == NO_SECTION {
                    // htmlparser2 lit alors `slice(-1)` : le dernier caractère.
                    if let Some((at, _)) = self.src.char_indices().next_back() {
                        self.ontext(at, end);
                    }
                } else if self.section_start < end {
                    self.ontext(self.section_start, end);
                }
            }
        }
        self.onend();
    }

    // ── Arbre (Parser de htmlparser2) ─────────────────────────────────────

    fn ontext(&mut self, start: usize, end: usize) {
        let data = self.slice(start, end);
        self.end_index = end.saturating_sub(1);
        self.handler.text(data, self.start_index);
        self.start_index = end;
    }

    fn read_tag_name(&self, start: usize, end: usize) -> String {
        let name = self.slice(start, end).to_lowercase();
        if self.foreign.last() == Some(&Foreign::Svg) {
            return svg_adjusted(&name).map_or(name, str::to_owned);
        }
        if self.foreign.len() > 1
            && let Some(adjusted) = svg_adjusted(&name)
            && self.counts.get(adjusted).is_some_and(|count| *count > 0)
        {
            return adjusted.to_owned();
        }
        if !self.in_foreign() && name == "image" {
            return "img".to_owned();
        }
        name
    }

    fn onopentagname(&mut self, start: usize, end: usize) {
        self.end_index = end;
        let name = self.read_tag_name(start, end);
        self.emit_open_tag(name);
    }

    fn push(&mut self, name: String) {
        *self.counts.entry(name.clone()).or_insert(0) += 1;
        self.stack.push(name);
    }

    fn emit_open_tag(&mut self, name: String) {
        self.open_tag_start = self.start_index;
        if name == "form" && self.counts.get("form").is_some_and(|count| *count > 0) {
            self.tagname = String::new();
            self.attribs = None;
            return;
        }
        let implied = open_implies_close(&name);
        while self
            .stack
            .last()
            .is_some_and(|top| implied.contains(&top.as_str()))
        {
            self.pop_element(true);
        }
        if !is_void_element(&name) {
            if name == "svg" {
                self.foreign.push(Foreign::Svg);
            } else if name == "math" {
                self.foreign.push(Foreign::MathMl);
            } else if is_html_integration(&name) {
                self.foreign.push(Foreign::None);
            }
            self.push(name.clone());
        }
        self.tagname = name;
        self.attribs = Some(Vec::new());
        self.attrib_names.clear();
    }

    fn end_open_tag(&mut self) {
        self.start_index = self.open_tag_start;
        if let Some(attributes) = self.attribs.take() {
            let name = self.tagname.clone();
            self.handler
                .open_tag(&name, attributes, self.start_index, self.end_index);
        }
        if is_void_element(&self.tagname) {
            let name = self.tagname.clone();
            self.handler.close_tag(&name, true, self.start_index);
        }
        self.tagname.clear();
    }

    fn onopentagend(&mut self, end: usize) {
        self.end_index = end;
        self.end_open_tag();
        self.start_index = end + 1;
    }

    fn onclosetag(&mut self, start: usize, end: usize) {
        self.end_index = end;
        let name = self.read_tag_name(start, end);
        if !is_void_element(&name) {
            if self.counts.get(&name).is_some_and(|count| *count > 0) {
                while let Some(top) = self.stack.last() {
                    let found = *top == name;
                    self.pop_element(!found);
                    if found {
                        break;
                    }
                }
            } else if name == "p" {
                self.emit_open_tag("p".to_owned());
                self.close_current_tag(true);
            }
        } else if name == "br" {
            self.handler
                .open_tag("br", Vec::new(), self.start_index, self.end_index);
            self.handler.close_tag("br", false, self.start_index);
        }
        self.start_index = end + 1;
    }

    fn onselfclosingtag(&mut self, end: usize) {
        self.end_index = end;
        if self.in_foreign() {
            self.close_current_tag(false);
            self.start_index = end + 1;
        } else {
            self.onopentagend(end);
        }
    }

    fn pop_element(&mut self, implied: bool) {
        let Some(element) = self.stack.pop() else {
            return;
        };
        if let Some(count) = self.counts.get_mut(&element) {
            *count = count.saturating_sub(1);
        }
        if matches!(element.as_str(), "math" | "svg") || is_html_integration(&element) {
            self.foreign.pop();
        }
        self.handler.close_tag(&element, implied, self.start_index);
    }

    fn close_current_tag(&mut self, is_open_implied: bool) {
        let name = self.tagname.clone();
        self.end_open_tag();
        if !name.is_empty() && self.stack.last() == Some(&name) {
            self.pop_element(!is_open_implied);
        }
    }

    fn onattribname(&mut self, start: usize, end: usize) {
        self.start_index = start;
        self.attribname = self.slice(start, end).to_lowercase();
    }

    fn onattribdata(&mut self, start: usize, end: usize) {
        if start < end {
            let data = self.slice(start, end);
            self.attribvalue.push_str(data);
        }
    }

    fn onattribend(&mut self) {
        let value = std::mem::take(&mut self.attribvalue);
        let name = std::mem::take(&mut self.attribname);
        if let Some(attributes) = self.attribs.as_mut()
            && self.attrib_names.insert(name.clone())
        {
            attributes.push((name, value));
        }
    }

    fn oncomment(&mut self) {
        self.end_index = self.index;
        self.start_index = self.index + 1;
    }

    fn oncdata(&mut self, start: usize, end: usize) {
        self.end_index = self.index;
        if self.in_foreign() {
            let value = self.slice(start, end);
            self.handler.text(value, self.start_index);
        }
        self.start_index = self.index + 1;
    }

    fn onend(&mut self) {
        self.end_index = self.start_index;
        let open: Vec<String> = self.stack.iter().rev().cloned().collect();
        for name in open {
            self.handler.close_tag(&name, true, self.start_index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Log(Vec<String>);

    impl Handler for Log {
        fn open_tag(&mut self, name: &str, attributes: Vec<(String, String)>, _: usize, _: usize) {
            let attributes: Vec<String> = attributes
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect();
            self.0.push(format!("<{name} {}>", attributes.join(",")));
        }
        fn text(&mut self, text: &str, _: usize) {
            if let Some(last) = self.0.last_mut()
                && last.starts_with('"')
            {
                last.pop();
                last.push_str(text);
                last.push('"');
                return;
            }
            self.0.push(format!("\"{text}\""));
        }
        fn close_tag(&mut self, name: &str, implied: bool, _: usize) {
            self.0
                .push(format!("</{name}{}>", if implied { "?" } else { "" }));
        }
    }

    fn events(html: &str) -> String {
        let mut log = Log::default();
        parse(html, &mut log);
        log.0.join("")
    }

    #[test]
    fn balises_et_attributs() {
        assert_eq!(
            events("<P Class=x id='y' z=\"a&amp;b\" w>t</p>"),
            "<p class=x,id=y,z=a&b,w=>\"t\"</p>"
        );
        assert_eq!(events("<p a=1 a=2>"), "<p a=1></p?>");
        assert_eq!(events("<br/><img src=x>"), "<br ></br?><img src=x></img?>");
        assert_eq!(events("a</br>b"), "\"a\"<br ></br>\"b\"");
        assert_eq!(events("a</p>b"), "\"a\"<p ></p>\"b\"");
        assert_eq!(events("<image src=x>"), "<img src=x></img?>");
    }

    #[test]
    fn texte_brut_et_commentaires() {
        assert_eq!(
            events("<style>a<b>&amp;</style>x"),
            "<style >\"a<b>&amp;\"</style>\"x\""
        );
        assert_eq!(events("<title>&amp;</title>"), "<title >\"&\"</title>");
        assert_eq!(events("a<!-- b -->c<!x>d<?p?>e"), "\"acde\"");
        assert_eq!(events("<svg><![CDATA[x]]></svg>"), "<svg >\"x\"</svg>");
        assert_eq!(
            events("<svg><style><b>x</b></style></svg>"),
            "<svg ><style ><b >\"x\"</b></style></svg>"
        );
    }

    #[test]
    fn fins_de_texte() {
        assert_eq!(events("<p>a</div xyz"), "<p >\"az\"</p?>");
        assert_eq!(events("<p>a<br/"), "<p >\"a/\"</p?>");
        assert_eq!(events("<p>a<scr"), "<p >\"ascr\"</p?>");
        assert_eq!(events("<p>a<div"), "<p >\"a\"</p?>");
        assert_eq!(events("a<"), "\"a<\"");
    }

    #[test]
    fn references() {
        let text = |s: &str| decode_reference(s, 0, EntityMode::Text).map(|(_, v)| v);
        let attribute = |s: &str| decode_reference(s, 0, EntityMode::Attribute).map(|(_, v)| v);
        assert_eq!(text("notit;").as_deref(), Some("\u{ac}"));
        assert_eq!(attribute("notit;"), None);
        assert_eq!(text("notin;").as_deref(), Some("\u{2209}"));
        assert_eq!(text("#x6a;").as_deref(), Some("j"));
        assert_eq!(text("#128;").as_deref(), Some("\u{20ac}"));
        assert_eq!(text("#99999999999;").as_deref(), Some("\u{fffd}"));
        assert_eq!(text("#;"), None);
        assert_eq!(attribute("amp=x"), None);
        assert_eq!(attribute("amp x").as_deref(), Some("&"));
    }
}
