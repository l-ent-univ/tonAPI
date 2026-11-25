//! Application de scolarité `etudiants-api` : `api/etudiants/notes`, derrière le CAS, donne le
//! numéro étudiant et la formation (profil) ainsi que les notes.
//!
//! [vu] Paul-Valéry, 27/10/2026 : réponse JSON `{numero, nom, prenom, etapes: [{libelle, codEtape,
//! nbIP, semestres, listeDAs}], epreuves, …}`. [code] client Angular de l'application : chaque
//! semestre est `{intitule, classifiedElementPedagogiques: [{codElement, libElement,
//! niveauElement, noteEpreuve1, noteEpreuve2, note, resultat}]}`. Aucun relevé réel n'a encore été
//! observé : types des notes [inconnu], d'où une lecture tolérante (texte ou nombre).
//!
//! [code] même client : `api/etudiants/calendrier` donne `epreuves` (période → épreuves
//! `{libPeriode, dateEpreuve, heureDebutEpreuve, dureeEpreuve, codEpreuve, libEpreuve,
//! salleEpreuve, batEpreuve, infoEpreuve, elementPedagogiques: [{codElement, libElement}]}`) ;
//! `api/etudiants/attestations` et `…/annuelle` donnent `attestations` (année → `[{annee, type,
//! libelle, session}]`, fichier `<base>/<annee>_<type sans '/'>_<session>`) ; `…/reussites` donne
//! `attestationReussites` (clé → nom, fichier `<base>/<clé>`). [vu] 27/10/2026 : ces trois champs
//! sont vides sur le compte observé, formats des dates et durées [inconnu].

use chrono::{NaiveDate, NaiveTime};
use chrono_tz::Tz;
use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde_json::Value;
use tonapi_core::{
    Document, DocumentKind, ErrorCode, Evaluation, Exam, Grade, GradeReport, ModuleGrades, Problem,
    SemesterGrades,
};
use url::Url;

use crate::common::{resolve_local, root, text_or_number, university_tz};
use crate::net::{Net, upstream};
use crate::{Context, Download};

const PROVIDER: &str = "etudiants-api";
const NOTES: &str = "api/etudiants/notes";
const CALENDAR: &str = "api/etudiants/calendrier";
/// Listes d'attestations et préfixe de leurs fichiers, par nature.
const ATTESTATIONS: [(DocumentKind, &str, &str); 3] = [
    (
        DocumentKind::SemesterTranscript,
        "sem",
        "api/etudiants/attestations/",
    ),
    (
        DocumentKind::AnnualTranscript,
        "anu",
        "api/etudiants/attestations/annuelle/",
    ),
    (
        DocumentKind::Achievement,
        "reu",
        "api/etudiants/attestations/reussites/",
    ),
];

/// Ce que le dossier ajoute au profil.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Record {
    pub student_number: Option<String>,
    pub program: Option<String>,
}

pub(crate) async fn fetch_record(ctx: &mut Context<'_>, base_url: &Url) -> Result<Record, Problem> {
    parse_record(&one(ctx, base_url, NOTES).await?)
}

pub(crate) async fn fetch_grades(
    ctx: &mut Context<'_>,
    base_url: &Url,
) -> Result<GradeReport, Problem> {
    parse_grades(&one(ctx, base_url, NOTES).await?)
}

pub(crate) async fn fetch_exams(
    ctx: &mut Context<'_>,
    base_url: &Url,
) -> Result<Vec<Exam>, Problem> {
    let tz = university_tz(ctx)?;
    parse_exams(&one(ctx, base_url, CALENDAR).await?, tz)
}

/// Attestations des trois listes, puis le certificat de scolarité en lien s'il est configuré.
pub(crate) async fn fetch_documents(
    ctx: &mut Context<'_>,
    base_url: &Url,
    certificate_url: Option<&Url>,
) -> Result<Vec<Document>, Problem> {
    let paths = ATTESTATIONS.map(|(_, _, path)| path.trim_end_matches('/'));
    let values = many(ctx, base_url, &paths).await?;
    let mut out = Vec::new();
    if let Some(url) = certificate_url {
        out.push(Document {
            id: "certificat".to_owned(),
            title: "Certificat de scolarité".to_owned(),
            kind: DocumentKind::Certificate,
            year: None,
            session: None,
            downloadable: false,
            url: Some(url.to_string()),
        });
    }
    for ((kind, prefix, _), value) in ATTESTATIONS.iter().zip(&values) {
        out.extend(parse_attestations(value, *kind, prefix)?);
    }
    Ok(out)
}

/// Fichier d'une attestation, par l'identifiant donné dans la liste.
pub(crate) async fn download(
    ctx: &mut Context<'_>,
    base_url: &Url,
    id: &str,
) -> Result<Download, Problem> {
    let unknown = || Problem::new(ErrorCode::NotFound).detail("document inconnu");
    let (prefix, key) = id.split_once('.').ok_or_else(unknown)?;
    let path = ATTESTATIONS
        .iter()
        .find(|(_, p, _)| *p == prefix)
        .map(|(_, _, path)| *path)
        .ok_or_else(unknown)?;
    if key.is_empty()
        || key.len() > 80
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(unknown());
    }
    let base = root(base_url);
    let url = base
        .join(path)
        .and_then(|u| u.join(key))
        .map_err(|_| upstream(PROVIDER, "adresse de l'application illisible"))?;
    let mut net = Net::open(ctx, PROVIDER)?;
    let result = async {
        if let Some(file) = get_file(&mut net, url.clone(), id).await? {
            return Ok(file);
        }
        net.open_protected(base.clone()).await?;
        get_file(&mut net, url, id).await?.ok_or_else(|| {
            Problem::new(ErrorCode::SessionExpired)
                .detail("l'application de scolarité refuse la session après le passage par le CAS")
        })
    }
    .await;
    net.save(ctx);
    result
}

/// `None` quand l'application n'a pas de session (redirection, 401, 403 ou page HTML).
async fn get_file(net: &mut Net, url: Url, id: &str) -> Result<Option<Download>, Problem> {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("application/pdf, application/octet-stream;q=0.9"),
    );
    let (status, headers, bytes) = net.download(url, headers).await?;
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(Problem::new(ErrorCode::NotFound).detail("document inconnu"));
    }
    if status.is_server_error() {
        return Err(Problem::new(ErrorCode::ServiceUnavailable)
            .detail(format!("{PROVIDER} : l'application répond HTTP {status}")));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    if !status.is_success() || content_type.starts_with("text/html") {
        return Ok(None);
    }
    let pdf = bytes.starts_with(b"%PDF");
    let file_name = format!(
        "{}.{}",
        id.replace('.', "-"),
        if pdf { "pdf" } else { "bin" }
    );
    Ok(Some(Download {
        content_type: if pdf {
            "application/pdf".to_owned()
        } else {
            content_type
        },
        file_name,
        bytes,
    }))
}

async fn one(ctx: &mut Context<'_>, base_url: &Url, path: &str) -> Result<Value, Problem> {
    let mut values = many(ctx, base_url, &[path]).await?;
    values
        .pop()
        .ok_or_else(|| upstream(PROVIDER, "réponse absente"))
}

/// Routes JSON lues dans la même session réseau (un seul passage par le CAS).
async fn many(
    ctx: &mut Context<'_>,
    base_url: &Url,
    paths: &[&str],
) -> Result<Vec<Value>, Problem> {
    let base = root(base_url);
    let mut net = Net::open(ctx, PROVIDER)?;
    let result = async {
        let mut out = Vec::with_capacity(paths.len());
        for path in paths {
            let url = base
                .join(path)
                .map_err(|_| upstream(PROVIDER, "adresse de l'application illisible"))?;
            out.push(read(&mut net, &base, url).await?);
        }
        Ok(out)
    }
    .await;
    net.save(ctx);
    result
}

/// La route JSON, après une entrée par le CAS si l'application n'a pas encore de session.
async fn read(net: &mut Net, base: &Url, url: Url) -> Result<Value, Problem> {
    if let Some(value) = get_json(net, url.clone()).await? {
        return Ok(value);
    }
    net.open_protected(base.clone()).await?;
    get_json(net, url).await?.ok_or_else(|| {
        Problem::new(ErrorCode::SessionExpired)
            .detail("l'application de scolarité refuse la session après le passage par le CAS")
    })
}

/// `None` quand l'application n'a pas de session (redirection, 401, 403 ou page HTML).
async fn get_json(net: &mut Net, url: Url) -> Result<Option<Value>, Problem> {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    let (status, _, body) = net.request(Method::GET, url, headers, None).await?;
    if status.is_server_error() {
        return Err(Problem::new(ErrorCode::ServiceUnavailable)
            .detail(format!("{PROVIDER} : l'application répond HTTP {status}")));
    }
    if !status.is_success() {
        return Ok(None);
    }
    Ok(serde_json::from_str(&body).ok())
}

/// Étapes (inscriptions) retenues au plus dans le dossier.
const MAX_PROGRAMS: usize = 16;

pub(crate) fn parse_record(value: &Value) -> Result<Record, Problem> {
    let root = value
        .as_object()
        .ok_or_else(|| upstream(PROVIDER, "dossier étudiant illisible"))?;
    // Plusieurs étapes possibles (double cursus) : leurs libellés, dans l'ordre, sans doublon.
    // Au plus `MAX_PROGRAMS` : une réponse démesurée ne gonfle ni le calcul ni la réponse.
    let mut programs: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for step in root
        .get("etapes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if programs.len() >= MAX_PROGRAMS {
            break;
        }
        if let Some(label) = text_or_number(step.get("libelle"))
            && seen.insert(label.clone())
        {
            programs.push(label);
        }
    }
    Ok(Record {
        student_number: text_or_number(root.get("numero")),
        program: (!programs.is_empty()).then(|| programs.join(" ; ")),
    })
}

/// Note sur 20 : nombre, ou texte `12.5`, `12,50`. Autre texte (ABI, DEF…) : statut.
fn mark(value: Option<&Value>) -> (Option<Grade>, Option<String>) {
    let grade = |v: f64| {
        (v.is_finite() && (0.0..=20.0).contains(&v)).then_some(Grade {
            value: v,
            out_of: 20.0,
        })
    };
    match value {
        Some(Value::Number(n)) => (n.as_f64().and_then(grade), None),
        Some(Value::String(s)) => {
            let s = s.trim();
            if s.is_empty() {
                return (None, None);
            }
            match s.replace(',', ".").parse::<f64>().ok().and_then(grade) {
                Some(g) => (Some(g), None),
                None => (None, Some(s.to_owned())),
            }
        }
        _ => (None, None),
    }
}

pub(crate) fn parse_grades(value: &Value) -> Result<GradeReport, Problem> {
    let steps = value
        .get("etapes")
        .and_then(Value::as_array)
        .ok_or_else(|| upstream(PROVIDER, "liste des étapes absente"))?;
    let mut semesters = Vec::new();
    for step in steps {
        let step_code = text_or_number(step.get("codEtape")).unwrap_or_default();
        let Some(map) = step.get("semestres").and_then(Value::as_object) else {
            continue;
        };
        for (key, semester) in map {
            let name = text_or_number(semester.get("intitule")).unwrap_or_else(|| key.clone());
            let mut modules = Vec::new();
            let elements = semester
                .get("classifiedElementPedagogiques")
                .and_then(Value::as_array)
                .into_iter()
                .flatten();
            for (n, element) in elements.enumerate() {
                let code = text_or_number(element.get("codElement"))
                    .unwrap_or_else(|| format!("{}", n + 1));
                let name =
                    text_or_number(element.get("libElement")).unwrap_or_else(|| code.clone());
                let mut evaluations = Vec::new();
                for (field, label) in [
                    ("noteEpreuve1", "Évaluation 1"),
                    ("noteEpreuve2", "Évaluation 2"),
                ] {
                    let (grade, status) = mark(element.get(field));
                    if grade.is_some() || status.is_some() {
                        evaluations.push(Evaluation {
                            id: format!("{code}-{field}"),
                            name: label.to_owned(),
                            date: None,
                            grade,
                            coefficient: None,
                            status,
                            ..Evaluation::default()
                        });
                    }
                }
                let (average, average_status) = mark(element.get("note"));
                // Le résultat (ADM, AJ…) et une note finale non numérique deviennent des mentions.
                for (suffix, label, status) in [
                    (
                        "resultat",
                        "Résultat",
                        text_or_number(element.get("resultat")),
                    ),
                    ("note", "Note", average_status),
                ] {
                    if let Some(status) = status {
                        evaluations.push(Evaluation {
                            id: format!("{code}-{suffix}"),
                            name: label.to_owned(),
                            date: None,
                            grade: None,
                            coefficient: None,
                            status: Some(status),
                            ..Evaluation::default()
                        });
                    }
                }
                modules.push(ModuleGrades {
                    code,
                    name,
                    average,
                    evaluations,
                    ..ModuleGrades::default()
                });
            }
            semesters.push(SemesterGrades {
                id: if step_code.is_empty() {
                    key.clone()
                } else {
                    format!("{step_code}-{key}")
                },
                name,
                average: None,
                rank: None,
                units: Vec::new(),
                modules,
                ..SemesterGrades::default()
            });
        }
    }
    Ok(GradeReport {
        semesters,
        ..GradeReport::default()
    })
}

/// Épreuves de toutes les périodes, triées par date (les épreuves sans date à la fin).
pub(crate) fn parse_exams(value: &Value, tz: Tz) -> Result<Vec<Exam>, Problem> {
    let periods = value
        .get("epreuves")
        .and_then(Value::as_object)
        .ok_or_else(|| upstream(PROVIDER, "calendrier des épreuves absent"))?;
    let mut exams = Vec::new();
    for (key, list) in periods {
        let Some(list) = list.as_array() else {
            return Err(upstream(PROVIDER, "période d'épreuves illisible"));
        };
        for (n, item) in list.iter().enumerate() {
            let code = text_or_number(item.get("codEpreuve"));
            let date = text_or_number(item.get("dateEpreuve")).and_then(|d| exam_date(&d));
            let time = text_or_number(item.get("heureDebutEpreuve")).and_then(|t| exam_time(&t));
            let start = match (date, time) {
                (Some(date), Some(time)) => resolve_local(tz, date.and_time(time)),
                // Date sans heure : minuit local, pour que le jour reste lisible.
                (Some(date), None) => resolve_local(tz, date.and_time(NaiveTime::MIN)),
                _ => None,
            };
            let courses = item
                .get("elementPedagogiques")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|e| {
                    match (
                        text_or_number(e.get("codElement")),
                        text_or_number(e.get("libElement")),
                    ) {
                        (Some(c), Some(l)) => Some(format!("{c} - {l}")),
                        (c, l) => c.or(l),
                    }
                })
                .collect();
            let id = code.clone().unwrap_or_else(|| format!("{}", n + 1));
            exams.push(Exam {
                id: format!("{key}-{id}"),
                title: text_or_number(item.get("libEpreuve"))
                    .or(code)
                    .unwrap_or_else(|| "Épreuve".to_owned()),
                start,
                duration_minutes: text_or_number(item.get("dureeEpreuve"))
                    .and_then(|d| minutes(&d)),
                room: text_or_number(item.get("salleEpreuve")),
                building: text_or_number(item.get("batEpreuve")),
                session: text_or_number(item.get("libPeriode")),
                courses,
                notice: text_or_number(item.get("infoEpreuve")),
            });
        }
    }
    exams.sort_by(|a, b| match (a.start, b.start) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    Ok(exams)
}

/// `JJ/MM/AAAA` (Apogée) ou `AAAA-MM-JJ`, éventuellement suivi d'une heure ignorée.
fn exam_date(raw: &str) -> Option<NaiveDate> {
    let day = raw.split(['T', ' ']).next()?;
    NaiveDate::parse_from_str(day, "%d/%m/%Y")
        .or_else(|_| NaiveDate::parse_from_str(day, "%Y-%m-%d"))
        .ok()
}

/// `14:00`, `14h00`, `14h`, `14.00` ou `1400`.
fn exam_time(raw: &str) -> Option<NaiveTime> {
    let raw = raw.trim().to_ascii_lowercase();
    let (h, m) = match raw.split_once([':', 'h', '.']) {
        Some((h, m)) => (h, if m.is_empty() { "0" } else { m }),
        None if raw.len() == 4 => (raw.get(..2)?, raw.get(2..)?),
        None => return None,
    };
    let m = m.get(..2).unwrap_or(m);
    NaiveTime::from_hms_opt(h.trim().parse().ok()?, m.trim().parse().ok()?, 0)
}

/// Durée : `2:00`, `2h00`, `2h`, `1h30` ; un nombre seul est lu en minutes [déduit].
fn minutes(raw: &str) -> Option<u32> {
    let raw = raw.trim().to_ascii_lowercase();
    if let Some((h, m)) = raw.split_once([':', 'h']) {
        let h: u32 = h.trim().parse().ok()?;
        let m: u32 = if m.trim().is_empty() {
            0
        } else {
            m.trim().get(..2.min(m.trim().len()))?.parse().ok()?
        };
        return if m < 60 {
            h.checked_mul(60)?.checked_add(m)
        } else {
            None
        };
    }
    raw.parse().ok().filter(|&v| v > 0)
}

/// Attestations d'une liste : `attestations` (année → fichiers) ou `attestationReussites`.
pub(crate) fn parse_attestations(
    value: &Value,
    kind: DocumentKind,
    prefix: &str,
) -> Result<Vec<Document>, Problem> {
    let bad = || upstream(PROVIDER, "liste des attestations illisible");
    let mut out = Vec::new();
    if kind == DocumentKind::Achievement {
        let map = value
            .get("attestationReussites")
            .and_then(Value::as_object)
            .ok_or_else(bad)?;
        for (key, name) in map {
            out.push(Document {
                id: format!("{prefix}.{key}"),
                title: text_or_number(Some(name))
                    .unwrap_or_else(|| "Attestation de réussite".to_owned()),
                kind,
                year: None,
                session: None,
                downloadable: true,
                url: None,
            });
        }
        return Ok(out);
    }
    let years = value
        .get("attestations")
        .and_then(Value::as_object)
        .ok_or_else(bad)?;
    for (year, list) in years {
        for item in list.as_array().ok_or_else(bad)? {
            let annee = text_or_number(item.get("annee")).unwrap_or_else(|| year.clone());
            let kind_code = text_or_number(item.get("type"))
                .ok_or_else(bad)?
                .replace('/', "");
            let session = text_or_number(item.get("session"));
            let key = format!("{annee}_{kind_code}_{}", session.as_deref().unwrap_or(""));
            out.push(Document {
                id: format!("{prefix}.{key}"),
                title: text_or_number(item.get("libelle")).unwrap_or(kind_code),
                kind,
                year: Some(annee),
                session,
                downloadable: true,
                url: None,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `Vec::contains` rendait le dédoublonnage quadratique.
    #[test]
    fn etapes_bornees() {
        let etapes: Vec<Value> = (0..50_000)
            .map(|i| json!({ "libelle": format!("Étape {i}") }))
            .collect();
        let record = parse_record(&json!({ "etapes": etapes })).unwrap();
        let program = record.program.unwrap();
        assert_eq!(program.split(" ; ").count(), MAX_PROGRAMS);
        assert!(program.starts_with("Étape 0 ; Étape 1"));
    }

    #[test]
    fn numero_et_formation() {
        let value = json!({
            "numero": "21000000", "nom": "DEMO", "prenom": "Camille", "ajac": false,
            "etapes": [
                { "libelle": "Licence 1  Arts", "codEtape": "XX1", "semestres": {} },
                { "libelle": "Licence 1  Arts", "codEtape": "XX1", "semestres": {} },
                { "libelle": "Certificat demo", "codEtape": "XX2" }
            ],
            "epreuves": {}
        });
        let record = parse_record(&value).unwrap();
        assert_eq!(record.student_number.as_deref(), Some("21000000"));
        assert_eq!(
            record.program.as_deref(),
            Some("Licence 1 Arts ; Certificat demo")
        );
    }

    #[test]
    fn champs_absents_et_numero_entier() {
        assert_eq!(parse_record(&json!({})).unwrap(), Record::default());
        let record =
            parse_record(&json!({ "numero": 21_000_000, "etapes": [{ "codEtape": "X" }] }))
                .unwrap();
        assert_eq!(record.student_number.as_deref(), Some("21000000"));
        assert_eq!(record.program, None);
        assert!(parse_record(&json!([])).is_err());
    }

    #[test]
    fn notes_par_semestre_et_element() {
        let value = json!({ "etapes": [{
            "codEtape": "XX1", "libelle": "Licence 1 Arts", "nbIP": 1,
            "semestres": { "S1": { "intitule": "Semestre 1", "classifiedElementPedagogiques": [
                { "codElement": "UE1", "libElement": "Fondamentaux", "niveauElement": 1,
                  "noteEpreuve1": "12,5", "noteEpreuve2": "", "note": 13.25, "resultat": "ADM" },
                { "codElement": "EC11", "libElement": "Histoire", "niveauElement": 2,
                  "noteEpreuve1": "ABI", "note": "DEF", "resultat": null },
                { "codElement": "EC12", "libElement": "Hors barème", "note": "25" }
            ] } }
        }] });
        let report = parse_grades(&value).unwrap();
        let s1 = &report.semesters[0];
        assert_eq!((s1.id.as_str(), s1.name.as_str()), ("XX1-S1", "Semestre 1"));
        let ue = &s1.modules[0];
        assert_eq!(ue.average.as_ref().map(|g| g.value), Some(13.25));
        assert_eq!(
            ue.evaluations[0].grade.as_ref().map(|g| g.value),
            Some(12.5)
        );
        assert_eq!(
            ue.evaluations.len(),
            2,
            "éval 1 et résultat ; éval 2 vide ignorée"
        );
        assert_eq!(ue.evaluations[1].status.as_deref(), Some("ADM"));
        // Une mention n'est jamais une note nulle.
        let ec = &s1.modules[1];
        assert_eq!(ec.average, None);
        assert_eq!(ec.evaluations[0].status.as_deref(), Some("ABI"));
        assert!(
            ec.evaluations
                .iter()
                .any(|e| e.status.as_deref() == Some("DEF"))
        );
        assert_eq!(s1.modules[2].average, None, "25/20 n'est pas une note");
    }

    #[test]
    fn epreuves_triees_avec_fuseau() {
        let value = json!({ "epreuves": { "S1": [
            { "libPeriode": "Session 1", "codEpreuve": "EP2", "libEpreuve": "Histoire",
              "dateEpreuve": "15/12/2026", "heureDebutEpreuve": "14h00", "dureeEpreuve": "2h00",
              "salleEpreuve": "A101", "batEpreuve": "Bât. A", "infoEpreuve": " ",
              "elementPedagogiques": [{ "codElement": "EC11", "libElement": "Histoire" }] },
            { "codEpreuve": "EP1", "libEpreuve": "Lettres", "dateEpreuve": "2026-12-14",
              "heureDebutEpreuve": "09:30", "dureeEpreuve": "90", "elementPedagogiques": [] },
            { "codEpreuve": "EP3", "dateEpreuve": "à fixer" }
        ] } });
        let exams = parse_exams(&value, chrono_tz::Europe::Paris).unwrap();
        let ids: Vec<&str> = exams.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["S1-EP1", "S1-EP2", "S1-EP3"]);
        assert_eq!(
            exams[1].start.map(|d| d.to_rfc3339()).as_deref(),
            Some("2026-12-15T14:00:00+01:00")
        );
        assert_eq!(exams[0].duration_minutes, Some(90));
        assert_eq!(exams[1].duration_minutes, Some(120));
        assert_eq!(exams[1].courses, ["EC11 - Histoire"]);
        assert_eq!(exams[1].notice, None, "consigne vide ignorée");
        assert_eq!((exams[2].start, exams[2].title.as_str()), (None, "EP3"));
        assert!(parse_exams(&json!({}), chrono_tz::Europe::Paris).is_err());
    }

    #[test]
    fn heure_sautee_au_passage_a_l_ete() {
        let value = json!({ "epreuves": { "S2": [
            { "codEpreuve": "E", "dateEpreuve": "28/03/2027", "heureDebutEpreuve": "02:30" }
        ] } });
        let exams = parse_exams(&value, chrono_tz::Europe::Paris).unwrap();
        assert_eq!(
            exams[0].start.map(|d| d.to_rfc3339()).as_deref(),
            Some("2027-03-28T03:30:00+02:00")
        );
    }

    #[test]
    fn attestations_et_reussites() {
        let sem = json!({ "attestations": { "2025": [
            { "annee": "2025", "type": "S1/L1", "libelle": "Semestre 1", "session": "1" }
        ] }, "attestationReussites": {} });
        let docs = parse_attestations(&sem, DocumentKind::SemesterTranscript, "sem").unwrap();
        assert_eq!(docs[0].id, "sem.2025_S1L1_1");
        assert_eq!(docs[0].title, "Semestre 1");
        assert_eq!(docs[0].year.as_deref(), Some("2025"));
        let reu = json!({ "attestations": {}, "attestationReussites": { "7": "Licence Arts" } });
        let docs = parse_attestations(&reu, DocumentKind::Achievement, "reu").unwrap();
        assert_eq!(
            (docs[0].id.as_str(), docs[0].title.as_str()),
            ("reu.7", "Licence Arts")
        );
        assert!(parse_attestations(&json!({}), DocumentKind::AnnualTranscript, "anu").is_err());
    }

    #[test]
    fn aucune_note_saisie() {
        let report =
            parse_grades(&json!({ "etapes": [{ "codEtape": "XX1", "semestres": {} }] })).unwrap();
        assert!(report.semesters.is_empty());
        assert!(parse_grades(&json!({})).is_err());
    }
}
