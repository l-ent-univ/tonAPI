//! Lecture des formulaires et des messages d'erreur dans les pages HTML.
//!
//! Tout est copié dans des types possédés : le document HTML n'est pas `Send` et ne doit pas
//! survivre à un `await`.

use scraper::{ElementRef, Html, Selector};

/// Formulaire HTML.
#[derive(Debug, Clone, Default)]
pub struct Form {
    pub id: Option<String>,
    pub action: Option<String>,
    /// Champs qu'un navigateur enverrait (hors boutons et cases non cochées), dans l'ordre.
    pub fields: Vec<(String, String)>,
    /// Boutons nommés : `(name, value)`.
    pub buttons: Vec<(String, String)>,
    /// Noms de tous les contrôles, y compris boutons et cases à cocher.
    pub names: Vec<String>,
    pub has_password: bool,
}

impl Form {
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    #[must_use]
    pub fn value(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

fn selector(css: &str) -> Selector {
    Selector::parse(css).unwrap_or_else(|_| unreachable!("sélecteur constant : {css}"))
}

/// Imbrication au-delà de laquelle une page n'est pas analysée. html5ever parcourt la pile des
/// éléments ouverts à chaque balise : sur des milliers de `<div>` imbriqués, le coût devient
/// quadratique et bloque un fil du runtime. Une vraie page de connexion reste sous 100.
const MAX_DEPTH: usize = 512;

/// Éléments sans balise fermante.
const VOID: [&str; 13] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// Document analysé, ou `None` si la page est trop imbriquée pour l'être sans risque.
fn parse(html: &str) -> Option<Html> {
    if too_deep(html) {
        tracing::debug!("page HTML trop imbriquée : non analysée");
        return None;
    }
    Some(Html::parse_document(html))
}

/// Estimation linéaire de l'imbrication : balises ouvrantes moins fermantes, hors éléments vides,
/// commentaires, scripts et styles. Elle surestime une page mal fermée, jamais l'inverse sur une
/// page bien formée.
fn too_deep(html: &str) -> bool {
    let bytes = html.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while let Some(offset) = bytes
        .get(i..)
        .and_then(|rest| rest.iter().position(|b| *b == b'<'))
    {
        i += offset + 1;
        let rest = bytes.get(i..).unwrap_or_default();
        if rest.starts_with(b"!--") {
            i += find(rest, b"-->").map_or(rest.len(), |e| e + 3);
            continue;
        }
        let closing = rest.first() == Some(&b'/');
        let name_start = usize::from(closing);
        let name_len = rest
            .get(name_start..)
            .unwrap_or_default()
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric())
            .count();
        if name_len == 0 {
            continue;
        }
        let name = rest
            .get(name_start..name_start + name_len)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let tag_end = find(rest, b">").map_or(rest.len(), |e| e + 1);
        let self_closing = rest.get(..tag_end).is_some_and(|t| t.ends_with(b"/>"));
        if closing {
            depth = depth.saturating_sub(1);
        } else if !self_closing && !VOID.iter().any(|v| v.as_bytes() == name.as_slice()) {
            depth += 1;
            if depth > MAX_DEPTH {
                return true;
            }
            // Contenu brut : un `<` de script n'ouvre rien.
            if name == b"script" || name == b"style" {
                let close = if name == b"script" {
                    b"</script".as_slice()
                } else {
                    b"</style"
                };
                let after = rest.get(tag_end..).unwrap_or_default();
                i += tag_end + find_ignore_case(after, close).unwrap_or(after.len());
                continue;
            }
        }
        i += tag_end;
    }
    false
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Comme [`find`], sans tenir compte de la casse ASCII et sans copier `haystack` : une copie par
/// `<script>` rendait l'analyse quadratique sur une page de plusieurs Mo. `needle` n'est pas vide.
fn find_ignore_case(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

/// Tous les formulaires de la page.
#[must_use]
pub fn forms(html: &str) -> Vec<Form> {
    let Some(document) = parse(html) else {
        return Vec::new();
    };
    let form_selector = selector("form");
    let control_selector = selector("input, button, select, textarea");
    document
        .select(&form_selector)
        .map(|form| {
            let mut result = Form {
                id: form.value().attr("id").map(str::to_owned),
                action: form.value().attr("action").map(str::to_owned),
                ..Form::default()
            };
            for control in form.select(&control_selector) {
                read_control(control, &mut result);
            }
            result
        })
        .collect()
}

fn read_control(control: ElementRef<'_>, form: &mut Form) {
    let element = control.value();
    let Some(name) = element.attr("name").filter(|n| !n.is_empty()) else {
        return;
    };
    form.names.push(name.to_owned());
    let value = element.attr("value").unwrap_or_default().to_owned();
    match element.name() {
        "button" => form.buttons.push((name.to_owned(), value)),
        "input" => match element
            .attr("type")
            .unwrap_or("text")
            .to_ascii_lowercase()
            .as_str()
        {
            "submit" | "button" | "image" | "reset" => form.buttons.push((name.to_owned(), value)),
            "checkbox" | "radio" if element.attr("checked").is_none() => {}
            "password" => {
                form.has_password = true;
                form.fields.push((name.to_owned(), value));
            }
            _ => form.fields.push((name.to_owned(), value)),
        },
        _ => form.fields.push((name.to_owned(), value)),
    }
}

/// Classes qui désignent à coup sûr une erreur.
const STRONG: &[&str] = &[
    "alert-danger",
    "banner-danger",
    "error",
    "errors",
    "text-danger",
    "error-message",
];
/// Classes génériques, retenues seulement s'il n'y a rien de plus sûr. `banner-warning` est exclu :
/// c'est le bandeau de confidentialité permanent du CAS de Rennes.
const WEAK: &[&str] = &["alert", "message", "messages"];

/// Message d'erreur affiché par la page (CAS, OTP), s'il y en a un.
#[must_use]
pub fn error_message(html: &str) -> Option<String> {
    let document = parse(html)?;
    let candidates = selector("div, p, span");
    let mut weak = None;
    for element in document.select(&candidates) {
        // Blocs de gabarit non rendus (Thymeleaf) : jamais un vrai message.
        if element
            .value()
            .attrs()
            .any(|(name, _)| name.starts_with("th:"))
        {
            continue;
        }
        let classes: Vec<&str> = element.value().classes().collect();
        let is_errors_id = element.value().id() == Some("errors");
        let strong = is_errors_id || classes.iter().any(|c| STRONG.contains(c));
        if !strong && !classes.iter().any(|c| WEAK.contains(c)) {
            continue;
        }
        let text = element.text().collect::<Vec<_>>().join(" ");
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        let text: String = text.chars().take(300).collect();
        if strong {
            return Some(text);
        }
        weak.get_or_insert(text);
    }
    weak
}

/// Titre de la page.
#[must_use]
pub fn title(html: &str) -> Option<String> {
    let document = parse(html)?;
    document
        .select(&selector("title"))
        .next()
        .map(|t| t.text().collect::<String>().trim().to_owned())
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    const CAS: &str = r#"<html><head><title>CAS</title></head><body>
      <div class="banner banner-warning">Votre mot de passe est confidentiel</div>
      <div th:if="${x}" class="alert alert-danger">gabarit</div>
      <div class="banner banner-danger"><span>Mauvais identifiant / mot de passe.</span></div>
      <form method="post" id="fm1" action="login">
        <input id="username" type="text" name="username"/>
        <input type="password" id="password" name="password"/>
        <input type="hidden" name="execution" value="abc_def"/>
        <input type="hidden" name="_eventId" value="submit"/>
        <input type="hidden" name="geolocation"/>
        <input type="checkbox" name="publicWorkstation" value="true"/>
        <button name="submitBtn" type="submit">Se connecter</button>
      </form></body></html>"#;

    /// Des milliers de balises imbriquées rendaient l'analyse quadratique.
    #[test]
    fn page_trop_imbriquee_non_analysee() {
        let deep = format!(
            "{}<form><input name=\"x\"></form>",
            "<div>".repeat(MAX_DEPTH + 1)
        );
        let started = std::time::Instant::now();
        assert!(forms(&deep).is_empty());
        assert_eq!(error_message(&deep), None);
        assert!(started.elapsed() < std::time::Duration::from_millis(200));

        // Une page ordinaire, éléments vides, script et commentaire compris, reste lue.
        let script = format!(
            "<script>for(i=0;i<n;i++){{}}{}</script>",
            "if(a<b)".repeat(600)
        );
        let normal = format!(
            "<!-- <div><div> -->{script}<br><img src=x><p>{}</p>{CAS}",
            "<br><input name=\"y\">".repeat(600)
        );
        assert!(!too_deep(&normal));
        assert_eq!(forms(&normal).len(), 1);
    }

    /// Banc (ignoré) : `cargo test -p tonapi-http --release -- --ignored --nocapture banc_forms`.
    fn page_synthetique(scripts: usize, total: usize) -> String {
        let corps = "var a = (b < c) ? 1 : 2; /* x */ ".repeat(total / scripts / 34 + 1);
        let mut page = String::with_capacity(total + 4096);
        page.push_str("<html><head><title>Synthese</title>");
        for i in 0..scripts {
            let _ = write!(
                page,
                "<script>/*{i}*/{corps}</SCRIPT><style>p{{color:red}}</style>"
            );
        }
        page.push_str("</head><body>");
        page.push_str(CAS);
        page.push_str("</body></html>");
        page
    }

    #[test]
    #[ignore = "banc : mesure, pas de vérification"]
    fn banc_forms() {
        let page = page_synthetique(3000, 4 * 1024 * 1024);
        let mut too_deep_ms = Vec::new();
        let mut forms_ms = Vec::new();
        for _ in 0..7 {
            let t = std::time::Instant::now();
            assert!(!too_deep(&page));
            too_deep_ms.push(t.elapsed().as_secs_f64() * 1000.0);
            let t = std::time::Instant::now();
            assert_eq!(forms(&page).len(), 1);
            assert_eq!(title(&page).as_deref(), Some("Synthese"));
            assert!(error_message(&page).is_some());
            forms_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let stat = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            (v[v.len() / 2], v[v.len() - 1])
        };
        println!("taille={} octets", page.len());
        println!("too_deep   médiane/max (ms) = {:?}", stat(&mut too_deep_ms));
        println!("forms+title+error_message   = {:?}", stat(&mut forms_ms));
    }

    /// La recherche sans copie donne exactement la position de l'ancienne version (copie en
    /// minuscules puis recherche), y compris sur des octets non UTF-8 et quand rien n'est trouvé.
    #[test]
    fn recherche_insensible_a_la_casse_identique_a_la_copie_en_minuscules() {
        let cas: [&[u8]; 8] = [
            b"a</script>b",
            b"a</SCRIPT>b",
            b"a</ScRiPt >b</script>",
            b"<div><div></Script",
            b"pas de balise fermante",
            b"</scrip",
            b"\xff\xfe</STYLE\xc3",
            b"",
        ];
        for entree in cas {
            for needle in [b"</script".as_slice(), b"</style"] {
                assert_eq!(
                    find_ignore_case(entree, needle),
                    find(&entree.to_ascii_lowercase(), needle),
                    "{entree:?} / {needle:?}"
                );
            }
        }
        let melange =
            "<SCRIPT>if(a<b){}</Script><div><DIV></div></div><form><input name=\"x\"></form>";
        assert!(!too_deep(melange));
        assert_eq!(forms(melange).len(), 1);
    }

    #[test]
    fn formulaire_cas() {
        let forms = forms(CAS);
        assert_eq!(forms.len(), 1);
        let form = &forms[0];
        assert_eq!(form.id.as_deref(), Some("fm1"));
        assert_eq!(form.action.as_deref(), Some("login"));
        assert!(form.has_password);
        assert_eq!(form.value("execution"), Some("abc_def"));
        assert!(form.has("submitBtn") && form.has("publicWorkstation"));
        assert!(form.value("publicWorkstation").is_none());
        assert_eq!(title(CAS).as_deref(), Some("CAS"));
    }

    #[test]
    fn message_d_erreur_sans_bandeau_ni_gabarit() {
        assert_eq!(
            error_message(CAS).as_deref(),
            Some("Mauvais identifiant / mot de passe.")
        );
        assert_eq!(
            error_message(r#"<div class="banner banner-warning">info</div>"#),
            None
        );
        assert_eq!(
            error_message(r#"<div id="errors"> Le code est invalide. </div>"#).as_deref(),
            Some("Le code est invalide.")
        );
    }
}
