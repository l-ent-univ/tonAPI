//! Réponse de `api/events` de TEMPO, sans réseau. Repris de l'analyse de Tafac (`tempoApi.js`),
//! dont les tests portent la forme des événements ; aucune réponse réelle n'a été lue par tonAPI.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime};
use chrono_tz::Tz;
use serde_json::Value;
use tonapi_core::{Course, Problem};

use super::PROVIDER;
use crate::common::{html_to_text, id_string, resolve_local};
use crate::net::upstream;

/// Titre d'une séance que TEMPO envoie sans titre.
const UNTITLED: &str = "Cours";

/// Séances de la réponse qui chevauchent `from..=to` (jours locaux), sans doublon, triées par
/// début.
///
/// La réponse est un tableau JSON non trié. Chaque événement porte `id`, `start` et `end`
/// (`AAAA-MM-JJ HH:mm` à l'heure de l'université, ou une date seule pour une journée entière),
/// `title`, `description` (le groupe), `location` (une salle par ligne), `people`, `color`
/// (hexadécimal sans `#`) et `info`. Les textes sont du HTML. Un événement sans identifiant ou
/// sans début lisible est écarté ; si aucun ne se lit, le format a changé.
pub(crate) fn parse_events(
    payload: &Value,
    tz: Tz,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Course>, Problem> {
    let events = payload
        .as_array()
        .ok_or_else(|| upstream(PROVIDER, "la réponse n'est pas une liste d'événements"))?;
    let mut courses: Vec<Course> = events.iter().filter_map(|e| course(e, tz)).collect();
    if courses.is_empty() && !events.is_empty() {
        return Err(upstream(
            PROVIDER,
            "aucun événement lisible dans la réponse",
        ));
    }
    let window = midnight(tz, from).zip(to.succ_opt().and_then(|next| midnight(tz, next)));
    let Some((start, end)) = window.filter(|_| from <= to) else {
        return Ok(Vec::new());
    };
    courses.retain(|course| course.end > start && course.start < end);
    // Tri stable : de deux événements de même identifiant, le premier reçu reste.
    courses.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.end.cmp(&b.end)));
    let mut seen = std::collections::HashSet::new();
    courses.retain(|course| seen.insert(course.id.clone()));
    Ok(courses)
}

fn midnight(tz: Tz, day: NaiveDate) -> Option<DateTime<FixedOffset>> {
    resolve_local(tz, day.and_time(NaiveTime::MIN))
}

/// Un événement de TEMPO ; `None` s'il n'a pas d'identifiant ou pas de début lisible.
fn course(event: &Value, tz: Tz) -> Option<Course> {
    let id = id_string(event.get("id"))?;
    let (start_day, start_time) = clock(event.get("start")?.as_str()?)?;
    let start = resolve_local(tz, start_day.and_time(start_time.unwrap_or(NaiveTime::MIN)))?;
    let end_clock = event.get("end").and_then(Value::as_str).and_then(clock);
    let end = if start_time.is_some() {
        // Une fin illisible : la séance s'arrête où elle commence.
        end_clock
            .and_then(|(day, time)| resolve_local(tz, day.and_time(time.unwrap_or(NaiveTime::MIN))))
            .unwrap_or(start)
    } else {
        // Une journée entière va de minuit à minuit, son dernier jour compris.
        let last = end_clock
            .filter(|(_, time)| time.is_none())
            .map_or(start_day, |(day, _)| day);
        midnight(tz, last.succ_opt()?)?
    };

    let field = |name: &str| {
        event
            .get(name)
            .and_then(Value::as_str)
            .map(html_to_text)
            .filter(|text| !text.is_empty())
    };
    let teachers = event
        .get("people")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(html_to_text)
        .filter(|name| !name.is_empty())
        .collect();
    let rooms = event
        .get("location")
        .and_then(Value::as_str)
        .map(lines)
        .unwrap_or_default();
    let color = field("color").filter(|c| c.len() == 6 && c.bytes().all(|b| b.is_ascii_hexdigit()));
    Some(Course {
        id,
        title: field("title").unwrap_or_else(|| UNTITLED.to_owned()),
        kind: None,
        start,
        end,
        rooms,
        teachers,
        groups: field("description").into_iter().collect(),
        description: field("info"),
        color,
        activity_id: None,
    })
}

/// `AAAA-MM-JJ HH:mm` (ou `T` entre les deux, des secondes ignorées) : le jour et l'heure ; une
/// date seule : le jour, sans heure.
fn clock(written: &str) -> Option<(NaiveDate, Option<NaiveTime>)> {
    let written = written.trim();
    let day = NaiveDate::parse_from_str(written.get(..10)?, "%Y-%m-%d").ok()?;
    let rest = written.get(10..)?;
    if rest.is_empty() {
        return Some((day, None));
    }
    let time = rest
        .strip_prefix([' ', 'T'])
        .and_then(|time| time.get(..5))
        .and_then(|time| NaiveTime::parse_from_str(time, "%H:%M").ok())?;
    Some((day, Some(time)))
}

/// Lignes d'un champ HTML : `<br>` et les retours à la ligne séparent, le reste est du texte.
fn lines(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut broken = String::with_capacity(html.len());
    let mut from = 0;
    while let Some(at) = lower.get(from..).and_then(|rest| rest.find("<br")) {
        let start = from + at;
        let Some(close) = lower.get(start..).and_then(|rest| rest.find('>')) else {
            break;
        };
        broken.push_str(html.get(from..start).unwrap_or_default());
        broken.push('\n');
        from = start + close + 1;
    }
    broken.push_str(html.get(from..).unwrap_or_default());
    broken
        .split(['\n', '\r'])
        .map(html_to_text)
        .filter(|line| !line.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Europe::Paris;
    use serde_json::json;
    use tonapi_core::ErrorCode;

    const EVENTS: &str = include_str!("../../tests/fixtures/tempo/evenements.json");

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn parse(payload: &Value, from: NaiveDate, to: NaiveDate) -> Vec<Course> {
        parse_events(payload, Paris, from, to).unwrap()
    }

    fn fixture(from: NaiveDate, to: NaiveDate) -> Vec<Course> {
        parse(&serde_json::from_str(EVENTS).unwrap(), from, to)
    }

    fn one(event: &Value) -> Course {
        let courses = parse(&json!([event]), day(2020, 1, 1), day(2030, 1, 1));
        assert_eq!(courses.len(), 1);
        courses.into_iter().next().unwrap()
    }

    #[test]
    fn une_semaine_triee_sans_doublon() {
        let courses = fixture(day(2026, 11, 2), day(2026, 11, 8));
        let ids: Vec<&str> = courses.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["9909", "9910"]);

        let first = courses.first().unwrap();
        assert_eq!(first.title, "UEF 2 - Histoire des médias - CM1");
        assert_eq!(first.start.to_rfc3339(), "2026-11-02T10:45:00+01:00");
        assert_eq!(first.end.to_rfc3339(), "2026-11-02T12:45:00+01:00");
        assert_eq!(first.rooms, ["B204 Bâtiment B", "Campus de test"]);
        assert_eq!(first.teachers, ["Martin C."]);
        assert_eq!(first.groups, ["L2 INFO-COM CM1 - G 1 2 3"]);
        assert_eq!(first.description, None);
        assert_eq!(first.color.as_deref(), Some("62a5ff"));
        assert_eq!(
            (first.kind.as_deref(), first.activity_id.as_deref()),
            (None, None)
        );
    }

    #[test]
    fn textes_html_lus_comme_du_texte() {
        let courses = fixture(day(2026, 11, 4), day(2026, 11, 4));
        let course = courses.first().unwrap();
        assert_eq!(course.title, "Théâtre & danse CC");
        assert_eq!(course.rooms, ["Salle A", "Salle B '2'"]);
        assert_eq!(course.teachers, ["Ménard A.", "Le Demo B."]);
        assert_eq!(course.groups, ["L2 ARTS TD3"]);
        assert_eq!(course.description.as_deref(), Some("Cours annulé"));
        assert_eq!(course.color.as_deref(), Some("FFAA00"));
    }

    #[test]
    fn journee_entiere_de_minuit_a_minuit() {
        let courses = fixture(day(2026, 11, 11), day(2026, 11, 11));
        let course = courses.first().unwrap();
        assert_eq!(course.id, "9911");
        assert_eq!(course.start.to_rfc3339(), "2026-11-11T00:00:00+01:00");
        assert_eq!(course.end.to_rfc3339(), "2026-11-12T00:00:00+01:00");
        // Plusieurs jours : le dernier compris.
        let long = one(&json!({ "id": 1, "start": "2026-12-21", "end": "2026-12-23" }));
        assert_eq!(long.end.to_rfc3339(), "2026-12-24T00:00:00+01:00");
        // La veille et le lendemain ne la contiennent pas.
        assert!(fixture(day(2026, 11, 10), day(2026, 11, 10)).is_empty());
        assert!(fixture(day(2026, 11, 12), day(2026, 11, 12)).is_empty());
    }

    #[test]
    fn heures_de_paris_a_travers_les_changements_d_heure() {
        let at = |start: &str| one(&json!({ "id": 1, "start": start, "end": start })).start;
        assert_eq!(
            at("2026-10-08 10:45").to_rfc3339(),
            "2026-10-08T10:45:00+02:00"
        );
        // Lendemain du passage à l'heure d'hiver (25/10/2026), puis à l'heure d'été (28/03/2027).
        assert_eq!(
            at("2026-10-26 08:00").to_rfc3339(),
            "2026-10-26T08:00:00+01:00"
        );
        assert_eq!(
            at("2027-03-29 08:00").to_rfc3339(),
            "2027-03-29T08:00:00+02:00"
        );
        // Heure répétée : la première ; heure sautée : une heure plus tard.
        assert_eq!(
            at("2026-10-25 02:30").to_rfc3339(),
            "2026-10-25T02:30:00+02:00"
        );
        assert_eq!(
            at("2027-03-28 02:30").to_rfc3339(),
            "2027-03-28T03:30:00+02:00"
        );
        // Séparateur `T` et secondes.
        assert_eq!(
            at("2026-11-02T10:45:00").to_rfc3339(),
            "2026-11-02T10:45:00+01:00"
        );
    }

    #[test]
    fn evenement_sans_titre_ni_fin() {
        let courses = fixture(day(2026, 11, 9), day(2026, 11, 9));
        let untitled = courses.first().unwrap();
        assert_eq!(untitled.title, "Cours");
        assert!(untitled.groups.is_empty() && untitled.rooms.is_empty());

        let endless = one(&json!({ "id": "7", "start": "2026-11-03 09:00", "end": "bientôt" }));
        assert_eq!(endless.end, endless.start);
        // Une couleur qui n'en est pas une n'est pas rendue.
        let odd = one(&json!({ "id": 8, "start": "2026-11-03 09:00", "color": "rouge" }));
        assert_eq!(odd.color, None);
    }

    #[test]
    fn evenements_illisibles() {
        // Sans début lisible ou sans identifiant : écarté, les autres restent.
        let mixed = json!([
            { "id": 1, "start": "02/11/2026 10:45" },
            { "id": 2, "start": "" },
            { "start": "2026-11-02 08:00" },
            { "id": 3, "start": 20_261_102 },
            { "id": 4, "start": "2026-11-02 09:00", "end": "2026-11-02 10:00" },
        ]);
        let courses = parse(&mixed, day(2026, 11, 2), day(2026, 11, 2));
        assert_eq!(courses.len(), 1);
        assert_eq!(courses.first().unwrap().id, "4");

        // Aucun événement lisible, ou autre chose qu'une liste : le format a changé.
        for payload in [
            json!([{ "id": 1, "debut": "2026-11-02 10:45" }]),
            json!({ "message": "non" }),
            json!("texte"),
        ] {
            let error = parse_events(&payload, Paris, day(2026, 11, 2), day(2026, 11, 2));
            assert_eq!(error.unwrap_err().code, ErrorCode::UpstreamChanged);
        }
        // Une liste vide est une période sans cours.
        assert!(parse(&json!([]), day(2026, 11, 2), day(2026, 11, 8)).is_empty());
    }

    #[test]
    fn plage_inversee_ou_hors_des_evenements() {
        assert!(fixture(day(2026, 11, 8), day(2026, 11, 2)).is_empty());
        assert!(fixture(day(2026, 10, 26), day(2026, 11, 1)).is_empty());
        // Tout le mois : quatre séances (9913 n'a pas d'horaire, 9909 est en double).
        assert_eq!(fixture(day(2026, 11, 1), day(2026, 11, 30)).len(), 4);
    }
}
