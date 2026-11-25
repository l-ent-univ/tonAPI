//! Analyse pure des réponses de la passerelle ScoDoc, sans réseau : classement de la réponse de
//! `data.php`, relevé de notes (bulletin BUT) et absences.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime};
use chrono_tz::Tz;
use serde_json::Value;
use tonapi_core::{
    Absence, AbsenceReport, AbsenceTotals, DistributionUnavailable, ErrorCode, Evaluation,
    EvaluationKind, Grade, GradeDistribution, GradeNotice, GradeReport, GradeStats, GroupRank,
    JuryDecision, ModuleGrades, Problem, SemesterGrades, SemesterRef, StudentGroup, UnitGrade,
    UnitModule,
};

use super::{PROVIDER, bad};
use crate::common::{html_to_lines, id_string, resolve_local};

/// Les notes et moyennes du bulletin ScoDoc sont ramenées sur 20 [déduit]. `note.max` du bulletin
/// est la meilleure note de la promotion, pas le barème.
const OUT_OF: f64 = 20.0;
/// Mention donnée à `"~"` (pas encore de note) [déduit].
const PENDING: &str = "ATT";

/// Réponse de `data.php`, une fois lue.
#[derive(Debug)]
pub(super) enum Reply {
    Data(Value),
    /// `{"redirect":"/services/doAuth.php"}` en HTTP 200 : la session PHP est perdue.
    SessionLost,
}

/// Distingue les données, la session perdue et l'erreur applicative de la passerelle.
pub(super) fn classify_reply(value: Value) -> Result<Reply, Problem> {
    let Value::Object(map) = &value else {
        return Err(bad("réponse inattendue (objet JSON attendu)"));
    };
    if map.contains_key("redirect") {
        return Ok(Reply::SessionLost);
    }
    if map.contains_key("erreur") {
        // Le message est écrit par l'IUT et peut citer l'étudiant : on ne le recopie pas.
        return Err(Problem::new(ErrorCode::ServiceUnavailable).detail(format!(
            "{PROVIDER} : la passerelle a répondu par une erreur"
        )));
    }
    Ok(Reply::Data(value))
}

/// Contenu d'une cellule de note.
#[derive(Debug, PartialEq)]
pub(super) enum Cell {
    Value(f64),
    /// Mention à la place d'une note (`ABS`, `EXC`, `ATT`…).
    Status(String),
    Empty,
}

/// `"12.50"` ou `"12,50"`. `NaN` et `inf` ne sont pas des notes.
fn decimal(text: &str) -> Option<f64> {
    let value: f64 = text.trim().replace(',', ".").parse().ok()?;
    value.is_finite().then_some(value)
}

/// Nombre envoyé comme nombre ou comme chaîne.
fn lossy_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64().filter(|v| v.is_finite()),
        Value::String(s) => decimal(s),
        _ => None,
    }
}

/// `"12.50"` → note ; `"~"` → en attente ; `"-"` et `""` → vide ; `"ABS"`, `"EXC"`… → mention.
/// Une valeur qui n'est pas un nombre ne devient jamais zéro.
pub(super) fn cell(value: Option<&Value>) -> Cell {
    match value {
        Some(Value::String(s)) => {
            let text = s.trim();
            if text.is_empty() || text == "-" {
                Cell::Empty
            } else if text == "~" {
                Cell::Status(PENDING.to_owned())
            } else if let Some(v) = decimal(text) {
                Cell::Value(v)
            } else {
                Cell::Status(text.to_uppercase())
            }
        }
        Some(number @ Value::Number(_)) => lossy_f64(number).map_or(Cell::Empty, Cell::Value),
        _ => Cell::Empty,
    }
}

/// `{"value": …, "min": …}` (bulletin) ou la valeur elle-même (moyenne d'UE d'une ressource).
fn scalar(value: Option<&Value>) -> Option<&Value> {
    match value {
        Some(object @ Value::Object(_)) => object.get("value"),
        other => other,
    }
}

fn average(value: Option<&Value>) -> Option<Grade> {
    match cell(scalar(value)) {
        Cell::Value(value) => Some(Grade {
            value,
            out_of: OUT_OF,
        }),
        Cell::Status(_) | Cell::Empty => None,
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    let s = value?.as_str()?.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

/// Entrées d'un objet. PHP envoie `[]` pour un objet vide.
fn entries<'a>(
    value: Option<&'a Value>,
    what: &str,
) -> Result<Vec<(&'a String, &'a Value)>, Problem> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) if a.is_empty() => Ok(Vec::new()),
        Some(Value::Object(map)) => Ok(map.iter().collect()),
        Some(_) => Err(bad(format!("`{what}` : objet attendu"))),
    }
}

/// Éléments d'un tableau. PHP envoie parfois `{}` pour un tableau vide.
fn items<'a>(value: Option<&'a Value>, what: &str) -> Result<&'a [Value], Problem> {
    match value {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(a)) => Ok(a),
        Some(Value::Object(m)) if m.is_empty() => Ok(&[]),
        Some(_) => Err(bad(format!("`{what}` : tableau attendu"))),
    }
}

/// Tri « naturel » : `R1.2` avant `R1.10`.
pub(super) fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let run = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(d) = chars.next_if(char::is_ascii_digit) {
                        digits.push(d);
                    }
                    digits.trim_start_matches('0').to_owned()
                };
                let (left, right) = (run(&mut a), run(&mut b));
                let order = left.len().cmp(&right.len()).then_with(|| left.cmp(&right));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

fn sorted<'a>(mut list: Vec<(&'a String, &'a Value)>) -> Vec<(&'a String, &'a Value)> {
    list.sort_by(|(a, _), (b, _)| natural_cmp(a, b));
    list
}

/// Date d'une évaluation dans le fuseau de l'université. Un instant avec décalage est converti ;
/// une date sans fuseau est prise telle quelle.
pub(super) fn evaluation_date(value: Option<&Value>, tz: Tz) -> Option<NaiveDate> {
    let text = value?.as_str()?.trim();
    if let Ok(instant) = DateTime::parse_from_rfc3339(text) {
        return Some(instant.with_timezone(&tz).date_naive());
    }
    NaiveDate::parse_from_str(text.get(..10)?, "%Y-%m-%d").ok()
}

/// Notes de la promotion (`min`, `moy`, `max`) d'une cellule `{value, min, max, moy}`.
fn stats(value: Option<&Value>) -> Option<GradeStats> {
    let value = value?;
    let read = |key: &str| match cell(value.get(key)) {
        Cell::Value(number) => Some(number),
        Cell::Status(_) | Cell::Empty => None,
    };
    let stats = GradeStats {
        min: read("min"),
        mean: read("moy"),
        max: read("max"),
    };
    (stats != GradeStats::default()).then_some(stats)
}

/// Un réglage d'affichage du bulletin (`options.show_…`) : permis sauf s'il vaut `false` ou `0`.
fn shown(options: Option<&Value>, key: &str) -> bool {
    match options.and_then(|o| o.get(key)) {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        _ => true,
    }
}

/// Ni `null`, ni `false`, ni `0`, ni chaîne vide : ScoDoc marque ainsi une UE capitalisée.
fn present(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null | Value::Bool(false)) => false,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(_) => true,
    }
}

/// Entier positif envoyé comme nombre ou comme chaîne.
fn small_number(value: Option<&Value>) -> Option<u32> {
    let number = value.and_then(lossy_f64)?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // borné juste avant
    (number.fract() == 0.0 && (0.0..=10_000.0).contains(&number)).then_some(number as u32)
}

fn date(value: Option<&Value>) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value?.as_str()?.trim().get(..10)?, "%Y-%m-%d").ok()
}

/// Instant complet (`2026-09-11T08:00:00+02:00`), rendu dans le fuseau de l'université.
fn instant(value: Option<&Value>, tz: Tz) -> Option<DateTime<FixedOffset>> {
    let instant = DateTime::parse_from_rfc3339(value?.as_str()?.trim()).ok()?;
    Some(instant.with_timezone(&tz).fixed_offset())
}

/// `#` suivi de 3 à 8 chiffres hexadécimaux.
fn color(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    let digits = text.strip_prefix('#')?;
    ((3..=8).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| text.to_owned())
}

/// Texte d'un message écrit en HTML par l'établissement, une ligne par paragraphe.
fn message(value: Option<&Value>) -> Option<String> {
    let text = html_to_lines(value?.as_str()?);
    (!text.is_empty()).then_some(text)
}

fn evaluation_kind(value: Option<&Value>) -> EvaluationKind {
    match value.and_then(lossy_f64) {
        Some(n) if (n - 1.0).abs() < f64::EPSILON => EvaluationKind::Resit,
        Some(n) if (n - 2.0).abs() < f64::EPSILON => EvaluationKind::SecondSession,
        Some(n) if (n - 3.0).abs() < f64::EPSILON => EvaluationKind::Bonus,
        _ => EvaluationKind::Normal,
    }
}

/// `poids` : poids de l'évaluation dans chaque UE, par code d'UE.
fn weights(value: Option<&Value>) -> BTreeMap<String, f64> {
    entries(value, "poids")
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(unit, weight)| Some((unit.clone(), lossy_f64(weight)?)))
        .collect()
}

fn parse_evaluation(value: &Value, tz: Tz) -> Result<Evaluation, Problem> {
    if !value.is_object() {
        return Err(bad("évaluation : objet attendu"));
    }
    let id = id_string(value.get("id")).ok_or_else(|| bad("évaluation sans identifiant"))?;
    let (grade, status) = match cell(scalar(value.get("note"))) {
        Cell::Value(value) => (
            Some(Grade {
                value,
                out_of: OUT_OF,
            }),
            None,
        ),
        Cell::Status(status) => (None, Some(status)),
        Cell::Empty => (None, None),
    };
    // `date` est l'ancien nom de `date_debut`.
    let day = evaluation_date(value.get("date_debut"), tz)
        .or_else(|| evaluation_date(value.get("date"), tz));
    // Sans heure (`00:00`), l'épreuve n'a qu'un jour.
    let timed = |key: &str| instant(value.get(key), tz).filter(|at| at.time() != NaiveTime::MIN);
    Ok(Evaluation {
        name: text(value.get("description")).unwrap_or_else(|| format!("Évaluation {id}")),
        date: day,
        coefficient: value.get("coef").and_then(lossy_f64),
        id,
        grade,
        status,
        kind: evaluation_kind(value.get("evaluation_type")),
        weights: weights(value.get("poids")),
        stats: stats(value.get("note")),
        start: timed("date_debut"),
        end: timed("date_fin"),
    })
}

fn parse_module(code: &str, value: &Value, kind: &str, tz: Tz) -> Result<ModuleGrades, Problem> {
    if !value.is_object() {
        return Err(bad("ressource ou SAÉ : objet attendu"));
    }
    let evaluations = items(value.get("evaluations"), "evaluations")?
        .iter()
        .map(|e| parse_evaluation(e, tz))
        .collect::<Result<_, _>>()?;
    Ok(ModuleGrades {
        code: code.to_owned(),
        name: text(value.get("titre")).unwrap_or_else(|| code.to_owned()),
        average: average(value.get("moyenne")),
        evaluations,
        id: id_string(value.get("id")),
        kind: Some(kind.to_owned()),
    })
}

/// Points de bonus ou de malus ; `0` n'en est pas.
fn points(value: Option<&Value>) -> Option<f64> {
    value.and_then(lossy_f64).filter(|points| *points != 0.0)
}

/// Ce que le relevé dit en dehors de l'UE elle-même : réglages, capitalisations, jury.
struct UnitContext<'a> {
    show_ranks: bool,
    show_credits: bool,
    capitalized: Option<&'a Value>,
    /// Code de la décision du jury, par identifiant ScoDoc de l'UE et par acronyme.
    decisions: HashMap<String, String>,
}

/// `semestre.decision_ue` : `[{ue_id, acronyme, code}]`, une fois le jury passé.
fn unit_decisions(semester: Option<&Value>) -> HashMap<String, String> {
    let mut decisions = HashMap::new();
    let list = semester
        .and_then(|s| s.get("decision_ue"))
        .and_then(Value::as_array);
    for entry in list.into_iter().flatten() {
        let Some(code) = text(entry.get("code")).map(|code| code.to_uppercase()) else {
            continue;
        };
        if let Some(id) = id_string(entry.get("ue_id")) {
            decisions.insert(format!("id:{id}"), code.clone());
        }
        if let Some(acronym) = text(entry.get("acronyme")) {
            decisions.insert(format!("code:{acronym}"), code);
        }
    }
    decisions
}

fn parse_unit(code: &str, value: &Value, context: &UnitContext<'_>) -> Result<UnitGrade, Problem> {
    if !value.is_object() {
        return Err(bad("UE : objet attendu"));
    }
    let grade = value.get("moyenne");
    let credits = value.get("ECTS");
    let mut modules = Vec::new();
    for key in ["ressources", "saes"] {
        for (module, link) in sorted(entries(value.get(key), key)?) {
            modules.push(UnitModule {
                code: module.clone(),
                coefficient: link.get("coef").and_then(lossy_f64),
                average: average(link.get("moyenne")),
            });
        }
    }
    let decision = id_string(value.get("id"))
        .and_then(|id| context.decisions.get(&format!("id:{id}")))
        .or_else(|| context.decisions.get(&format!("code:{code}")))
        .cloned();
    Ok(UnitGrade {
        code: code.to_owned(),
        name: text(value.get("titre")).unwrap_or_else(|| code.to_owned()),
        average: average(grade),
        ects: credits.and_then(|e| e.get("total")).and_then(lossy_f64),
        ects_earned: credits
            .and_then(|e| e.get("acquis"))
            .and_then(lossy_f64)
            .filter(|_| context.show_credits),
        color: color(value.get("color")),
        stats: stats(grade),
        rank: grade
            .filter(|_| context.show_ranks)
            .and_then(|g| rank_of(g.get("rang"), g.get("total"))),
        bonus: points(value.get("bonus")),
        malus: points(value.get("malus")),
        capitalized: present(value.get("capitalise"))
            || present(context.capitalized.and_then(|all| all.get(code))),
        decision,
        modules,
    })
}

/// Rang (`12/56`) à partir de la place et de l'effectif. `-` ou vide quand l'IUT masque les rangs.
fn rank_of(position: Option<&Value>, total: Option<&Value>) -> Option<String> {
    let position = match position? {
        Value::String(s) if !s.trim().is_empty() && s.trim() != "-" => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    match total.and_then(lossy_f64) {
        Some(total) if total >= 1.0 => Some(format!("{position}/{total:.0}")),
        _ => Some(position),
    }
}

/// Rang du semestre (`{value, total}`).
pub(super) fn rank(value: Option<&Value>) -> Option<String> {
    let value = value?;
    rank_of(value.get("value"), value.get("total"))
}

/// Groupes de l'étudiant (`semestre.groupes`) : objets `{id, group_name, partition}` ou simples noms.
fn groups(semester: Option<&Value>) -> Vec<StudentGroup> {
    let list = semester
        .and_then(|s| s.get("groupes"))
        .and_then(Value::as_array);
    let mut groups: Vec<StudentGroup> = Vec::new();
    for entry in list.into_iter().flatten().take(50) {
        let name = text(entry.get("group_name"))
            .or_else(|| text(entry.get("nom")))
            .or_else(|| text(Some(entry)));
        let Some(name) = name else { continue };
        if groups.iter().any(|known| known.name == name) {
            continue;
        }
        groups.push(StudentGroup {
            id: id_string(entry.get("id")).or_else(|| id_string(entry.get("group_id"))),
            name,
            partition: entry
                .get("partition")
                .and_then(|p| text(p.get("partition_name"))),
        });
    }
    groups
}

/// Rang dans chaque groupe (`rang.groupes`) : par identifiant de groupe, `{value, total}` ou
/// `[value, total]`.
fn group_ranks(ranks: Option<&Value>, groups: &[StudentGroup]) -> Vec<GroupRank> {
    let mut out = Vec::new();
    for (key, entry) in sorted(entries(ranks, "groupes").unwrap_or_default()) {
        let rank = match entry {
            Value::Array(pair) => rank_of(pair.first(), pair.get(1)),
            other => rank_of(other.get("value"), other.get("total")),
        };
        let Some(rank) = rank else { continue };
        let named = text(entry.get("group_name"))
            .or_else(|| text(entry.get("nom")))
            .or_else(|| {
                groups
                    .iter()
                    .find(|group| group.id.as_deref() == Some(key.as_str()))
                    .map(|group| group.name.clone())
            });
        out.push(GroupRank {
            group: named.unwrap_or_else(|| key.clone()),
            rank,
        });
    }
    out
}

fn jury_decision(value: Option<&Value>) -> Option<JuryDecision> {
    let value = value?;
    Some(JuryDecision {
        code: text(value.get("code"))?.to_uppercase(),
        date: date(value.get("date")),
    })
}

/// Département du relevé : `etudiant.dept_acronym`, sinon celui de l'adresse de la photo
/// (`/ScoDoc/<département>/…`). Rien d'autre n'est lu dans `etudiant`.
fn department(report: &Value) -> Option<String> {
    let student = report.get("etudiant")?;
    text(student.get("dept_acronym")).or_else(|| {
        let url = student.get("photo_url")?.as_str()?;
        let department = url.split('/').nth(2)?.trim();
        let plain = department
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        (!department.is_empty() && plain).then(|| department.to_owned())
    })
}

/// Liste de départements d'un réglage de la passerelle (`"MMI, INFO"`). Vide : tous.
fn department_listed(list: Option<&Value>, department: Option<&str>) -> bool {
    let raw = list.and_then(Value::as_str).unwrap_or_default();
    let mut names = raw
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let mut any = false;
    let listed = names.by_ref().any(|name| {
        any = true;
        department == Some(name)
    });
    listed || !any
}

/// Semestres de l'étudiant (`semestres`), dans l'ordre de la passerelle : du plus ancien au plus
/// récent.
fn available(listed: &[Value]) -> Vec<SemesterRef> {
    listed
        .iter()
        .filter_map(|semester| {
            let id = id_string(semester.get("formsemestre_id"))?;
            let number = small_number(semester.get("semestre_id"));
            let name = text(semester.get("titre"))
                .or_else(|| number.map(|n| format!("Semestre {n}")))
                .unwrap_or_else(|| format!("Semestre {id}"));
            Some(SemesterRef {
                id,
                name,
                number,
                year: text(semester.get("annee_scolaire")),
            })
        })
        .collect()
}

/// Relevé du semestre courant. Le bulletin n'est lu que dans la clé `relevé` (avec accent) ;
/// les données personnelles de `relevé.etudiant` ne sont jamais copiées.
pub(super) fn parse_grades(data: &Value, tz: Tz) -> Result<GradeReport, Problem> {
    parse_report(data, tz, None)
}

/// Relevé d'une réponse de `dataPremièreConnexion` ou de `relevéEtudiant`. `requested` est le
/// semestre demandé à `relevéEtudiant` : il nomme le relevé quand celui-ci ne dit pas son semestre
/// (relevé non publié).
pub(super) fn parse_report(
    data: &Value,
    tz: Tz,
    requested: Option<&str>,
) -> Result<GradeReport, Problem> {
    let Some(report @ Value::Object(_)) = data.get("relevé") else {
        return Err(bad("clé `relevé` absente ou illisible"));
    };
    let listed = items(data.get("semestres"), "semestres")?;
    let available = available(listed);
    // Le semestre courant est le dernier de la liste triée par la passerelle [lu].
    let current = id_string(report.get("formsemestre_id"))
        .or_else(|| requested.map(str::to_owned))
        .or_else(|| available.last().map(|semester| semester.id.clone()));
    let Some(id) = current else {
        // Aucun semestre : la passerelle renvoie un relevé vide.
        return Ok(GradeReport::default());
    };

    let semester = report.get("semestre");
    let known = available.iter().find(|semester| semester.id == id);
    let number = semester
        .and_then(|s| small_number(s.get("numero")))
        .or_else(|| known.and_then(|k| k.number));
    let name = known
        .map(|k| k.name.clone())
        .or_else(|| number.map(|n| format!("Semestre {n}")))
        .unwrap_or_else(|| format!("Semestre {id}"));

    let options = report.get("options");
    let show_ranks = shown(options, "show_rangs");
    let show_credits = shown(options, "show_ects");
    let context = UnitContext {
        show_ranks: shown(options, "show_ue_rangs"),
        show_credits,
        capitalized: report.get("ues_capitalisees"),
        decisions: unit_decisions(semester),
    };
    let units = sorted(entries(report.get("ues"), "ues")?)
        .into_iter()
        .map(|(code, value)| parse_unit(code, value, &context))
        .collect::<Result<_, _>>()?;
    let mut modules = Vec::new();
    for (key, kind) in [("ressources", "ressource"), ("saes", "sae")] {
        for (code, value) in sorted(entries(report.get(key), key)?) {
            modules.push(parse_module(code, value, kind, tz)?);
        }
    }

    let groups = groups(semester);
    let ranks = semester.and_then(|s| s.get("rang"));
    let credits = semester
        .and_then(|s| s.get("ECTS"))
        .filter(|_| show_credits);
    let department = department(report);
    let published = report.get("publie").and_then(Value::as_bool);
    // Les réglages de la passerelle ne viennent qu'avec `dataPremièreConnexion`.
    let config = data.get("config").filter(|config| config.is_object());
    let transcript = config.map(|config| {
        config.get("releve_PDF").is_some_and(|pdf| flag(Some(pdf)))
            && department_listed(config.get("liste_dep_publi_PDF"), department.as_deref())
    });
    // La passerelle ne donne une répartition que pour un relevé publié.
    let distributions = config.map(|config| {
        config.get("histogramme") != Some(&Value::Bool(false)) && published == Some(true)
    });

    Ok(GradeReport {
        semesters: vec![SemesterGrades {
            id,
            name,
            average: average(semester.and_then(|s| s.get("notes"))),
            rank: ranks.filter(|_| show_ranks).and_then(|r| rank(Some(r))),
            units,
            modules,
            number,
            year: semester
                .and_then(|s| text(s.get("annee_universitaire")))
                .or_else(|| known.and_then(|k| k.year.clone())),
            start: semester.and_then(|s| date(s.get("date_debut"))),
            end: semester.and_then(|s| date(s.get("date_fin"))),
            kind: text(report.get("type")),
            program: report
                .get("formation")
                .and_then(|f| text(f.get("acronyme"))),
            department,
            published,
            notice: message(report.get("message")),
            updated_at: instant(report.get("date"), tz),
            stats: stats(semester.and_then(|s| s.get("notes"))),
            group_ranks: if show_ranks {
                group_ranks(ranks.and_then(|r| r.get("groupes")), &groups)
            } else {
                Vec::new()
            },
            groups,
            ects: credits.and_then(|e| e.get("total")).and_then(lossy_f64),
            ects_earned: credits.and_then(|e| e.get("acquis")).and_then(lossy_f64),
            decision: jury_decision(semester.and_then(|s| s.get("decision"))),
            year_decision: jury_decision(semester.and_then(|s| s.get("decision_annee"))),
            transcript,
            distributions,
        }],
        available,
    })
}

/// Le relevé contient cette évaluation. `listeNotes` répond pour n'importe quelle évaluation de
/// l'établissement : seules celles du relevé de l'étudiant sont demandées.
pub(super) fn owns_evaluation(data: &Value, evaluation: &str) -> bool {
    let Some(report) = data.get("relevé") else {
        return false;
    };
    ["ressources", "saes"].iter().any(|key| {
        entries(report.get(key), key)
            .unwrap_or_default()
            .into_iter()
            .flat_map(|(_, module)| items(module.get("evaluations"), "evaluations").unwrap_or(&[]))
            .any(|entry| id_string(entry.get("id")).as_deref() == Some(evaluation))
    })
}

/// `relevé.publie` vaut `true`. Absent (département masqué) ou `false` : la passerelle n'affiche
/// pas de répartition.
pub(super) fn is_published(data: &Value) -> bool {
    data.get("relevé").and_then(|report| report.get("publie")) == Some(&Value::Bool(true))
}

/// Notes au plus dans une réponse de `listeNotes`.
const MAX_DISTRIBUTION: usize = 5000;
/// En dessous, une répartition désignerait des personnes : la passerelle répond `["too low"]`.
const MIN_DISTRIBUTION: u32 = 6;

fn no_distribution(reason: DistributionUnavailable) -> GradeDistribution {
    GradeDistribution {
        counts: Vec::new(),
        total: 0,
        out_of: OUT_OF,
        unavailable: Some(reason),
    }
}

/// Relevé non publié : pas de répartition.
pub(super) fn hidden_distribution() -> GradeDistribution {
    no_distribution(DistributionUnavailable::Hidden)
}

/// Réponse de `listeNotes` : un tableau de notes déjà ramenées sur 20, `["too low"]`, ou
/// `{"erreur": "L'histogramme n'est pas activé"}`. Les notes sont rangées par point entier, 20
/// dans la dernière case.
pub(super) fn parse_distribution(value: &Value) -> Result<GradeDistribution, Problem> {
    let grades = match value {
        Value::Array(grades) => grades,
        Value::Object(map) => {
            let refused = map
                .get("erreur")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return if refused.to_lowercase().contains("histogramme") {
                Ok(no_distribution(DistributionUnavailable::Disabled))
            } else {
                Err(bad("répartition : réponse inattendue"))
            };
        }
        _ => return Err(bad("répartition : tableau attendu")),
    };
    if grades.first().and_then(Value::as_str) == Some("too low") {
        return Ok(no_distribution(DistributionUnavailable::TooFew));
    }
    if grades.len() > MAX_DISTRIBUTION {
        return Err(bad("répartition : trop de notes"));
    }
    let mut counts = vec![0_u32; 21];
    let mut total = 0_u32;
    for grade in grades {
        let grade = grade
            .as_f64()
            .filter(|grade| grade.is_finite())
            .ok_or_else(|| bad("répartition : note illisible"))?;
        // 12,999999 d'un calcul flottant est un 13.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // borné entre 0 et 20
        let bucket = (grade + 1e-9).floor().clamp(0.0, OUT_OF) as usize;
        if let Some(count) = counts.get_mut(bucket) {
            *count += 1;
            total += 1;
        }
    }
    if total < MIN_DISTRIBUTION {
        return Ok(no_distribution(DistributionUnavailable::TooFew));
    }
    Ok(GradeDistribution {
        counts,
        total,
        out_of: OUT_OF,
        unavailable: None,
    })
}

/// Réponse de `getReportPageMessage` : `{"message": "<HTML>"}`, rendue en texte.
pub(super) fn parse_notice(value: &Value) -> GradeNotice {
    GradeNotice {
        message: message(value.get("message")),
    }
}

/// Identifiant de l'étudiant pour la passerelle (`auth.session`), tel que `bulletin_PDF.php`
/// l'attend. Il vient de la session ouverte, jamais de la requête du client.
pub(super) fn student_id(data: &Value) -> Option<String> {
    let id = data.get("auth")?.get("session")?.as_str()?.trim();
    let plain = id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '-'));
    ((1..=64).contains(&id.len()) && plain).then(|| id.to_owned())
}

/// Type de bulletin à demander en PDF : `DUT` si le relevé le dit, `BUT` sinon.
pub(super) fn transcript_type(data: &Value) -> &'static str {
    let kind = data.get("relevé").and_then(|report| report.get("type"));
    if kind.and_then(Value::as_str) == Some("DUT") {
        "DUT"
    } else {
        "BUT"
    }
}

/// Instant local à partir d'un jour et d'une heure décimale (`15.5` = 15 h 30). Une heure qui
/// n'existe pas (saut du changement d'heure) avance d'une heure ; une heure qui existe deux fois
/// prend la première.
pub(super) fn at(day: NaiveDate, hours: f64, tz: Tz) -> Result<DateTime<FixedOffset>, Problem> {
    if !(0.0..=24.0).contains(&hours) {
        return Err(bad("horaire d'absence hors d'une journée"));
    }
    #[allow(clippy::cast_possible_truncation)] // 0 ≤ hours ≤ 24 : au plus 1 440 minutes
    let minutes = (hours * 60.0).round() as i64;
    let local = day
        .and_time(NaiveTime::MIN)
        .checked_add_signed(chrono::Duration::minutes(minutes))
        .ok_or_else(|| bad("date d'absence hors limites"))?;
    resolve_local(tz, local).ok_or_else(|| bad("horaire d'absence illisible"))
}

fn flag(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|v| v != 0.0),
        Some(Value::String(s)) => matches!(s.trim().to_lowercase().as_str(), "true" | "1" | "oui"),
        _ => false,
    }
}

/// Titre des ressources et SAÉ par identifiant ScoDoc (`matiereComplet` est un `moduleimpl_id`).
fn module_titles(data: &Value) -> HashMap<String, String> {
    let mut titles = HashMap::new();
    let Some(report) = data.get("relevé") else {
        return titles;
    };
    for key in ["ressources", "saes"] {
        for (_, value) in entries(report.get(key), key).unwrap_or_default() {
            if let (Some(id), Some(title)) = (id_string(value.get("id")), text(value.get("titre")))
            {
                titles.insert(id, title);
            }
        }
    }
    titles
}

/// Absences du semestre courant. Seules les entrées `absent` comptent : les retards et les présences
/// du même détail sont ignorés. Les totaux sont recomptés sur ces entrées, car `totauxAbsences` ne
/// contient que la métrique choisie par l'IUT.
pub(super) fn parse_absences(data: &Value, tz: Tz) -> Result<AbsenceReport, Problem> {
    if !matches!(
        data.get("absences"),
        Some(Value::Object(_) | Value::Array(_))
    ) {
        return Err(bad("clé `absences` absente ou illisible"));
    }
    let titles = module_titles(data);
    let mut absences = Vec::new();
    for (day_key, list) in entries(data.get("absences"), "absences")? {
        let day = NaiveDate::parse_from_str(day_key, "%Y-%m-%d")
            .map_err(|_| bad("absences : jour illisible"))?;
        for entry in items(Some(list), "absences[jour]")? {
            let status = entry
                .get("statut")
                .and_then(Value::as_str)
                .ok_or_else(|| bad("absence sans statut"))?;
            if !status.trim().eq_ignore_ascii_case("absent") {
                continue;
            }
            let from = entry
                .get("debut")
                .and_then(lossy_f64)
                .ok_or_else(|| bad("absence sans heure de début"))?;
            let to = entry
                .get("fin")
                .and_then(lossy_f64)
                .ok_or_else(|| bad("absence sans heure de fin"))?;
            let last_day = text(entry.get("dateFin"))
                .and_then(|d| NaiveDate::parse_from_str(d.get(..10)?, "%Y-%m-%d").ok())
                .unwrap_or(day);
            absences.push(Absence {
                // Sans `data_absences_scodoc`, la passerelle n'envoie pas d'identifiant.
                id: id_string(entry.get("idAbs"))
                    .unwrap_or_else(|| format!("{day_key}-{from}-{to}")),
                start: at(day, from, tz)?,
                end: at(last_day, to, tz)?,
                justified: flag(entry.get("justifie")),
                module: id_string(entry.get("matiereComplet"))
                    .and_then(|id| titles.get(&id).cloned()),
            });
        }
    }
    absences.sort_by_key(|a| a.start);
    let count = |justified: bool| {
        let n = absences.iter().filter(|a| a.justified == justified).count();
        u32::try_from(n).unwrap_or(u32::MAX)
    };
    let totals = AbsenceTotals {
        justified: count(true),
        unjustified: count(false),
    };
    Ok(AbsenceReport { absences, totals })
}
