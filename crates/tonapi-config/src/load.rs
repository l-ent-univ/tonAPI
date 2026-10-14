//! Lecture du dossier `universities/`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::diagnostics::from_toml_error;
use crate::establishment::EstablishmentFile;
use crate::services::Services;
use crate::university::UniversityFile;
use crate::validate::{Issue, check_establishment, check_university};

/// Universités acceptées, par identifiant.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    universities: BTreeMap<String, ResolvedUniversity>,
}

/// Une université et ses composantes acceptées.
#[derive(Debug, Clone)]
pub struct ResolvedUniversity {
    /// Dossier de l'université.
    pub dir: PathBuf,
    pub university: UniversityFile,
    /// Composantes, par identifiant.
    pub establishments: BTreeMap<String, EstablishmentFile>,
}

impl ResolvedUniversity {
    /// Services d'un étudiant de la composante donnée (ceux de l'université si `None` ou inconnue).
    #[must_use]
    pub fn services_for(&self, establishment: Option<&str>) -> Services {
        let base = &self.university.services;
        match establishment.and_then(|id| self.establishments.get(id)) {
            Some(e) => base.merged_with(&e.services),
            None => base.clone(),
        }
    }

    /// Composante marquée `default = true`.
    #[must_use]
    pub fn default_establishment(&self) -> Option<&EstablishmentFile> {
        self.establishments.values().find(|e| e.default)
    }

    /// Certificats PEM supplémentaires pour la composante donnée (université + composante).
    #[must_use]
    pub fn extra_ca(&self, establishment: Option<&str>) -> Vec<PathBuf> {
        let own = establishment
            .and_then(|id| self.establishments.get(id))
            .and_then(|e| e.tls.as_ref());
        self.university
            .tls
            .iter()
            .chain(own)
            .flat_map(|t| &t.extra_ca)
            .map(|p| self.dir.join(p))
            .collect()
    }
}

/// Résultat d'un chargement : fichiers acceptés et problèmes des fichiers refusés.
#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    pub accepted: Vec<PathBuf>,
    pub rejected: Vec<Issue>,
}

impl LoadReport {
    /// Vrai si aucun fichier n'a été refusé.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.rejected.is_empty()
    }
}

impl Registry {
    /// Lit `dir/<id>/university.toml` et `dir/<id>/establishments/*.toml`.
    ///
    /// Un `university.toml` refusé écarte l'université entière ; une composante refusée n'écarte qu'elle.
    /// Les dossiers dont le nom commence par `.` ou `_` sont ignorés.
    #[must_use]
    pub fn load(dir: &Path) -> (Registry, LoadReport) {
        let mut registry = Registry::default();
        let mut report = LoadReport::default();

        let entries = match sorted_entries(dir) {
            Ok(entries) => entries,
            Err(e) => {
                report.rejected.push(io_issue(dir, &e));
                return (registry, report);
            }
        };

        for univ_dir in entries.into_iter().filter(|p| p.is_dir()) {
            let Some(dir_name) = file_name(&univ_dir) else {
                continue;
            };
            if dir_name.starts_with('.') || dir_name.starts_with('_') {
                continue;
            }
            if let Some(resolved) = load_university(&univ_dir, &dir_name, &mut report) {
                registry
                    .universities
                    .insert(resolved.university.id.clone(), resolved);
            }
        }
        (registry, report)
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&ResolvedUniversity> {
        self.universities.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ResolvedUniversity> {
        self.universities.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.universities.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.universities.is_empty()
    }
}

fn load_university(
    univ_dir: &Path,
    dir_name: &str,
    report: &mut LoadReport,
) -> Option<ResolvedUniversity> {
    let file = univ_dir.join("university.toml");
    let (university, src) = parse::<UniversityFile>(&file, report)?;
    let issues = check_university(&file, &src, dir_name, &university);
    if !issues.is_empty() {
        report.rejected.extend(issues);
        return None;
    }
    report.accepted.push(file);

    let mut establishments = BTreeMap::new();
    let est_dir = univ_dir.join("establishments");
    if est_dir.is_dir() {
        let entries = match sorted_entries(&est_dir) {
            Ok(entries) => entries,
            Err(e) => {
                report.rejected.push(io_issue(&est_dir, &e));
                Vec::new()
            }
        };
        let mut default: Option<String> = None;
        for file in entries
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        {
            let stem = file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Some((establishment, src)) = parse::<EstablishmentFile>(&file, report) else {
                continue;
            };
            let mut issues =
                check_establishment(&file, &src, &stem, &establishment, &university, univ_dir);
            if establishment.default
                && let Some(other) = &default
            {
                let position = crate::diagnostics::locate(&src, "default");
                issues.push(Issue {
                    file: file.clone(),
                    line: position.map(|p| p.0),
                    column: position.map(|p| p.1),
                    path: Some("default".to_owned()),
                    message: format!("`{other}` est déjà la composante par défaut"),
                    suggestion: None,
                });
            }
            if !issues.is_empty() {
                report.rejected.extend(issues);
                continue;
            }
            if establishment.default {
                default = Some(establishment.id.clone());
            }
            report.accepted.push(file);
            establishments.insert(establishment.id.clone(), establishment);
        }
    }

    Some(ResolvedUniversity {
        dir: univ_dir.to_path_buf(),
        university,
        establishments,
    })
}

fn parse<T: DeserializeOwned>(file: &Path, report: &mut LoadReport) -> Option<(T, String)> {
    let src = match fs::read_to_string(file) {
        Ok(src) => src,
        Err(e) => {
            report.rejected.push(io_issue(file, &e));
            return None;
        }
    };
    match toml::from_str::<T>(&src) {
        Ok(value) => Some((value, src)),
        Err(e) => {
            report.rejected.push(from_toml_error(file, &src, &e));
            None
        }
    }
}

fn sorted_entries(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort();
    Ok(entries)
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().into_owned())
}

fn io_issue(path: &Path, e: &std::io::Error) -> Issue {
    Issue {
        file: path.to_path_buf(),
        line: None,
        column: None,
        path: None,
        message: format!("lecture impossible : {e}"),
        suggestion: None,
    }
}
