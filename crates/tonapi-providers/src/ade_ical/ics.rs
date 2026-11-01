//! Analyse iCal (RFC 5545), sans réseau : lignes repliées, propriétés et paramètres, VEVENT,
//! dates UTC, `TZID` ou flottantes, et plage de jours locaux.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use tonapi_core::{Course, ErrorCode, Problem};

use super::PROVIDER;
use super::fields::{describe, session_kind, split_rooms, strip_date_prefix};
use crate::common::resolve_local;
use crate::net::upstream;

pub(crate) fn is_calendar(body: &str) -> bool {
    body.trim_start_matches('\u{feff}')
        .trim_start()
        .get(..15)
        .is_some_and(|head| head.eq_ignore_ascii_case("BEGIN:VCALENDAR"))
}

/// Séances du flux `ics` qui chevauchent `from..=to` (jours locaux), triées par début.
pub(crate) fn parse_timetable(
    ics: &str,
    tz: Tz,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Course>, Problem> {
    if from > to {
        return Ok(Vec::new());
    }
    let window = Window::new(tz, from, to)?;
    let mut courses = Vec::new();
    for (index, event) in read_events(ics)?.into_iter().enumerate() {
        // Le flux couvre l'année : une séance hors plage, même mal formée, ne bloque pas la semaine.
        if window.excludes(&event, tz) {
            continue;
        }
        let course = event.into_course(index + 1, tz)?;
        if window.overlaps(&course) {
            courses.push(course);
        }
    }
    courses.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.id.cmp(&b.id)));
    Ok(courses)
}

/// Plage demandée : de minuit local de `from` à minuit local du lendemain de `to` (exclu).
struct Window {
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
}

impl Window {
    fn new(tz: Tz, from: NaiveDate, to: NaiveDate) -> Result<Self, Problem> {
        let out_of_range =
            || Problem::new(ErrorCode::InvalidRequest).detail("plage de dates hors limites");
        let day_after = to.succ_opt().ok_or_else(out_of_range)?;
        Ok(Window {
            start: midnight(tz, from).ok_or_else(out_of_range)?,
            end: midnight(tz, day_after).ok_or_else(out_of_range)?,
        })
    }

    /// Vrai seulement si les dates lisibles de l'événement le placent hors de la plage ; dans le
    /// doute, l'événement est analysé (et validé) normalement.
    fn excludes(&self, event: &RawEvent, tz: Tz) -> bool {
        let Some(start) = event.start.as_ref().and_then(|raw| raw.resolve(tz)) else {
            return false;
        };
        if start >= self.end {
            return true;
        }
        if start >= self.start {
            return false;
        }
        match &event.end {
            Some(raw) => raw.resolve(tz).is_some_and(|end| end <= self.start),
            None => event.start.as_ref().is_some_and(|raw| !raw.date_only),
        }
    }

    fn overlaps(&self, course: &Course) -> bool {
        course.start < self.end && (course.start >= self.start || course.end > self.start)
    }
}

fn midnight(tz: Tz, date: NaiveDate) -> Option<DateTime<FixedOffset>> {
    resolve_local(tz, date.and_time(NaiveTime::MIN))
}

/// Date iCal brute : valeur et paramètres utiles.
struct RawTime {
    value: String,
    tzid: Option<String>,
    date_only: bool,
}

impl RawTime {
    fn new(value: &str, params: &[(String, String)]) -> Self {
        let value = value.trim().to_owned();
        let param = |name: &str| {
            params
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, v)| v.as_str())
        };
        let date_only = param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE"))
            || (value.len() == 8 && value.chars().all(|c| c.is_ascii_digit()));
        RawTime {
            tzid: param("TZID").map(str::to_owned),
            value,
            date_only,
        }
    }

    /// Instant dans le fuseau de l'université. `None` si la valeur ou le `TZID` sont illisibles.
    ///
    /// - `…Z` : UTC (c'est la forme du flux ADE) ;
    /// - `TZID=…` : heure locale de ce fuseau ;
    /// - sans fuseau : heure flottante, lue dans le fuseau de l'université ;
    /// - `VALUE=DATE` : minuit local.
    fn resolve(&self, tz: Tz) -> Option<DateTime<FixedOffset>> {
        if self.date_only {
            let date = NaiveDate::parse_from_str(&self.value, "%Y%m%d").ok()?;
            return midnight(tz, date);
        }
        let (text, utc) = match self.value.strip_suffix(['Z', 'z']) {
            Some(text) => (text, true),
            None => (self.value.as_str(), false),
        };
        let naive = NaiveDateTime::parse_from_str(text, "%Y%m%dT%H%M%S").ok()?;
        let instant = if utc {
            Utc.from_utc_datetime(&naive).with_timezone(&tz)
        } else {
            let zone = match &self.tzid {
                Some(id) => id.parse::<Tz>().ok()?,
                None => tz,
            };
            resolve_local(zone, naive)?.with_timezone(&tz)
        };
        Some(instant.fixed_offset())
    }
}

/// VEVENT tel que lu, avant conversion.
#[derive(Default)]
struct RawEvent {
    uid: Option<String>,
    summary: Option<String>,
    location: Option<String>,
    description: Option<String>,
    start: Option<RawTime>,
    end: Option<RawTime>,
    cancelled: bool,
}

impl RawEvent {
    fn set(&mut self, name: &str, params: &[(String, String)], value: &str) {
        match name {
            "UID" => self.uid = Some(value.trim().to_owned()),
            "SUMMARY" => self.summary = Some(unescape(value)),
            "LOCATION" => self.location = Some(unescape(value)),
            "DESCRIPTION" => self.description = Some(unescape(value)),
            "STATUS" => self.cancelled = value.trim().eq_ignore_ascii_case("CANCELLED"),
            "DTSTART" => self.start = Some(RawTime::new(value, params)),
            "DTEND" => self.end = Some(RawTime::new(value, params)),
            _ => {}
        }
    }

    /// `n` est le rang de l'événement dans le flux (pour les messages d'erreur, sans donnée).
    fn into_course(self, n: usize, tz: Tz) -> Result<Course, Problem> {
        let missing = |what: &str| upstream(PROVIDER, format!("événement n° {n} sans {what}"));
        let unreadable =
            |what: &str| upstream(PROVIDER, format!("événement n° {n} : {what} illisible"));

        let id = self
            .uid
            .filter(|uid| !uid.is_empty())
            .ok_or_else(|| missing("UID"))?;
        let summary = self
            .summary
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| missing("SUMMARY"))?;
        let start_raw = self.start.ok_or_else(|| missing("DTSTART"))?;
        let start = start_raw.resolve(tz).ok_or_else(|| unreadable("DTSTART"))?;
        let end = match &self.end {
            Some(raw) => raw.resolve(tz).ok_or_else(|| unreadable("DTEND"))?,
            // Journée entière sans fin : le lendemain. Sinon durée nulle (RFC 5545 §3.6.1).
            None if start_raw.date_only => start
                .date_naive()
                .succ_opt()
                .and_then(|next| midnight(tz, next))
                .ok_or_else(|| unreadable("DTSTART"))?,
            None => start,
        };
        if end < start {
            return Err(upstream(
                PROVIDER,
                format!("événement n° {n} : DTEND avant DTSTART"),
            ));
        }

        let rooms = split_rooms(self.location.as_deref().unwrap_or_default());
        let described = describe(self.description.as_deref().unwrap_or_default());
        let summary = strip_date_prefix(summary.trim()).trim().to_owned();
        let label = described.other.first().cloned();
        let cancelled =
            self.cancelled || described.cancelled || summary.to_lowercase().contains("annul");
        let kind = session_kind(&summary, label.as_deref(), &rooms);

        let title = label.clone().unwrap_or_else(|| summary.clone());
        let mut notes = Vec::new();
        if cancelled {
            notes.push("Statut : annulé".to_owned());
        }
        if title != summary {
            notes.push(summary);
        }
        notes.extend(described.other.into_iter().skip(1));

        Ok(Course {
            id,
            title,
            kind,
            start,
            end,
            rooms,
            teachers: described.teachers,
            groups: described.groups,
            description: (!notes.is_empty()).then(|| notes.join("\n")),
        })
    }
}

/// Lit les VEVENT. Le flux doit se terminer par `END:VCALENDAR`, sinon il est considéré tronqué.
fn read_events(ics: &str) -> Result<Vec<RawEvent>, Problem> {
    if !is_calendar(ics) {
        return Err(upstream(PROVIDER, "le flux n'est pas un calendrier iCal"));
    }
    let mut events = Vec::new();
    let mut current: Option<RawEvent> = None;
    // Profondeur des composants imbriqués dans un VEVENT (VALARM…), dont les champs sont ignorés.
    let mut nested = 0_u32;
    let mut closed = false;
    for line in unfold(ics) {
        let Some((name, params, value)) = split_property(&line) else {
            continue;
        };
        match name.as_str() {
            "BEGIN" if value.eq_ignore_ascii_case("VEVENT") => {
                current = Some(RawEvent::default());
                nested = 0;
            }
            "BEGIN" if current.is_some() => nested += 1,
            "END" if value.eq_ignore_ascii_case("VEVENT") => {
                if let Some(event) = current.take() {
                    events.push(event);
                }
            }
            "END" if value.eq_ignore_ascii_case("VCALENDAR") => closed = true,
            "END" if current.is_some() => nested = nested.saturating_sub(1),
            _ if nested == 0 => {
                if let Some(event) = current.as_mut() {
                    event.set(&name, &params, value);
                }
            }
            _ => {}
        }
    }
    if !closed || current.is_some() {
        return Err(upstream(PROVIDER, "flux iCal tronqué"));
    }
    Ok(events)
}

/// Défait le repliage des lignes (RFC 5545 §3.1) : une ligne qui commence par un espace ou une
/// tabulation prolonge la précédente.
fn unfold(ics: &str) -> Vec<String> {
    let normalized = ics.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<String> = Vec::new();
    for raw in normalized.split('\n') {
        if let Some(rest) = raw.strip_prefix([' ', '\t'])
            && let Some(last) = lines.last_mut()
        {
            last.push_str(rest);
        } else if !raw.is_empty() {
            lines.push(raw.to_owned());
        }
    }
    lines
}

/// Ligne de contenu décomposée : nom, paramètres, valeur.
type Property<'a> = (String, Vec<(String, String)>, &'a str);

/// `NOM;PARAM=valeur:contenu` → nom en majuscules, paramètres (noms en majuscules, guillemets
/// retirés) et contenu. Les `;` et `:` entre guillemets ne séparent rien.
fn split_property(line: &str) -> Option<Property<'_>> {
    let mut quoted = false;
    let colon = line.char_indices().find_map(|(i, c)| {
        if c == '"' {
            quoted = !quoted;
        }
        (c == ':' && !quoted).then_some(i)
    })?;
    let head = line.get(..colon)?;
    let value = line.get(colon + 1..)?;

    let mut parts = split_outside_quotes(head, ';').into_iter();
    let name = parts.next()?.trim().to_ascii_uppercase();
    let params = parts
        .filter_map(|part| {
            let (key, val) = part.split_once('=')?;
            Some((
                key.trim().to_ascii_uppercase(),
                val.trim().trim_matches('"').to_owned(),
            ))
        })
        .collect();
    Some((name, params, value))
}

fn split_outside_quotes(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if c == separator && !quoted {
            parts.push(text.get(start..i).unwrap_or_default());
            start = i + c.len_utf8();
        }
    }
    parts.push(text.get(start..).unwrap_or_default());
    parts
}

/// Échappements d'un champ TEXT : `\n`, `\,`, `\;`, `\\`. Un autre `\x` garde `x`.
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use chrono_tz::Europe::Paris;

    use super::*;

    const SEMAINE: &str = include_str!("../../tests/fixtures/ade_ical/semaine.ics");
    const CHANGEMENT_HEURE: &str =
        include_str!("../../tests/fixtures/ade_ical/changement-heure.ics");
    const INCOMPLET: &str = include_str!("../../tests/fixtures/ade_ical/evenement-incomplet.ics");

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn parse(ics: &str, from: NaiveDate, to: NaiveDate) -> Result<Vec<Course>, Problem> {
        parse_timetable(ics, Paris, from, to)
    }

    #[test]
    fn lit_une_semaine_normale() {
        let courses = parse(SEMAINE, day(2026, 10, 5), day(2026, 10, 11)).unwrap();
        let ids: Vec<&str> = courses.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "ADE60-demo-0001",
                "ADE60-demo-0002",
                "ADE60-demo-0003",
                "ADE60-demo-0004"
            ]
        );

        // Cours magistral : UTC → Paris, libellé tiré de la description, enseignant remis à l'endroit.
        let cm = &courses[0];
        assert_eq!(cm.title, "Programmation web");
        assert_eq!(cm.kind.as_deref(), Some("CM"));
        assert_eq!(cm.start.to_rfc3339(), "2026-10-06T08:00:00+02:00");
        assert_eq!(cm.end.to_rfc3339(), "2026-10-06T09:30:00+02:00");
        assert_eq!(cm.rooms, ["Amphi A"]);
        assert_eq!(cm.teachers, ["Camille Demo"]);
        assert_eq!(cm.groups, ["INFO GR A - L1 Portail informatique"]);
        assert_eq!(cm.description.as_deref(), Some("HAE102E Info GrA"));

        // Deux salles (`\,`), deux enseignants, enseignant fictif d'ADE écarté, TP par la salle.
        let tp = &courses[1];
        assert_eq!(tp.rooms, ["Tdinfo 36.205", "Tdinfo 36.206"]);
        assert_eq!(tp.teachers, ["Alex Exemple", "Camille Demo"]);
        assert_eq!(tp.kind.as_deref(), Some("TP"));
        assert_eq!(tp.title, "Bases de données");

        // Sans description : titre = résumé, séance TD d'après la salle.
        let td = &courses[2];
        assert_eq!(td.title, "SAE103 Design");
        assert_eq!(td.kind.as_deref(), Some("TD"));
        assert!(td.teachers.is_empty() && td.groups.is_empty());
        assert!(td.description.is_none());

        // Événement ponctuel : préfixe de date retiré.
        let reunion = &courses[3];
        assert_eq!(reunion.title, "Réunion de rentrée");
        assert_eq!(reunion.kind.as_deref(), Some("Événement"));
    }

    #[test]
    fn lignes_repliees_et_echappements() {
        let courses = parse(SEMAINE, day(2026, 10, 6), day(2026, 10, 6)).unwrap();
        let tp = courses.iter().find(|c| c.id == "ADE60-demo-0002").unwrap();
        // Le libellé du cours est replié sur trois lignes dans la fixture.
        assert_eq!(tp.title, "Bases de données");
        assert_eq!(unescape(r"a\, b\; c\\d\ne\Nf"), "a, b; c\\d\ne\nf");
        assert_eq!(unfold("A:1\r\n 2\r\n\t3\r\nB:4\r\n"), ["A:123", "B:4"]);
    }

    #[test]
    fn evenement_hors_plage_ignore() {
        // La fixture contient aussi une séance le 2026-12-01 et le 2026-10-06.
        let courses = parse(SEMAINE, day(2026, 10, 13), day(2026, 10, 19)).unwrap();
        assert!(courses.is_empty());
        let courses = parse(SEMAINE, day(2026, 12, 1), day(2026, 12, 1)).unwrap();
        assert_eq!(courses.len(), 1);
        assert_eq!(courses[0].id, "ADE60-demo-0005");
        // Plage inversée : rien, sans erreur.
        assert!(
            parse(SEMAINE, day(2026, 10, 11), day(2026, 10, 5))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn un_evenement_mal_forme_hors_plage_ne_bloque_pas() {
        let ics = "BEGIN:VCALENDAR\r\n\
                   BEGIN:VEVENT\r\nUID:u1\r\nDTSTART:20260915T080000Z\r\nDTEND:20260915T090000Z\r\nEND:VEVENT\r\n\
                   BEGIN:VEVENT\r\nUID:u2\r\nSUMMARY:Cours\r\nDTSTART:20261014T080000Z\r\nDTEND:20261014T090000Z\r\nEND:VEVENT\r\n\
                   END:VCALENDAR\r\n";
        let courses = parse(ics, day(2026, 10, 12), day(2026, 10, 18)).unwrap();
        assert_eq!(courses.len(), 1);
        // Dans la plage, le même défaut reste une erreur.
        assert!(parse(ics, day(2026, 9, 14), day(2026, 9, 20)).is_err());
    }

    #[test]
    fn la_plage_se_lit_en_jours_locaux() {
        // 2026-10-06 22:30 UTC = 2026-10-07 00:30 à Paris : appartient au 7, pas au 6.
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:u1\r\nSUMMARY:Nuit\r\n\
                   DTSTART:20261006T223000Z\r\nDTEND:20261006T233000Z\r\nEND:VEVENT\r\n\
                   END:VCALENDAR\r\n";
        assert!(
            parse(ics, day(2026, 10, 6), day(2026, 10, 6))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            parse(ics, day(2026, 10, 7), day(2026, 10, 7))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn passage_heure_d_ete_et_d_hiver() {
        let courses = parse(CHANGEMENT_HEURE, day(2026, 10, 23), day(2027, 3, 29)).unwrap();
        let starts: Vec<(String, String)> = courses
            .iter()
            .map(|c| (c.id.clone(), c.start.to_rfc3339()))
            .collect();
        let start = |id: &str| {
            starts
                .iter()
                .find(|(i, _)| i == id)
                .map(|(_, s)| s.as_str())
                .unwrap()
        };
        // Même heure UTC avant et après le 25 octobre 2026 : décalage +02:00 puis +01:00.
        assert_eq!(start("avant-hiver"), "2026-10-23T08:00:00+02:00");
        assert_eq!(start("apres-hiver"), "2026-10-26T08:00:00+01:00");
        // Heure locale répétée (02:30 le 25 octobre) : première occurrence, +02:00.
        assert_eq!(start("repetee"), "2026-10-25T02:30:00+02:00");
        // Heure locale inexistante (02:30 le 28 mars 2027) : décalée après la coupure, +02:00.
        assert_eq!(start("inexistante"), "2027-03-28T03:30:00+02:00");
        // Heure locale ordinaire déclarée par `TZID`, et flottante lue dans le fuseau de l'université.
        assert_eq!(start("tzid"), "2026-11-02T09:00:00+01:00");
        assert_eq!(start("flottante"), "2026-11-03T10:00:00+01:00");
        // Journée entière : minuit local, fin le lendemain minuit local.
        let jour = courses.iter().find(|c| c.id == "journee").unwrap();
        assert_eq!(jour.start.to_rfc3339(), "2026-11-11T00:00:00+01:00");
        assert_eq!(jour.end.to_rfc3339(), "2026-11-12T00:00:00+01:00");
    }

    #[test]
    fn un_fuseau_inconnu_est_refuse() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:u1\r\nSUMMARY:X\r\n\
                   DTSTART;TZID=Nulle/Part:20261006T080000\r\nDTEND:20261006T090000Z\r\n\
                   END:VEVENT\r\nEND:VCALENDAR\r\n";
        let err = parse(ics, day(2026, 10, 6), day(2026, 10, 6)).unwrap_err();
        assert_eq!(err.code, ErrorCode::UpstreamChanged);
        assert!(err.detail.unwrap().contains("DTSTART"));
    }

    #[test]
    fn champ_obligatoire_manquant_donne_upstream_changed() {
        // Quatre événements : sans UID, sans DTSTART, sans SUMMARY, DTEND avant DTSTART.
        for (n, attendu) in [
            (1, "sans UID"),
            (2, "sans DTSTART"),
            (3, "sans SUMMARY"),
            (4, "DTEND avant DTSTART"),
        ] {
            let ics = nth_event(INCOMPLET, n);
            let err = parse(&ics, day(2026, 10, 1), day(2026, 10, 31)).unwrap_err();
            assert_eq!(err.code, ErrorCode::UpstreamChanged, "événement {n}");
            let detail = err.detail.unwrap();
            assert!(detail.contains(attendu), "{detail}");
            assert!(detail.starts_with("ade-ical-link"), "{detail}");
            assert!(!detail.contains("Camille"), "{detail}");
        }
    }

    /// Calendrier réduit au `n`-ième VEVENT de la fixture (rang à partir de 1).
    fn nth_event(ics: &str, n: usize) -> String {
        let mut blocks = ics.split("BEGIN:VEVENT");
        let header = blocks.next().unwrap();
        let block = blocks.nth(n - 1).unwrap();
        let body = block.split("END:VEVENT").next().unwrap();
        format!("{header}BEGIN:VEVENT{body}END:VEVENT\r\nEND:VCALENDAR\r\n")
    }

    #[test]
    fn sans_dtend_la_duree_est_nulle() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:u1\r\nSUMMARY:Point\r\n\
                   DTSTART:20261006T060000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let courses = parse(ics, day(2026, 10, 6), day(2026, 10, 6)).unwrap();
        assert_eq!(courses[0].start, courses[0].end);
    }

    #[test]
    fn refuse_ce_qui_n_est_pas_un_calendrier() {
        for body in ["<!DOCTYPE html><html></html>", "", "BEGIN:VCAL"] {
            let err = parse(body, day(2026, 10, 1), day(2026, 10, 2)).unwrap_err();
            assert_eq!(err.code, ErrorCode::UpstreamChanged);
        }
        // Flux coupé en plein événement.
        let tronque = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:u1\r\n";
        let err = parse(tronque, day(2026, 10, 1), day(2026, 10, 2)).unwrap_err();
        assert!(err.detail.unwrap().contains("tronqué"));
        // Calendrier vide mais complet : pas d'erreur.
        let vide = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n";
        assert!(
            parse(vide, day(2026, 10, 1), day(2026, 10, 2))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn les_composants_imbriques_sont_ignores() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:u1\r\nSUMMARY:Cours\r\n\
                   DTSTART:20261006T060000Z\r\nDTEND:20261006T070000Z\r\n\
                   BEGIN:VALARM\r\nSUMMARY:Rappel\r\nEND:VALARM\r\nEND:VEVENT\r\n\
                   END:VCALENDAR\r\n";
        let courses = parse(ics, day(2026, 10, 6), day(2026, 10, 6)).unwrap();
        assert_eq!(courses[0].title, "Cours");
    }

    #[test]
    fn seance_annulee_signalee_dans_la_description() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:u1\r\nSUMMARY:HAI101I Maths\r\n\
                   DTSTART:20261006T060000Z\r\nDTEND:20261006T070000Z\r\nSTATUS:CANCELLED\r\n\
                   END:VEVENT\r\nEND:VCALENDAR\r\n";
        let courses = parse(ics, day(2026, 10, 6), day(2026, 10, 6)).unwrap();
        assert_eq!(courses[0].description.as_deref(), Some("Statut : annulé"));
    }
}
