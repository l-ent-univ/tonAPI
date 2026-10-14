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

/// Tous les formulaires de la page.
#[must_use]
pub fn forms(html: &str) -> Vec<Form> {
    let document = Html::parse_document(html);
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
    let document = Html::parse_document(html);
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
    let document = Html::parse_document(html);
    document
        .select(&selector("title"))
        .next()
        .map(|t| t.text().collect::<String>().trim().to_owned())
}

#[cfg(test)]
mod tests {
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
