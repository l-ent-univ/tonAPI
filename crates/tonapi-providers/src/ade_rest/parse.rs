//! Analyse pure des réponses d'ADE (API REST du portail), sans réseau : jetons, sélection de
//! ressources, arbre de groupes et événements.

use std::collections::HashSet;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use chrono_tz::Tz;
use serde_json::Value;
use tonapi_core::{Course, Problem, TimetableGroup};

use super::{PROVIDER, Tokens};
use crate::common::{id_string, resolve_local, str_field};
use crate::net::upstream;

/// Profondeur maximale d'un arbre de ressources lu depuis le service.
pub(super) const MAX_DEPTH: usize = 32;

/// Les réponses sont enveloppées dans `{"data": …}` ; retire un niveau s'il y en a un.
fn unwrap_data(value: &Value) -> &Value {
    value.get("data").unwrap_or(value)
}

/// Jeton SSO de `sso/accessToken` (`token` ou `data.token`). Vide quand ADE n'a pas de session :
/// la réponse est alors un HTTP 200 avec `errMsg`.
pub(super) fn parse_sso_token(value: &Value) -> Option<String> {
    str_field(value, "token")
        .or_else(|| value.get("data").and_then(|d| str_field(d, "token")))
        .map(str::to_owned)
}

/// Réponse de `ssologin`. `authData` est lu tel que `data.authData.authData` ou `data.authData` ;
/// le projet est `authData.projectId` (le `data.projectId` observé vaut 0).
pub(super) fn parse_ssologin(value: &Value) -> Result<Tokens, Problem> {
    let data = unwrap_data(value);
    let auth = data
        .get("authData")
        .map(|a| a.get("authData").unwrap_or(a))
        .filter(|a| a.is_object())
        .ok_or_else(|| upstream(PROVIDER, "ssologin sans `authData`"))?;
    let project = auth
        .get("projectId")
        .and_then(Value::as_i64)
        .or_else(|| {
            data.get("projectId")
                .and_then(Value::as_i64)
                .filter(|p| *p != 0)
        })
        .ok_or_else(|| upstream(PROVIDER, "ssologin sans numéro de projet"))?;
    let access = str_field(auth, "accessToken")
        .ok_or_else(|| upstream(PROVIDER, "ssologin sans jeton d'API"))?;
    Ok(Tokens {
        access: access.to_owned(),
        refresh: str_field(auth, "refreshToken").map(str::to_owned),
        project,
    })
}

/// Réponse de `refreshtoken` : nouveau jeton d'API et, si présent, nouveau jeton de
/// rafraîchissement.
pub(super) fn parse_refresh(value: &Value) -> Option<(String, Option<String>)> {
    let data = unwrap_data(value);
    let access = str_field(data, "accessToken")?.to_owned();
    Some((access, str_field(data, "refreshToken").map(str::to_owned)))
}

/// Ids de ressources dans une valeur JSON : nombres, chaînes numériques, ou `id` d'un objet.
fn collect_ids(value: &Value, out: &mut Vec<i64>, depth: usize) {
    if depth > 8 {
        return;
    }
    match value {
        Value::Number(n) => out.extend(n.as_i64()),
        Value::String(s) => out.extend(s.trim().parse::<i64>().ok()),
        Value::Array(items) => items.iter().for_each(|v| collect_ids(v, out, depth + 1)),
        Value::Object(map) => match map.get("id") {
            Some(id) => collect_ids(id, out, depth + 1),
            None => map.values().for_each(|v| collect_ids(v, out, depth + 1)),
        },
        _ => {}
    }
}

/// Ressources choisies par l'étudiant (`PortalResourcesAllSelection`). `{"data":""}` : aucune.
///
/// [inconnu] Le format de la chaîne n'a jamais été capturé. On accepte du JSON (nombres, tableaux,
/// objets avec `id`) ou des ids séparés par `;`, `,`, `|` ou des espaces. Une sélection non vide
/// dont on ne tire aucun id donne `upstream_changed` plutôt qu'un emploi du temps vide.
pub(super) fn parse_selection(value: &Value) -> Result<Vec<i64>, Problem> {
    let data = unwrap_data(value);
    let mut ids = Vec::new();
    match data {
        Value::Null => return Ok(ids),
        Value::String(s) if s.trim().is_empty() => return Ok(ids),
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(inner) => collect_ids(&inner, &mut ids, 0),
            Err(_) => ids.extend(
                s.split(|c: char| matches!(c, ';' | ',' | '|') || c.is_whitespace())
                    .filter_map(|t| t.parse::<i64>().ok()),
            ),
        },
        other => collect_ids(other, &mut ids, 0),
    }
    // Les ids négatifs sont ceux, synthétiques, des catégories de l'arbre.
    let mut seen = HashSet::new();
    ids.retain(|id| *id > 0 && seen.insert(*id));
    if ids.is_empty() {
        return Err(upstream(
            PROVIDER,
            "sélection de ressources dans un format inconnu",
        ));
    }
    Ok(ids)
}

/// Nœuds enfants : `branch`, `leaf` et `children` mis bout à bout.
fn child_nodes(node: &Value) -> Vec<&Value> {
    ["branch", "leaf", "children"]
        .iter()
        .filter_map(|key| node.get(key).and_then(Value::as_array))
        .flatten()
        .collect()
}

/// Groupes listés au plus.
const MAX_GROUPS: usize = 5_000;

fn walk_tree(
    node: &Value,
    path: &mut Vec<String>,
    selected: &HashSet<String>,
    out: &mut Vec<TimetableGroup>,
    depth: usize,
) -> Result<(), Problem> {
    if depth > MAX_DEPTH {
        return Err(upstream(PROVIDER, "arbre des ressources trop profond"));
    }
    // Chaque groupe copie son chemin : un arbre démesuré multiplierait la mémoire.
    if out.len() >= MAX_GROUPS {
        return Err(upstream(PROVIDER, "arbre des ressources trop grand"));
    }
    let id =
        id_string(node.get("id")).ok_or_else(|| upstream(PROVIDER, "nœud de l'arbre sans `id`"))?;
    let name =
        str_field(node, "name").ok_or_else(|| upstream(PROVIDER, "nœud de l'arbre sans `name`"))?;
    path.push(name.to_owned());
    let children = child_nodes(node);
    let is_selected = selected.contains(&id);
    // Une feuille est un groupe sélectionnable ; une branche n'apparaît que si l'étudiant l'a
    // choisie (le comportement d'une ressource parente n'est pas confirmé).
    if children.is_empty() || is_selected {
        out.push(TimetableGroup {
            id,
            name: name.to_owned(),
            path: path.clone(),
            selected: is_selected,
        });
    }
    for child in children {
        walk_tree(child, path, selected, out, depth + 1)?;
    }
    path.pop();
    Ok(())
}

/// Catégories d'étudiants dans `resourcestree` ; les enseignants et les salles sont ignorés.
fn is_student_category(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "trainee" | "students")
}

/// Groupes d'étudiants de l'arbre `resourcestree`, avec leur chemin. `selected` marque ceux que
/// l'étudiant a choisis.
pub(super) fn parse_groups(tree: &Value, selected: &[i64]) -> Result<Vec<TimetableGroup>, Problem> {
    let data = unwrap_data(tree);
    let categories = data.get("category").unwrap_or(data);
    let items: Vec<&Value> = match categories {
        Value::Array(items) => items.iter().collect(),
        Value::Object(_) => vec![categories],
        _ => return Err(upstream(PROVIDER, "arbre des ressources illisible")),
    };
    let selected: HashSet<String> = selected.iter().map(i64::to_string).collect();
    let mut groups = Vec::new();
    for item in items {
        match str_field(item, "category") {
            Some(category) if !is_student_category(category) => {}
            Some(_) => {
                for child in child_nodes(item) {
                    walk_tree(child, &mut Vec::new(), &selected, &mut groups, 0)?;
                }
            }
            // Sans catégorie : un nœud d'arbre, qu'on ne peut pas trier.
            None => walk_tree(item, &mut Vec::new(), &selected, &mut groups, 0)?,
        }
    }
    Ok(groups)
}

/// `±HHMM`, `±HH:MM`, `±HH` ou `Z`.
pub(super) fn parse_offset(raw: &str) -> Option<FixedOffset> {
    if raw.eq_ignore_ascii_case("z") {
        return FixedOffset::east_opt(0);
    }
    let sign = match raw.chars().next()? {
        '+' => 1,
        '-' => -1,
        _ => return None,
    };
    let digits: String = raw[1..].chars().filter(|c| *c != ':').collect();
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (hours, minutes) = match digits.len() {
        2 => (digits.parse::<i32>().ok()?, 0),
        4 => (
            digits.get(..2)?.parse::<i32>().ok()?,
            digits.get(2..)?.parse::<i32>().ok()?,
        ),
        _ => return None,
    };
    if minutes >= 60 {
        return None;
    }
    FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60))
}

/// `HH:MM[:SS]` suivi d'un décalage facultatif (`+0200`, `+02:00`, `Z`).
pub(super) fn parse_clock(raw: &str) -> Option<(NaiveTime, Option<FixedOffset>)> {
    let raw = raw.trim();
    let cut = raw.find(['Z', 'z', '+', '-']).unwrap_or(raw.len());
    let (time, offset) = raw.split_at(cut);
    let time = NaiveTime::parse_from_str(time, "%H:%M:%S")
        .or_else(|_| NaiveTime::parse_from_str(time, "%H:%M"))
        .ok()?;
    let offset = if offset.is_empty() {
        None
    } else {
        Some(parse_offset(offset)?)
    };
    Some((time, offset))
}

/// Date + heure ADE → instant au fuseau de l'université. Avec décalage, il est respecté puis
/// converti ; sans décalage, l'heure est celle du fuseau de l'université (jamais celui du serveur).
fn instant(date: NaiveDate, clock: &str, tz: Tz) -> Result<DateTime<FixedOffset>, Problem> {
    let (time, offset) =
        parse_clock(clock).ok_or_else(|| upstream(PROVIDER, "heure d'événement illisible"))?;
    let naive: NaiveDateTime = date.and_time(time);
    let local = match offset {
        Some(offset) => offset
            .from_local_datetime(&naive)
            .single()
            .map(|d| d.with_timezone(&tz).fixed_offset()),
        // Heure répétée : la première occurrence ; heure sautée : une heure plus tard, comme ADE
        // iCal.
        None => resolve_local(tz, naive),
    };
    local.ok_or_else(|| {
        upstream(
            PROVIDER,
            "heure d'événement inexistante (changement d'heure)",
        )
    })
}

/// Noms des participants d'une catégorie, sans doublon. Catégories (minuscules) lues par tafac :
/// enseignant `instructor`/`category2`, salle `classroom`/`category3`, groupe
/// `trainee`/`category1`/`category8`. [inconnu] : la liste des `categoryN` de Rennes.
fn participants(event: &Value, categories: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let list = event
        .get("eventParticipants")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for participant in list {
        let Some(category) = str_field(participant, "category") else {
            continue;
        };
        if !categories.contains(&category.to_ascii_lowercase().as_str()) {
            continue;
        }
        if names.len() >= 50 {
            break;
        }
        if let Some(name) = str_field(participant, "name")
            && !names.iter().any(|n| n == name)
        {
            names.push(name.to_owned());
        }
    }
    names
}

fn parse_event(event: &Value, tz: Tz) -> Result<Course, Problem> {
    let id = id_string(event.get("id"))
        .or_else(|| id_string(event.get("activityId")))
        .ok_or_else(|| upstream(PROVIDER, "événement sans `id`"))?;
    let date = str_field(event, "date")
        .ok_or_else(|| upstream(PROVIDER, "événement sans `date`"))
        .and_then(|d| {
            NaiveDate::parse_from_str(d, "%d/%m/%Y")
                .map_err(|_| upstream(PROVIDER, "date d'événement illisible"))
        })?;
    let start_raw = str_field(event, "startHour")
        .ok_or_else(|| upstream(PROVIDER, "événement sans `startHour`"))?;
    let start = instant(date, start_raw, tz)?;
    // Fin absente : séance sans durée connue ; une fin avant le début est ramenée au début.
    let end = match str_field(event, "endHour") {
        Some(raw) => instant(date, raw, tz)?.max(start),
        None => start,
    };
    let title = str_field(event, "name")
        .or_else(|| {
            let fields = event.get("fields")?;
            str_field(fields, "Nom").or_else(|| str_field(fields, "NAME"))
        })
        .unwrap_or("Cours")
        .to_owned();
    Ok(Course {
        id,
        title,
        // [inconnu] aucun champ de type de séance (CM, TD, TP) n'est documenté.
        kind: None,
        start,
        end,
        rooms: participants(event, &["classroom", "category3"]),
        teachers: participants(event, &["instructor", "category2"]),
        groups: participants(event, &["trainee", "category1", "category8"]),
        description: None,
    })
}

/// Événements d'une réponse : blocs `{id, events:[…]}`, éventuellement sous `data`, `resources` ou
/// `eventsByResource`, ou événements à plat.
fn collect_events<'a>(value: &'a Value, out: &mut Vec<&'a Value>, depth: usize) {
    if depth > 8 {
        return;
    }
    match value {
        Value::Array(items) => items.iter().for_each(|v| collect_events(v, out, depth + 1)),
        Value::Object(map) => {
            if let Some(events) = map.get("events").and_then(Value::as_array) {
                out.extend(events);
            } else if map.contains_key("startHour") || map.contains_key("date") {
                out.push(value);
            }
            for key in ["data", "resources", "eventsByResource"] {
                if let Some(inner) = map.get(key) {
                    collect_events(inner, out, depth + 1);
                }
            }
        }
        _ => {}
    }
}

/// Trie par début puis identifiant, et ne garde qu'une fois un événement présent sous plusieurs
/// ressources (groupe, enseignant, salle).
pub(super) fn normalize(mut courses: Vec<Course>) -> Vec<Course> {
    courses.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.id.cmp(&b.id)));
    let mut seen = HashSet::new();
    courses.retain(|c| seen.insert(c.id.clone()));
    courses
}

/// Cours de `from` à `to` (inclus, jours du fuseau de l'université) d'une réponse
/// `resourcesEventsPortal`.
pub(super) fn parse_events(
    response: &Value,
    tz: Tz,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Course>, Problem> {
    let mut raw = Vec::new();
    collect_events(response, &mut raw, 0);
    let mut courses = Vec::with_capacity(raw.len());
    for event in raw {
        let course = parse_event(event, tz)?;
        let day = course.start.date_naive();
        if (from..=to).contains(&day) {
            courses.push(course);
        }
    }
    Ok(normalize(courses))
}

/// Filtre d'événements d'une ressource. Les dates sont en `MMJJAAAA` (mois d'abord), la fin est
/// incluse.
pub(super) fn events_filter(resource: i64, from: NaiveDate, to: NaiveDate) -> String {
    format!(
        "resources;{resource}/startdate;{}/enddate;{}/level;=8/field;Nom;CODEZ_RESOURCE;OWNER;",
        from.format("%m%d%Y"),
        to.format("%m%d%Y"),
    )
}
