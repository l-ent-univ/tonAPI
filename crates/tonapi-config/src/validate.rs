//! Règles que le typage seul ne garantit pas.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::SCHEMA_VERSION;
use crate::diagnostics::locate;
use crate::establishment::EstablishmentFile;
use crate::services::{AppsService, ProfileService, Services};
use crate::templates::is_placeholder;
use crate::university::{PortalConfig, Tls, UniversityFile};

/// Problème trouvé dans un fichier. Un seul problème suffit à refuser le fichier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub file: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    /// Chemin de la clé fautive (ex. `services.grades.base-url`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub message: String,
    /// Valeur probablement voulue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file.display())?;
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
            if let Some(column) = self.column {
                write!(f, ":{column}")?;
            }
        }
        write!(f, ": ")?;
        if let Some(path) = &self.path {
            write!(f, "{path} : ")?;
        }
        write!(f, "{}", self.message)?;
        if let Some(s) = &self.suggestion {
            write!(f, " — voulez-vous dire `{s}` ?")?;
        }
        Ok(())
    }
}

/// Accumule les problèmes d'un fichier.
pub(crate) struct Checker<'a> {
    file: &'a Path,
    src: &'a str,
    pub issues: Vec<Issue>,
}

impl<'a> Checker<'a> {
    pub fn new(file: &'a Path, src: &'a str) -> Self {
        Self {
            file,
            src,
            issues: Vec::new(),
        }
    }

    /// Ajoute un problème, positionné sur la première occurrence de `needle` si elle est donnée.
    pub fn push(&mut self, path: Option<&str>, needle: Option<&str>, message: impl Into<String>) {
        let position = needle.and_then(|n| locate(self.src, n));
        self.issues.push(Issue {
            file: self.file.to_path_buf(),
            line: position.map(|p| p.0),
            column: position.map(|p| p.1),
            path: path.map(str::to_owned),
            message: message.into(),
            suggestion: None,
        });
    }

    fn schema_version(&mut self, version: u32) {
        if version != SCHEMA_VERSION {
            self.push(
                Some("schema-version"),
                Some("schema-version"),
                format!("version {version} non prise en charge (attendu : {SCHEMA_VERSION})"),
            );
        }
    }

    fn id(&mut self, id: &str, expected: &str, what: &str) {
        if !is_valid_id(id) {
            self.push(
                Some("id"),
                Some(&format!("\"{id}\"")),
                format!("identifiant `{id}` invalide : seulement a-z, 0-9 et `-`"),
            );
        } else if id != expected {
            self.push(
                Some("id"),
                Some(&format!("\"{id}\"")),
                format!("l'identifiant `{id}` doit être identique au nom {what} `{expected}`"),
            );
        }
    }

    /// Parcourt toutes les valeurs : URL en `https`, aucune valeur à remplir.
    fn values<T: Serialize>(&mut self, value: &T) {
        let Ok(json) = serde_json::to_value(value) else {
            return;
        };
        let mut found = Vec::new();
        walk(&json, String::new(), &mut found);
        for (path, text) in found {
            if is_placeholder(&text) {
                self.push(Some(&path), Some(&text), "valeur à remplir");
            } else if text.contains("://") && !text.starts_with("https://") {
                self.push(
                    Some(&path),
                    Some(&text),
                    format!("`{text}` : seules les URL en https:// sont acceptées"),
                );
            }
        }
    }

    fn tls(&mut self, tls: Option<&Tls>, base: &Path) {
        for path in tls.iter().flat_map(|t| &t.extra_ca) {
            let needle = path.to_string_lossy();
            match std::fs::read_to_string(base.join(path)) {
                Ok(pem) if pem.contains("-----BEGIN CERTIFICATE-----") => {}
                Ok(_) => self.push(
                    Some("tls.extra-ca"),
                    Some(&needle),
                    format!("`{needle}` n'est pas un certificat PEM"),
                ),
                Err(e) => self.push(
                    Some("tls.extra-ca"),
                    Some(&needle),
                    format!("`{needle}` illisible : {e}"),
                ),
            }
        }
    }

    fn portal_services(&mut self, services: &Services, portal: &PortalConfig) {
        if *portal != PortalConfig::None {
            return;
        }
        if services.apps == Some(AppsService::Portal) {
            self.push(
                Some("services.apps"),
                Some("[services.apps]"),
                "`provider = \"portal\"` demande un `[portal]` dans university.toml",
            );
        }
        if matches!(services.profile, Some(ProfileService::Portal { .. })) {
            self.push(
                Some("services.profile"),
                Some("[services.profile]"),
                "`provider = \"portal\"` demande un `[portal]` dans university.toml",
            );
        }
    }
}

/// Identifiant d'université ou de composante : `a-z`, `0-9` et `-`, sans `-` au début ni à la fin.
#[must_use]
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && !id.ends_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn walk(value: &Value, path: String, out: &mut Vec<(String, String)>) {
    let join = |key: &str| {
        if path.is_empty() {
            key.to_owned()
        } else {
            format!("{path}.{key}")
        }
    };
    match value {
        Value::String(s) => out.push((path, s.clone())),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                walk(item, format!("{path}[{i}]"), out);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                walk(item, join(key), out);
            }
        }
        _ => {}
    }
}

/// Vérifie `university.toml`. `dir_name` est le nom du dossier de l'université.
pub(crate) fn check_university(
    file: &Path,
    src: &str,
    dir_name: &str,
    university: &UniversityFile,
) -> Vec<Issue> {
    let mut c = Checker::new(file, src);
    c.schema_version(university.schema_version);
    c.id(&university.id, dir_name, "du dossier");
    c.values(university);
    c.tls(
        university.tls.as_ref(),
        file.parent().unwrap_or(Path::new(".")),
    );
    c.portal_services(&university.services, &university.portal);
    c.issues
}

/// Vérifie un fichier de composante. `stem` est le nom du fichier sans `.toml`.
pub(crate) fn check_establishment(
    file: &Path,
    src: &str,
    stem: &str,
    establishment: &EstablishmentFile,
    university: &UniversityFile,
    university_dir: &Path,
) -> Vec<Issue> {
    let mut c = Checker::new(file, src);
    c.schema_version(establishment.schema_version);
    c.id(&establishment.id, stem, "du fichier");
    c.values(establishment);
    c.tls(establishment.tls.as_ref(), university_dir);
    c.portal_services(&establishment.services, &university.portal);
    for (i, rule) in establishment.affiliation.rules.iter().enumerate() {
        if rule.pattern_count() != 1 {
            c.push(
                Some(&format!("affiliation.rules[{i}]")),
                Some("[[affiliation.rules]]"),
                "une règle doit avoir exactement un de `contains`, `prefix`, `suffix`, non vide",
            );
        }
    }
    c.issues
}
