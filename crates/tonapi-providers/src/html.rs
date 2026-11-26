//! Lecture de pages HTML sans construire d'arbre : balises ouvrantes d'un nom donné, leurs
//! attributs, et le texte d'un fragment. Pour les portails qui n'ont pas d'API et dont la page est
//! la seule source (Drupal, iSorbonne).
//!
//! Une balise va de `<nom` au premier `>` : une valeur d'attribut qui contient `>` la coupe, comme
//! dans les expressions de Tafac dont ces analyses sont reprises.

use crate::common::decode_entities;

/// Balise ouvrante d'une page.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tag<'a> {
    /// Position du `<` dans le texte analysé.
    pub start: usize,
    /// Position qui suit le `>`.
    pub end: usize,
    /// La balise telle qu'elle est écrite, de `<` à `>`.
    source: &'a str,
}

impl Tag<'_> {
    /// Valeur d'un attribut (nom en minuscules), entités décodées. Un attribut sans valeur vaut
    /// la chaîne vide.
    pub(crate) fn attr(&self, name: &str) -> Option<String> {
        attributes(self.source)
            .into_iter()
            .find(|(found, _)| found.eq_ignore_ascii_case(name))
            .map(|(_, value)| decode_entities(value))
    }

    /// L'attribut `class` contient ce mot.
    pub(crate) fn has_class(&self, class: &str) -> bool {
        self.attr("class")
            .is_some_and(|classes| classes.split_whitespace().any(|found| found == class))
    }
}

/// Attributs d'une balise ouvrante : nom et valeur brute, dans l'ordre.
fn attributes(source: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    // Après `<nom`.
    let mut rest = source
        .trim_start_matches('<')
        .trim_start_matches(|c: char| !c.is_whitespace() && c != '>' && c != '/');
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        if rest.is_empty() || rest.starts_with('>') {
            return out;
        }
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '=' || c == '>' || c == '/')
            .unwrap_or(rest.len());
        let (name, after) = rest.split_at(name_end);
        let after = after.trim_start();
        let Some(value) = after.strip_prefix('=') else {
            if !name.is_empty() {
                out.push((name, ""));
            }
            // Un `=` sans nom devant lui : on avance d'un caractère pour ne pas boucler.
            rest = if name.is_empty() {
                after.get(1..).unwrap_or_default()
            } else {
                after
            };
            continue;
        };
        let value = value.trim_start();
        let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'');
        let (value, after) = if let Some(quote) = quote {
            let quoted = value.get(1..).unwrap_or_default();
            match quoted.find(quote) {
                Some(end) => (
                    quoted.get(..end).unwrap_or_default(),
                    quoted.get(end + 1..).unwrap_or_default(),
                ),
                None => (quoted.trim_end_matches('>'), ""),
            }
        } else {
            let end = value
                .find(|c: char| c.is_whitespace() || c == '>')
                .unwrap_or(value.len());
            value.split_at(end)
        };
        if !name.is_empty() {
            out.push((name, value));
        }
        rest = after;
    }
}

/// Balises ouvrantes `<nom …>` de `html`, dans l'ordre du document, casse ignorée. `name` est en
/// minuscules.
pub(crate) fn tags<'a>(html: &'a str, name: &str) -> Vec<Tag<'a>> {
    // Même longueur en octets que `html` : les positions valent pour les deux.
    let lower = html.to_ascii_lowercase();
    let needle = format!("<{name}");
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = lower.get(from..).and_then(|rest| rest.find(&needle)) {
        let start = from + at;
        let after_name = start + needle.len();
        // `<a` n'est pas `<abbr` : le nom finit sur un blanc, `>` ou `/`.
        let named = lower
            .as_bytes()
            .get(after_name)
            .is_none_or(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/');
        if !named {
            from = after_name;
            continue;
        }
        let Some(close) = lower.get(after_name..).and_then(|rest| rest.find('>')) else {
            break;
        };
        let end = after_name + close + 1;
        if let Some(source) = html.get(start..end) {
            out.push(Tag { start, end, source });
        }
        from = end;
    }
    out
}

/// Ce qui suit `tag` dans `html` jusqu'à sa première balise fermante `</nom`, casse ignorée.
/// `None` si elle manque.
pub(crate) fn inner<'a>(html: &'a str, tag: &Tag<'_>, name: &str) -> Option<&'a str> {
    let rest = html.get(tag.end..)?;
    let close = rest.to_ascii_lowercase().find(&format!("</{name}"))?;
    rest.get(..close)
}

/// Texte d'un fragment HTML : chaque balise vaut un espace, les entités sont décodées, les blancs
/// réduits à un espace.
pub(crate) fn text(fragment: &str) -> String {
    let mut stripped = String::with_capacity(fragment.len());
    let mut rest = fragment;
    while let Some(open) = rest.find('<') {
        stripped.push_str(rest.get(..open).unwrap_or_default());
        let tail = rest.get(open..).unwrap_or_default();
        if let Some(close) = tail.find('>') {
            stripped.push(' ');
            rest = tail.get(close + 1..).unwrap_or_default();
        } else {
            // Un `<` sans `>` n'ouvre pas de balise : c'est du texte.
            stripped.push_str(tail);
            rest = "";
        }
    }
    stripped.push_str(rest);
    decode_entities(&stripped)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balises_par_nom_et_attributs() {
        let html = r#"<p>Un <a href="/x?a=1&amp;b=2" class="links  gros" hidden>lien</a>, une <abbr title="x">abr.</abbr>
            et <A HREF='/y' data-href="/piege">autre</A> <a>nu</a></p>"#;
        let links = tags(html, "a");
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].attr("href").as_deref(), Some("/x?a=1&b=2"));
        assert_eq!(links[0].attr("hidden").as_deref(), Some(""));
        assert!(links[0].has_class("links") && links[0].has_class("gros"));
        assert!(!links[0].has_class("link"));
        // Guillemets simples, majuscules, et `data-href` qui n'est pas `href`.
        assert_eq!(links[1].attr("href").as_deref(), Some("/y"));
        assert_eq!(links[1].attr("data-href").as_deref(), Some("/piege"));
        assert_eq!(links[2].attr("href"), None);
        assert_eq!(inner(html, &links[0], "a"), Some("lien"));
        assert_eq!(inner(html, &links[1], "a"), Some("autre"));
    }

    #[test]
    fn balise_tronquee_ou_sans_fermeture() {
        assert!(tags("<div class=\"x", "div").is_empty());
        let html = "<label id=x>sans fin";
        let labels = tags(html, "label");
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].attr("id").as_deref(), Some("x"));
        assert_eq!(inner(html, &labels[0], "label"), None);
        // Des `=` isolés ne font pas boucler la lecture des attributs.
        assert_eq!(
            tags("<div = = a=1 =>", "div")[0].attr("a").as_deref(),
            Some("1")
        );
    }

    #[test]
    fn texte_sans_balises() {
        assert_eq!(
            text("  <span>Mes disques</span>\n<b>&quot;M&quot;</b> &amp; K&nbsp;: "),
            "Mes disques \"M\" & K :"
        );
        assert_eq!(text("a < b"), "a < b");
        assert_eq!(text("&lt;b&gt;"), "<b>");
        assert_eq!(text(""), "");
    }
}
