//! `tonapi report check` : contrôle d'un rapport d'établissement (`reports/*.md`) avant de le
//! committer.
//!
//! Claude écrit le rapport en ne notant que des structures ; ceci est le second filet. Une seule
//! trouvaille suffit à refuser le fichier.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

/// Donnée suspecte trouvée dans le rapport.
#[derive(Debug, PartialEq, Eq)]
pub struct Finding {
    pub path: String,
    pub reason: &'static str,
}

/// Raison pour laquelle `text` ne devrait pas figurer dans un rapport anonymisé.
fn suspicious(text: &str) -> Option<&'static str> {
    // Les remplaçants de l'outil (« <email> », « <jeton:40> ») sont attendus : on les retire d'abord.
    let mut cleaned = String::with_capacity(text.len());
    let mut depth = 0_u32;
    for c in text.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => cleaned.push(c),
            _ => {}
        }
    }
    let t = cleaned.as_str();

    if let Some(at) = t.find('@') {
        let (before, after) = t.split_at(at);
        let local_ok = before
            .chars()
            .last()
            .is_some_and(|c| c.is_alphanumeric() || "._-+".contains(c));
        if local_ok && after[1..].contains('.') {
            return Some("adresse e-mail");
        }
    }
    if let Some(i) = t.find("eyJ")
        && t[i..]
            .split(|c: char| c.is_whitespace() || c == '"')
            .next()
            .is_some_and(|w| w.matches('.').count() >= 2)
    {
        return Some("jeton JWT");
    }
    for prefix in ["ST-", "TGT-", "PGT-"] {
        if let Some(i) = t.find(prefix)
            && t[i + prefix.len()..].starts_with(|c: char| c.is_ascii_digit())
        {
            return Some("ticket CAS");
        }
    }
    let mut run = String::new();
    for c in t.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_alphanumeric() || "+/_=-".contains(c) {
            run.push(c);
            continue;
        }
        if run.len() >= 32
            && run.chars().any(|c| c.is_ascii_digit() && c != '9')
            && run
                .chars()
                .any(|c| c.is_ascii_alphabetic() && c != 'a' && c != 'A')
        {
            return Some("jeton ou identifiant long");
        }
        run.clear();
    }
    // Suite de chiffres consécutifs : les motifs de l'outil n'utilisent que des 9.
    let long_number = t
        .split(|c: char| !c.is_ascii_digit())
        .any(|digits| digits.len() >= 6 && digits.chars().any(|c| c != '9'));
    if long_number {
        return Some("nombre long (numéro étudiant, téléphone ?)");
    }
    None
}

fn walk(value: &Value, path: &str, findings: &mut Vec<Finding>) {
    match value {
        Value::String(s) => {
            if let Some(reason) = suspicious(s) {
                findings.push(Finding {
                    path: path.to_owned(),
                    reason,
                });
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                walk(item, &format!("{path}[{i}]"), findings);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                let child = format!("{path}.{key}");
                if let Some(reason) = suspicious(key) {
                    findings.push(Finding {
                        path: child.clone(),
                        reason,
                    });
                }
                walk(item, &child, findings);
            }
        }
        _ => {}
    }
}

/// Contrôle un document JSON : chaque clé et chaque texte.
pub fn check_json(value: &Value) -> Vec<Finding> {
    let mut findings = Vec::new();
    walk(value, "$", &mut findings);
    findings
}

/// Contrôle un texte (Markdown, TOML) ligne par ligne.
pub fn check_text(text: &str) -> Vec<Finding> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            suspicious(line).map(|reason| Finding {
                path: format!("ligne {}", i + 1),
                reason,
            })
        })
        .collect()
}

/// `tonapi report check <fichier>…` : vrai si aucun fichier ne contient de donnée suspecte.
pub fn check(paths: &[std::path::PathBuf]) -> Result<bool> {
    let mut clean = true;
    for path in paths {
        let findings = check_file(path)?;
        if findings.is_empty() {
            println!("✓ {}", path.display());
            continue;
        }
        clean = false;
        for f in &findings {
            println!("✗ {} ({}) : {}", path.display(), f.path, f.reason);
        }
    }
    if !clean {
        println!(
            "\nNe pas committer : remplacer chaque valeur par sa forme (`<id>`, `99.99`) ou la retirer."
        );
    }
    Ok(clean)
}

fn check_file(path: &Path) -> Result<Vec<Finding>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("lecture de {}", path.display()))?;
    if path.extension().is_some_and(|e| e == "json") {
        let value: Value = serde_json::from_str(&text)
            .with_context(|| format!("{} n'est pas du JSON valide", path.display()))?;
        Ok(check_json(&value))
    } else {
        Ok(check_text(&text))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn remplacants_acceptes() {
        for ok in [
            "https://cas.exemple.fr/cas/login?service=[https://ent.exemple.fr/]",
            "<email>",
            "<jeton:40>",
            "9999-99-99",
            "<identité:login>",
            "<AA-99-AaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAa>",
            "q=dataPremièreConnexion",
            "https://cas.iutbeziers.fr:8443/",
            "2026-10-18",
        ] {
            assert_eq!(suspicious(ok), None, "{ok}");
        }
    }

    #[test]
    fn fuites_refusees() {
        assert_eq!(suspicious("camille.martin@etu.fr"), Some("adresse e-mail"));
        assert_eq!(suspicious("x eyJhbGciOi.eyJzdWIi.sig y"), Some("jeton JWT"));
        assert_eq!(suspicious("?ticket=ST-42-AbCd"), Some("ticket CAS"));
        assert_eq!(
            suspicious("8f14e45fceea167a5a36dedd4bea2543f14e"),
            Some("jeton ou identifiant long")
        );
        assert_eq!(
            suspicious("e21304567890"),
            Some("nombre long (numéro étudiant, téléphone ?)")
        );
    }

    #[test]
    fn chemin_de_la_trouvaille() {
        let report = json!({
            "entries": [{ "url": "https://ent.fr/", "response": { "json": { "mail": "a.b@c.fr" } } }]
        });
        let findings = check_json(&report);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].path, "$.entries[0].response.json.mail");
    }

    #[test]
    fn rapport_markdown() {
        let clean = "| Notes | ScoDoc | `notes.iut.fr` | `GET /services/data.php?q=dataPremièreConnexion` | vu |\n\
                     Moyenne : `\"99.99\"`, groupe `AA A9`, ticket `ST-<…>`.";
        assert!(check_text(clean).is_empty());
        let dirty = "ok\nconnecté en tant que camille.martin@etu.fr\n";
        assert_eq!(
            check_text(dirty),
            [Finding {
                path: "ligne 2".into(),
                reason: "adresse e-mail"
            }]
        );
    }
}
