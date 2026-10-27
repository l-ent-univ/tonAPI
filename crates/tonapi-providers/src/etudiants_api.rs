//! Application de scolarité `etudiants-api` : `api/etudiants/notes`, derrière le CAS, donne le
//! numéro étudiant et la formation (profil) ainsi que les notes.
//!
//! [vu] Paul-Valéry, 27/10/2026 : réponse JSON `{numero, nom, prenom, etapes: [{libelle, codEtape,
//! nbIP, semestres, listeDAs}], epreuves, …}`. [code] client Angular de l'application : chaque
//! semestre est `{intitule, classifiedElementPedagogiques: [{codElement, libElement,
//! niveauElement, noteEpreuve1, noteEpreuve2, note, resultat}]}`. Aucun relevé réel n'a encore été
//! observé : types des notes [inconnu], d'où une lecture tolérante (texte ou nombre).

use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde_json::Value;
use tonapi_core::{
    ErrorCode, Evaluation, Grade, GradeReport, ModuleGrades, Problem, SemesterGrades,
};
use url::Url;

use crate::Context;
use crate::net::{Net, upstream};

const PROVIDER: &str = "etudiants-api";
const NOTES: &str = "api/etudiants/notes";

/// Ce que le dossier ajoute au profil.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Record {
    pub student_number: Option<String>,
    pub program: Option<String>,
}

fn root(base_url: &Url) -> Url {
    let mut base = base_url.clone();
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base.set_query(None);
    base.set_fragment(None);
    base
}

pub(crate) async fn fetch_record(ctx: &mut Context<'_>, base_url: &Url) -> Result<Record, Problem> {
    parse_record(&fetch_notes(ctx, base_url).await?)
}

pub(crate) async fn fetch_grades(
    ctx: &mut Context<'_>,
    base_url: &Url,
) -> Result<GradeReport, Problem> {
    parse_grades(&fetch_notes(ctx, base_url).await?)
}

async fn fetch_notes(ctx: &mut Context<'_>, base_url: &Url) -> Result<Value, Problem> {
    let base = root(base_url);
    let url = base
        .join(NOTES)
        .map_err(|_| upstream(PROVIDER, "adresse de l'application illisible"))?;
    let mut net = Net::open(ctx, PROVIDER)?;
    let result = read(&mut net, &base, url).await;
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

fn text(value: Option<&Value>) -> Option<String> {
    let text = match value? {
        Value::String(s) => s.split_whitespace().collect::<Vec<_>>().join(" "),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

pub(crate) fn parse_record(value: &Value) -> Result<Record, Problem> {
    let root = value
        .as_object()
        .ok_or_else(|| upstream(PROVIDER, "dossier étudiant illisible"))?;
    // Plusieurs étapes possibles (double cursus) : leurs libellés, dans l'ordre, sans doublon.
    let mut programs: Vec<String> = Vec::new();
    for step in root
        .get("etapes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(label) = text(step.get("libelle"))
            && !programs.contains(&label)
        {
            programs.push(label);
        }
    }
    Ok(Record {
        student_number: text(root.get("numero")),
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
        let step_code = text(step.get("codEtape")).unwrap_or_default();
        let Some(map) = step.get("semestres").and_then(Value::as_object) else {
            continue;
        };
        for (key, semester) in map {
            let name = text(semester.get("intitule")).unwrap_or_else(|| key.clone());
            let mut modules = Vec::new();
            let elements = semester
                .get("classifiedElementPedagogiques")
                .and_then(Value::as_array)
                .into_iter()
                .flatten();
            for (n, element) in elements.enumerate() {
                let code = text(element.get("codElement")).unwrap_or_else(|| format!("{}", n + 1));
                let name = text(element.get("libElement")).unwrap_or_else(|| code.clone());
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
                        });
                    }
                }
                let (average, average_status) = mark(element.get("note"));
                // Le résultat (ADM, AJ…) et une note finale non numérique deviennent des mentions.
                for (suffix, label, status) in [
                    ("resultat", "Résultat", text(element.get("resultat"))),
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
                        });
                    }
                }
                modules.push(ModuleGrades {
                    code,
                    name,
                    average,
                    evaluations,
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
            });
        }
    }
    Ok(GradeReport { semesters })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn aucune_note_saisie() {
        let report =
            parse_grades(&json!({ "etapes": [{ "codEtape": "XX1", "semestres": {} }] })).unwrap();
        assert!(report.semesters.is_empty());
        assert!(parse_grades(&json!({})).is_err());
    }
}
