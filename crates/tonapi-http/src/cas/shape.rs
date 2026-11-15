//! Diagnostic d'une page CAS non reconnue : sa structure, sans aucune valeur.

use crate::forms::{self, Form};

/// Structure d'une page pour le diagnostic : scripts, appels `f({…})`, clés JSON et champs de
/// formulaire. Aucune valeur n'est reprise.
pub(super) fn page_shape(body: &str, forms: &[Form]) -> String {
    let html = scraper::Html::parse_document(body);
    let mut sources = Vec::new();
    let mut calls = std::collections::BTreeSet::new();
    let mut keys = std::collections::BTreeSet::new();
    if let Ok(selector) = scraper::Selector::parse("script") {
        for script in html.select(&selector) {
            if let Some(src) = script.value().attr("src") {
                sources.push(src.split('?').next().unwrap_or_default().to_owned());
                continue;
            }
            let text: String = script.text().collect();
            collect_calls(&text, &mut calls);
            collect_keys(&text, &mut keys);
        }
    }
    let forms: Vec<String> = forms
        .iter()
        .map(|f| {
            let action = f
                .action
                .as_deref()
                .unwrap_or("")
                .split('?')
                .next()
                .unwrap_or("");
            format!("{action:?}:{:?}", f.names)
        })
        .collect();
    // Pour la page esup-otp : chaque `start(` et le type des paramètres attendus.
    let starts: Vec<String> = body
        .match_indices("start(")
        .map(|(index, call)| {
            let rest = body[index + call.len()..].trim_start();
            let value = serde_json::Deserializer::from_str(rest)
                .into_iter::<serde_json::Value>()
                .next()
                .and_then(Result::ok);
            match value {
                Some(serde_json::Value::Object(map)) => {
                    let types: Vec<String> = ["apiUrl", "uid", "userHash"]
                        .iter()
                        .map(|k| format!("{k}:{}", json_type(map.get(*k))))
                        .collect();
                    format!("objet({})", types.join(","))
                }
                Some(_) => "autre".to_owned(),
                None => rest
                    .chars()
                    .next()
                    .map_or("fin".to_owned(), |c| format!("{c:?}")),
            }
        })
        .collect();
    let ticket = forms_ticket(body);
    format!(
        "scripts={sources:?} appels={calls:?} clés={keys:?} formulaires={forms:?} start={starts:?} ticket={ticket}"
    )
}

fn json_type(value: Option<&serde_json::Value>) -> &'static str {
    match value {
        None => "absent",
        Some(serde_json::Value::String(s)) if s.is_empty() => "chaîne vide",
        Some(serde_json::Value::String(_)) => "chaîne",
        Some(serde_json::Value::Number(_)) => "nombre",
        Some(serde_json::Value::Null) => "null",
        Some(_) => "autre",
    }
}

/// État du champ `ticket` : absent, vide ou rempli (jamais sa valeur).
fn forms_ticket(body: &str) -> &'static str {
    match forms::forms(body)
        .iter()
        .find_map(|f| f.value("ticket").map(str::is_empty))
    {
        None => "absent",
        Some(true) => "vide",
        Some(false) => "rempli",
    }
}

/// Noms `f` des appels `f({` ou `f(\"` dans un script.
fn collect_calls(text: &str, out: &mut std::collections::BTreeSet<String>) {
    let bytes = text.as_bytes();
    for (i, _) in text.match_indices('(') {
        let next = text[i + 1..].trim_start().chars().next();
        if !matches!(next, Some('{' | '"' | '\'')) {
            continue;
        }
        let mut start = i;
        while start > 0
            && (bytes[start - 1].is_ascii_alphanumeric()
                || matches!(bytes[start - 1], b'_' | b'$' | b'.'))
        {
            start -= 1;
        }
        if start < i && out.len() < 30 {
            out.insert(text[start..i].to_owned());
        }
    }
}

/// Clés `"clé":` d'objets JSON ou JavaScript dans un script.
fn collect_keys(text: &str, out: &mut std::collections::BTreeSet<String>) {
    for (i, _) in text.match_indices('"') {
        let rest = &text[i + 1..];
        let Some(end) = rest.find('"') else { continue };
        let key = &rest[..end];
        let after = rest[end + 1..].trim_start();
        if after.starts_with(':')
            && !key.is_empty()
            && key.len() <= 40
            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && out.len() < 60
        {
            out.insert(key.to_owned());
        }
    }
}
