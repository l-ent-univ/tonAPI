//! Les attributs `style`, lus comme sanitize-html les lit : il passe `balise {valeur}` à postcss
//! (8.5), garde les déclarations de la première règle et les réécrit `prop:valeur` séparées par
//! `;`. Une erreur de postcss (chaîne, commentaire ou parenthèse non fermés, mot inconnu,
//! point-virgule manquant, accolade en trop) retire l'attribut entier ; un commentaire ou une règle
//! imbriquée devient `undefined:undefined`, comme dans Tafac.
//!
//! Seul ce qui change la sortie est repris de postcss : son découpage en lexèmes, la lecture d'une
//! déclaration (`!important` compris) et des règles. Linéaire : la recherche de `)` est mise en
//! cache.

/// Une entrée de la première règle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StyleNode {
    /// `prop: valeur`.
    Declaration {
        prop: String,
        value: String,
        important: bool,
    },
    /// Un commentaire, une règle ou une règle `@` imbriqués.
    Other,
}

impl StyleNode {
    /// Comme sanitize-html l'écrit dans l'attribut.
    pub(crate) fn write(&self, out: &mut String) {
        match self {
            Self::Declaration {
                prop,
                value,
                important,
            } => {
                out.push_str(prop);
                out.push(':');
                out.push_str(value);
                if *important {
                    out.push_str(" !important");
                }
            }
            Self::Other => out.push_str("undefined:undefined"),
        }
    }
}

/// Les entrées de `selector {value}`, ou `None` quand postcss refuse le texte.
pub(crate) fn parse_style(selector: &str, value: &str) -> Option<Vec<StyleNode>> {
    let css = format!("{selector} {{{value}}}");
    let mut parser = CssParser {
        tokens: Tokenizer::new(&css),
        open: Vec::new(),
        root_count: 0,
        first_is_container: false,
        out: Vec::new(),
    };
    parser.parse().ok()?;
    if parser.root_count == 0 || !parser.first_is_container {
        return None;
    }
    Some(parser.out)
}

// ── Lexèmes (lib/tokenize.js) ─────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Space,
    OpenSquare,
    CloseSquare,
    OpenCurly,
    CloseCurly,
    Colon,
    Semicolon,
    OpenParen,
    CloseParen,
    Brackets,
    String,
    AtWord,
    Word,
    Comment,
}

#[derive(Clone, Copy, Debug)]
struct Token<'a> {
    kind: Kind,
    text: &'a str,
}

struct Error;

struct Tokenizer<'a> {
    css: &'a str,
    bytes: &'a [u8],
    pos: usize,
    buffer: Vec<&'a str>,
    returned: Vec<Token<'a>>,
    last_bad_paren: Option<usize>,
    /// La dernière recherche de `)` : depuis où, et où.
    close_cache: Option<(usize, Option<usize>)>,
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\n' | b'\t' | b'\r' | b'\x0c')
}

impl<'a> Tokenizer<'a> {
    fn new(css: &'a str) -> Self {
        Self {
            css,
            bytes: css.as_bytes(),
            pos: 0,
            buffer: Vec::new(),
            returned: Vec::new(),
            last_bad_paren: None,
            close_cache: None,
        }
    }

    fn end_of_file(&self) -> bool {
        self.returned.is_empty() && self.pos >= self.bytes.len()
    }

    fn back(&mut self, token: Token<'a>) {
        self.returned.push(token);
    }

    fn slice(&self, start: usize, end: usize) -> &'a str {
        self.css.get(start..end).unwrap_or_default()
    }

    /// `css.indexOf(')', from)`, mise en cache : les recherches avancent.
    fn find_close(&mut self, from: usize) -> Option<usize> {
        if let Some((cached_from, found)) = self.close_cache
            && cached_from <= from
            && found.is_none_or(|at| at >= from)
        {
            return found;
        }
        let found = self.bytes[from.min(self.bytes.len())..]
            .iter()
            .position(|b| *b == b')')
            .map(|offset| from + offset);
        self.close_cache = Some((from, found));
        found
    }

    fn escaped_before(&self, at: usize) -> bool {
        let mut escaped = false;
        let mut index = at;
        while index > 0 && self.bytes[index - 1] == b'\\' {
            index -= 1;
            escaped = !escaped;
        }
        escaped
    }

    #[allow(clippy::too_many_lines)]
    fn next_token(&mut self) -> Result<Option<Token<'a>>, Error> {
        if let Some(token) = self.returned.pop() {
            return Ok(Some(token));
        }
        let length = self.bytes.len();
        if self.pos >= length {
            return Ok(None);
        }
        let pos = self.pos;
        let code = self.bytes[pos];
        let token;
        let mut next = pos;
        match code {
            b'\n' | b' ' | b'\t' | b'\r' | b'\x0c' => {
                next = pos + 1;
                while next < length && is_space(self.bytes[next]) {
                    next += 1;
                }
                token = Token {
                    kind: Kind::Space,
                    text: self.slice(pos, next),
                };
                next -= 1;
            }
            b'[' | b']' | b'{' | b'}' | b':' | b';' | b')' => {
                let kind = match code {
                    b'[' => Kind::OpenSquare,
                    b']' => Kind::CloseSquare,
                    b'{' => Kind::OpenCurly,
                    b'}' => Kind::CloseCurly,
                    b':' => Kind::Colon,
                    b';' => Kind::Semicolon,
                    _ => Kind::CloseParen,
                };
                token = Token {
                    kind,
                    text: self.slice(pos, pos + 1),
                };
            }
            b'(' => {
                let prev = self.buffer.pop().unwrap_or_default();
                let n = self.bytes.get(pos + 1).copied();
                if prev == "url"
                    && !matches!(
                        n,
                        Some(b'\'' | b'"' | b' ' | b'\n' | b'\t' | b'\x0c' | b'\r')
                    )
                {
                    let mut search = pos;
                    loop {
                        let Some(found) = self.find_close(search + 1) else {
                            return Err(Error);
                        };
                        search = found;
                        if !self.escaped_before(found) {
                            break;
                        }
                    }
                    next = search;
                    token = Token {
                        kind: Kind::Brackets,
                        text: self.slice(pos, next + 1),
                    };
                } else if self.last_bad_paren.is_some_and(|bad| pos <= bad) {
                    token = Token {
                        kind: Kind::OpenParen,
                        text: "(",
                    };
                } else {
                    let close = self.find_close(pos + 1);
                    let bad = match close {
                        None => true,
                        Some(close) => bad_bracket(self.slice(pos, close + 1)),
                    };
                    if bad {
                        self.last_bad_paren = Some(close.unwrap_or(length));
                        token = Token {
                            kind: Kind::OpenParen,
                            text: "(",
                        };
                    } else {
                        let close = close.unwrap_or(pos);
                        next = close;
                        token = Token {
                            kind: Kind::Brackets,
                            text: self.slice(pos, close + 1),
                        };
                    }
                }
            }
            b'\'' | b'"' => {
                let mut search = pos;
                loop {
                    let Some(offset) = self.bytes[search + 1..].iter().position(|b| *b == code)
                    else {
                        return Err(Error);
                    };
                    search = search + 1 + offset;
                    if !self.escaped_before(search) {
                        break;
                    }
                }
                next = search;
                token = Token {
                    kind: Kind::String,
                    text: self.slice(pos, next + 1),
                };
            }
            b'@' => {
                let end = self.bytes[pos + 1..]
                    .iter()
                    .position(|b| {
                        matches!(
                            b,
                            b'\t'
                                | b'\n'
                                | b'\x0c'
                                | b'\r'
                                | b' '
                                | b'"'
                                | b'#'
                                | b'\''
                                | b'('
                                | b')'
                                | b'/'
                                | b';'
                                | b'['
                                | b'\\'
                                | b']'
                                | b'{'
                                | b'}'
                        )
                    })
                    .map_or(length, |offset| pos + 1 + offset);
                next = end - 1;
                token = Token {
                    kind: Kind::AtWord,
                    text: self.slice(pos, end),
                };
            }
            b'\\' => {
                let mut escape = true;
                while self.bytes.get(next + 1) == Some(&b'\\') {
                    next += 1;
                    escape = !escape;
                }
                let following = self.bytes.get(next + 1).copied();
                if escape
                    && let Some(following) = following
                    && !matches!(following, b'/' | b' ' | b'\n' | b'\t' | b'\r' | b'\x0c')
                {
                    // Un caractère entier (postcss lit une unité UTF-16).
                    let width = self.css[next + 1..]
                        .chars()
                        .next()
                        .map_or(1, char::len_utf8);
                    next += width;
                    if following.is_ascii_hexdigit() {
                        while self.bytes.get(next + 1).is_some_and(u8::is_ascii_hexdigit) {
                            next += 1;
                        }
                        if self.bytes.get(next + 1) == Some(&b' ') {
                            next += 1;
                        }
                    }
                }
                token = Token {
                    kind: Kind::Word,
                    text: self.slice(pos, next + 1),
                };
            }
            _ => {
                if code == b'/' && self.bytes.get(pos + 1) == Some(&b'*') {
                    let Some(offset) = self.css[pos + 2..].find("*/") else {
                        return Err(Error);
                    };
                    next = pos + 2 + offset + 1;
                    token = Token {
                        kind: Kind::Comment,
                        text: self.slice(pos, next + 1),
                    };
                } else {
                    let mut end = pos + 1;
                    while end < length {
                        let b = self.bytes[end];
                        if matches!(
                            b,
                            b'\t'
                                | b'\n'
                                | b'\x0c'
                                | b'\r'
                                | b' '
                                | b'!'
                                | b'"'
                                | b'#'
                                | b'\''
                                | b'('
                                | b')'
                                | b':'
                                | b';'
                                | b'@'
                                | b'['
                                | b'\\'
                                | b']'
                                | b'{'
                                | b'}'
                        ) || (b == b'/' && self.bytes.get(end + 1) == Some(&b'*'))
                        {
                            break;
                        }
                        end += 1;
                    }
                    next = end - 1;
                    let text = self.slice(pos, end);
                    token = Token {
                        kind: Kind::Word,
                        text,
                    };
                    self.buffer.push(text);
                }
            }
        }
        self.pos = next + 1;
        Ok(Some(token))
    }
}

/// `/.[\r\n"'(/\\]/` : un de ces caractères après un caractère qui n'est pas une fin de ligne.
fn bad_bracket(content: &str) -> bool {
    let mut previous: Option<char> = None;
    for c in content.chars() {
        if let Some(previous) = previous
            && !matches!(previous, '\n' | '\r' | '\u{2028}' | '\u{2029}')
            && matches!(c, '\r' | '\n' | '"' | '\'' | '(' | '/' | '\\')
        {
            return true;
        }
        previous = Some(c);
    }
    false
}

// ── Lecture (lib/parser.js) ───────────────────────────────────────────────

struct CssParser<'a> {
    tokens: Tokenizer<'a>,
    /// Les blocs ouverts ; vrai pour celui du premier nœud de la racine.
    open: Vec<bool>,
    root_count: usize,
    first_is_container: bool,
    out: Vec<StyleNode>,
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn concat(tokens: &[Token<'_>]) -> String {
    tokens.iter().map(|token| token.text).collect()
}

impl<'a> CssParser<'a> {
    fn next(&mut self) -> Result<Option<Token<'a>>, Error> {
        self.tokens.next_token()
    }

    /// Un nœud ajouté au bloc courant.
    fn add(&mut self, node: StyleNode, container: bool) {
        if self.open.is_empty() {
            self.root_count += 1;
            if self.root_count == 1 {
                self.first_is_container = container;
            }
        } else if self.open.len() == 1 && self.open[0] {
            self.out.push(node);
        }
    }

    fn open_block(&mut self) {
        let first = self.open.is_empty() && self.root_count == 1;
        self.open.push(first);
    }

    fn end(&mut self) -> Result<(), Error> {
        self.open.pop().map(|_| ()).ok_or(Error)
    }

    fn parse(&mut self) -> Result<(), Error> {
        while !self.tokens.end_of_file() {
            let Some(token) = self.next()? else { break };
            match token.kind {
                Kind::Space | Kind::Semicolon => {}
                Kind::CloseCurly => self.end()?,
                Kind::Comment => self.add(StyleNode::Other, false),
                Kind::AtWord => self.atrule(token)?,
                Kind::OpenCurly => {
                    self.add(StyleNode::Other, true);
                    self.open_block();
                }
                _ => self.other(token)?,
            }
        }
        if self.open.is_empty() {
            Ok(())
        } else {
            Err(Error)
        }
    }

    fn atrule(&mut self, token: Token<'a>) -> Result<(), Error> {
        if token.text.len() <= 1 {
            return Err(Error);
        }
        let mut brackets: Vec<Kind> = Vec::new();
        let mut open = false;
        let mut closed_parent = false;
        while !self.tokens.end_of_file() {
            let Some(token) = self.next()? else { break };
            match token.kind {
                Kind::OpenParen => brackets.push(Kind::CloseParen),
                Kind::OpenSquare => brackets.push(Kind::CloseSquare),
                Kind::OpenCurly if !brackets.is_empty() => brackets.push(Kind::CloseCurly),
                kind if brackets.last() == Some(&kind) => {
                    brackets.pop();
                }
                _ => {}
            }
            if brackets.is_empty() {
                match token.kind {
                    Kind::Semicolon => break,
                    Kind::OpenCurly => {
                        open = true;
                        break;
                    }
                    Kind::CloseCurly => {
                        closed_parent = true;
                        break;
                    }
                    _ => {}
                }
            }
        }
        self.add(StyleNode::Other, open);
        if closed_parent {
            self.end()?;
        }
        if open {
            self.open_block();
        }
        Ok(())
    }

    fn other(&mut self, start: Token<'a>) -> Result<(), Error> {
        let custom = start.text.starts_with("--");
        let mut tokens: Vec<Token<'a>> = Vec::new();
        let mut brackets: Vec<Kind> = Vec::new();
        let mut colon = false;
        let mut end = false;
        let mut token = Some(start);
        while let Some(current) = token {
            tokens.push(current);
            match current.kind {
                Kind::OpenParen => brackets.push(Kind::CloseParen),
                Kind::OpenSquare => brackets.push(Kind::CloseSquare),
                Kind::OpenCurly if custom && colon => brackets.push(Kind::CloseCurly),
                kind if brackets.is_empty() => match kind {
                    Kind::Semicolon => {
                        if colon {
                            return self.decl(tokens, custom);
                        }
                        break;
                    }
                    Kind::OpenCurly => {
                        tokens.pop();
                        self.add(StyleNode::Other, true);
                        self.open_block();
                        return Ok(());
                    }
                    Kind::CloseCurly => {
                        if let Some(last) = tokens.pop() {
                            self.tokens.back(last);
                        }
                        end = true;
                        break;
                    }
                    Kind::Colon => colon = true,
                    _ => {}
                },
                kind if brackets.last() == Some(&kind) => {
                    brackets.pop();
                }
                _ => {}
            }
            token = self.next()?;
        }
        if self.tokens.end_of_file() {
            end = true;
        }
        if !brackets.is_empty() {
            return Err(Error);
        }
        if end && colon {
            if !custom {
                while let Some(last) = tokens.last().copied() {
                    if last.kind != Kind::Space && last.kind != Kind::Comment {
                        break;
                    }
                    tokens.pop();
                    self.tokens.back(last);
                }
            }
            self.decl(tokens, custom)
        } else {
            Err(Error)
        }
    }

    #[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
    fn decl(&mut self, mut tokens: Vec<Token<'a>>, custom: bool) -> Result<(), Error> {
        if tokens
            .last()
            .is_some_and(|token| token.kind == Kind::Semicolon)
        {
            tokens.pop();
        }
        let mut start = 0;
        loop {
            let Some(token) = tokens.get(start) else {
                return Err(Error);
            };
            if token.kind == Kind::Word {
                break;
            }
            if start + 1 == tokens.len() {
                return Err(Error);
            }
            start += 1;
        }
        let prop_start = start;
        while let Some(token) = tokens.get(start) {
            if matches!(token.kind, Kind::Colon | Kind::Space | Kind::Comment) {
                break;
            }
            start += 1;
        }
        let mut prop = concat(&tokens[prop_start..start]);
        while let Some(token) = tokens.get(start) {
            start += 1;
            if token.kind == Kind::Colon {
                break;
            }
            if token.kind == Kind::Word && token.text.chars().any(is_word_char) {
                return Err(Error);
            }
        }
        if prop.starts_with(['_', '*']) {
            prop.remove(0);
        }
        let first_spaces_start = start;
        while let Some(token) = tokens.get(start) {
            if token.kind != Kind::Space && token.kind != Kind::Comment {
                break;
            }
            start += 1;
        }
        let mut first_spaces: Vec<Token<'a>> = tokens[first_spaces_start..start].to_vec();
        let mut tokens: Vec<Token<'a>> = tokens[start..].to_vec();

        let mut important = false;
        let mut index = tokens.len();
        while index > 0 {
            index -= 1;
            let token = tokens[index];
            let lower = token.text.to_lowercase();
            if lower == "!important" {
                important = true;
                tokens.truncate(index);
                while tokens.last().is_some_and(|token| token.kind == Kind::Space) {
                    tokens.pop();
                }
                break;
            } else if lower == "important" {
                let mut cache = tokens.clone();
                let mut text = String::new();
                let mut j = index;
                while j > 0 {
                    let kind = cache.get(j).map_or(Kind::Space, |token| token.kind);
                    if text.trim().starts_with('!') && kind != Kind::Space {
                        break;
                    }
                    if let Some(popped) = cache.pop() {
                        text.insert_str(0, popped.text);
                    }
                    j -= 1;
                }
                if text.trim().starts_with('!') {
                    important = true;
                    tokens = cache;
                }
            }
            if token.kind != Kind::Space && token.kind != Kind::Comment {
                break;
            }
        }

        let has_word = tokens
            .iter()
            .any(|token| token.kind != Kind::Space && token.kind != Kind::Comment);
        if has_word {
            first_spaces.clear();
        }
        first_spaces.extend_from_slice(&tokens);
        let value = raw_value(&first_spaces, custom);
        if value.contains(':') && !custom && has_top_level_colon(&tokens)? {
            return Err(Error);
        }
        self.add(
            StyleNode::Declaration {
                prop,
                value,
                important,
            },
            false,
        );
        Ok(())
    }
}

/// `colon()` de postcss : un `:` hors parenthèses (une déclaration à laquelle il manque un `;`).
/// Un `:` en tête est une erreur aussi (« Double colon »).
fn has_top_level_colon(tokens: &[Token<'_>]) -> Result<bool, Error> {
    let mut brackets: i64 = 0;
    let mut previous: Option<Token<'_>> = None;
    for token in tokens {
        match token.kind {
            Kind::OpenParen => brackets += 1,
            Kind::CloseParen => brackets -= 1,
            _ => {}
        }
        if brackets == 0 && token.kind == Kind::Colon {
            match previous {
                None => return Err(Error),
                Some(previous) if previous.kind == Kind::Word && previous.text == "progid" => {
                    continue;
                }
                Some(_) => return Ok(true),
            }
        }
        previous = Some(*token);
    }
    Ok(false)
}

/// `raw()` de postcss : la valeur sans l'espace finale ni les commentaires voisins d'une espace.
fn raw_value(tokens: &[Token<'_>], custom: bool) -> String {
    let safe = |kind: Option<Kind>| matches!(kind, None | Some(Kind::Space));
    let mut value = String::new();
    let mut last: Option<char> = None;
    for (index, token) in tokens.iter().enumerate() {
        if token.kind == Kind::Space && index + 1 == tokens.len() && !custom {
            continue;
        }
        if token.kind == Kind::Comment {
            let previous = index
                .checked_sub(1)
                .and_then(|at| tokens.get(at))
                .map(|token| token.kind);
            let next = tokens.get(index + 1).map(|token| token.kind);
            if !safe(previous) && !safe(next) && last != Some(',') {
                value.push_str(token.text);
                last = token.text.chars().last();
            }
            continue;
        }
        value.push_str(token.text);
        if let Some(c) = token.text.chars().last() {
            last = Some(c);
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(value: &str) -> Option<String> {
        let nodes = parse_style("p", value)?;
        let mut out = String::new();
        for (index, node) in nodes.iter().enumerate() {
            if index > 0 {
                out.push(';');
            }
            node.write(&mut out);
        }
        Some(out)
    }

    #[test]
    fn comme_postcss() {
        // Les cas relevés sur Tafac (sanitize-html 2.18, postcss 8.5).
        let cases: &[(&str, Option<&str>)] = &[
            ("color:red;/*x*/", Some("color:red;undefined:undefined")),
            (
                "color: red /* c */ ; margin : 0 ",
                Some("color:red ;margin:0"),
            ),
            ("color:red !important", Some("color:red !important")),
            (
                "color:red!important;x:y ! important",
                Some("color:red !important;x:y !important"),
            ),
            ("color:red} x {color:blue", Some("color:red")),
            ("color", None),
            ("color:", Some("color:")),
            ("color:red;;margin:0;", Some("color:red;margin:0")),
            ("font-family:\"A;B\", x", Some("font-family:\"A;B\", x")),
            ("a:b(c;d)", Some("a:b(c;d)")),
            ("a:b(c", None),
            ("a:\"b", None),
            ("/*", None),
            ("{a:b}", Some("undefined:undefined")),
            ("a:b{c}", None),
            ("@media x{a:b}", Some("undefined:undefined")),
            ("a : b c  d", Some("a:b c  d")),
            ("  ", Some("")),
            ("a:b;c", None),
            ("-x-y:1;--v: { a }", Some("-x-y:1;--v:{ a }")),
            ("a:b\nc", Some("a:b\nc")),
            ("a:\\;b:c", None),
            ("a:\\\\;b:c", Some("a:\\\\;b:c")),
            ("a:url(x;y)", Some("a:url(x;y)")),
            ("a:'b;c'", Some("a:'b;c'")),
            ("a b:c", None),
            ("a:!important", Some("a: !important")),
            ("a:b !IMPORTANT", Some("a:b !important")),
            ("a:b !ie", Some("a:b !ie")),
            ("a:b}", None),
            ("}", None),
            ("a:b }{", Some("a:b")),
            ("a:b:c", None),
            (
                "background:url(\"x\") no-repeat",
                Some("background:url(\"x\") no-repeat"),
            ),
        ];
        for (value, expected) in cases {
            assert_eq!(written(value).as_deref(), *expected, "{value:?}");
        }
    }

    #[test]
    fn lineaire_sur_les_parentheses() {
        let value = format!("a:{}", "(".repeat(200_000));
        assert_eq!(written(&value), None);
        let value = format!("a:{}", "url(".repeat(50_000));
        assert_eq!(written(&value), None);
    }
}
