//! Université de démonstration : données fictives, stables et calées sur les dates demandées, pour
//! essayer l'API sans compte universitaire. Identifiants `demo` / `demo`.

use std::collections::HashSet;

use chrono::{
    DateTime, Datelike, Duration, DurationRound, FixedOffset, NaiveDate, TimeZone, Utc, Weekday,
};
use chrono_tz::Europe::Paris;
use tonapi_core::{
    Absence, AbsenceReport, AbsenceTotals, App, Course, Document, DocumentKind, ErrorCode,
    Evaluation, Exam, Grade, GradeReport, Homework, MailAddress, MailInbox, MailMessage,
    ModuleGrades, Problem, Profile, ResourceKind, ResourceNode, SemesterGrades, SemesterRef,
    TimetableGroup, TimetableResource, UnitGrade,
};

use crate::{Download, ResourceQuery, resource_tree};

pub(crate) fn check_credentials(username: &str, password: &str) -> Result<(), Problem> {
    if username == "demo" && password == "demo" {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::InvalidCredentials)
            .detail("université de démonstration : identifiant `demo`, mot de passe `demo`"))
    }
}

/// Heure de Paris → date ISO 8601 avec décalage.
fn paris(date: NaiveDate, hour: u32, minute: u32) -> DateTime<FixedOffset> {
    let naive = date.and_hms_opt(hour, minute, 0).unwrap_or_default();
    Paris
        .from_local_datetime(&naive)
        .earliest()
        .map_or_else(|| naive.and_utc().fixed_offset(), |d| d.fixed_offset())
}

/// Instant → heure de Paris, à la minute près.
fn to_paris(instant: DateTime<Utc>) -> DateTime<FixedOffset> {
    instant
        .duration_trunc(Duration::minutes(1))
        .unwrap_or(instant)
        .with_timezone(&Paris)
        .fixed_offset()
}

struct Slot {
    start: (u32, u32),
    end: (u32, u32),
    title: &'static str,
    kind: &'static str,
    room: &'static str,
    teacher: &'static str,
    group: &'static str,
}

const fn slot(
    start: (u32, u32),
    end: (u32, u32),
    title: &'static str,
    kind: &'static str,
    room: &'static str,
    teacher: &'static str,
    group: &'static str,
) -> Slot {
    Slot {
        start,
        end,
        title,
        kind,
        room,
        teacher,
        group,
    }
}

fn day_slots(weekday: Weekday) -> &'static [Slot] {
    const MON: &[Slot] = &[
        slot(
            (8, 30),
            (10, 0),
            "Développement web",
            "CM",
            "Amphi A",
            "M. Martin",
            "A2",
        ),
        slot(
            (10, 15),
            (12, 15),
            "Bases de données",
            "TD",
            "B104",
            "Mme Durand",
            "TD1",
        ),
        slot(
            (13, 45),
            (15, 45),
            "Réseaux",
            "TP",
            "C201",
            "M. Bernard",
            "TP1A",
        ),
    ];
    const TUE: &[Slot] = &[
        slot((9, 0), (11, 0), "Anglais", "TD", "B012", "Mme Smith", "TD1"),
        slot(
            (14, 0),
            (17, 0),
            "Projet tutoré (SAÉ 3.01)",
            "TP",
            "C105",
            "M. Martin",
            "TP1A",
        ),
    ];
    const WED: &[Slot] = &[slot(
        (8, 30),
        (12, 30),
        "Algorithmique avancée",
        "TP",
        "C201",
        "Mme Petit",
        "TP1A",
    )];
    const THU: &[Slot] = &[
        slot(
            (10, 15),
            (11, 45),
            "Communication",
            "CM",
            "Amphi B",
            "Mme Leroy",
            "A2",
        ),
        slot(
            (13, 30),
            (15, 30),
            "Bases de données",
            "TP",
            "C203",
            "Mme Durand",
            "TP1A",
        ),
    ];
    const FRI: &[Slot] = &[slot(
        (8, 30),
        (10, 30),
        "Mathématiques discrètes",
        "TD",
        "B104",
        "M. Moreau",
        "TD1",
    )];
    match weekday {
        Weekday::Mon => MON,
        Weekday::Tue => TUE,
        Weekday::Wed => WED,
        Weekday::Thu => THU,
        Weekday::Fri => FRI,
        Weekday::Sat | Weekday::Sun => &[],
    }
}

pub(crate) fn timetable(from: NaiveDate, to: NaiveDate) -> Vec<Course> {
    from.iter_days()
        .take_while(|d| *d <= to)
        .flat_map(|date| {
            day_slots(date.weekday())
                .iter()
                .enumerate()
                .map(move |(i, s)| Course {
                    id: format!("demo-{}-{i}", date.format("%Y%m%d")),
                    title: s.title.to_owned(),
                    kind: Some(s.kind.to_owned()),
                    start: paris(date, s.start.0, s.start.1),
                    end: paris(date, s.end.0, s.end.1),
                    rooms: vec![s.room.to_owned()],
                    teachers: vec![s.teacher.to_owned()],
                    groups: vec![s.group.to_owned()],
                    description: None,
                    color: None,
                    activity_id: None,
                    link: None,
                })
        })
        .collect()
}

/// Arbre des ressources de la démonstration : les groupes de l'étudiant, puis les enseignants et
/// les salles de son emploi du temps.
pub(crate) fn tree() -> Vec<ResourceNode> {
    fn leaf(kind: ResourceKind, id: &str, name: &str) -> ResourceNode {
        ResourceNode {
            id: id.to_owned(),
            name: name.to_owned(),
            kind,
            folder: false,
            children: Vec::new(),
            selected: false,
        }
    }
    fn folder(
        kind: ResourceKind,
        id: &str,
        name: &str,
        children: Vec<ResourceNode>,
    ) -> ResourceNode {
        ResourceNode {
            id: id.to_owned(),
            name: name.to_owned(),
            kind,
            folder: true,
            children,
            selected: false,
        }
    }
    use ResourceKind::{Group, Room, Teacher};
    vec![
        folder(
            Group,
            "demo-universite",
            "Université de démonstration",
            vec![folder(
                Group,
                "demo-but-info",
                "BUT Informatique",
                vec![folder(
                    Group,
                    "demo-info-a2",
                    "Informatique A2",
                    vec![
                        leaf(Group, "demo-td1", "TD1"),
                        leaf(Group, "demo-tp1a", "TP1A"),
                        leaf(Group, "demo-tp1b", "TP1B"),
                    ],
                )],
            )],
        ),
        folder(
            Teacher,
            "demo-enseignants",
            "Département Informatique",
            vec![
                leaf(Teacher, "demo-ens-bernard", "M. Bernard"),
                leaf(Teacher, "demo-ens-durand", "Mme Durand"),
                leaf(Teacher, "demo-ens-leroy", "Mme Leroy"),
                leaf(Teacher, "demo-ens-martin", "M. Martin"),
                leaf(Teacher, "demo-ens-moreau", "M. Moreau"),
                leaf(Teacher, "demo-ens-petit", "Mme Petit"),
                leaf(Teacher, "demo-ens-smith", "Mme Smith"),
            ],
        ),
        folder(
            Room,
            "demo-campus",
            "Campus de démonstration",
            vec![
                folder(
                    Room,
                    "demo-amphis",
                    "Amphithéâtres",
                    vec![
                        leaf(Room, "demo-salle-amphi-a", "Amphi A"),
                        leaf(Room, "demo-salle-amphi-b", "Amphi B"),
                    ],
                ),
                folder(
                    Room,
                    "demo-batiment-b",
                    "Bâtiment B",
                    vec![
                        leaf(Room, "demo-salle-b012", "B012"),
                        leaf(Room, "demo-salle-b104", "B104"),
                    ],
                ),
                folder(
                    Room,
                    "demo-batiment-c",
                    "Bâtiment C",
                    vec![
                        leaf(Room, "demo-salle-c105", "C105"),
                        leaf(Room, "demo-salle-c201", "C201"),
                        leaf(Room, "demo-salle-c203", "C203"),
                    ],
                ),
            ],
        ),
    ]
}

/// Cours des ressources demandées par leur identifiant ([`tree`]) : ceux d'un groupe, d'un
/// enseignant ou d'une salle. Un identifiant inconnu ne donne aucun cours.
pub(crate) fn timetable_of(from: NaiveDate, to: NaiveDate, resources: &[String]) -> Vec<Course> {
    let asked: Vec<TimetableResource> =
        resource_tree::resources(&tree(), &ResourceQuery::default())
            .into_iter()
            .filter(|resource| resources.contains(&resource.id))
            .collect();
    let takes_part = |course: &Course| {
        asked.iter().any(|resource| {
            let names = match resource.kind {
                ResourceKind::Group => &course.groups,
                ResourceKind::Teacher => &course.teachers,
                ResourceKind::Room => &course.rooms,
                ResourceKind::Other => return false,
            };
            names.contains(&resource.name)
        })
    };
    timetable(from, to).into_iter().filter(takes_part).collect()
}

/// Groupes que suit l'étudiant de la démonstration : son TD et son TP.
fn followed() -> HashSet<String> {
    HashSet::from(["demo-td1", "demo-tp1a"].map(str::to_owned))
}

/// L'arbre des ressources, ses groupes suivis marqués.
pub(crate) fn followed_tree() -> Vec<ResourceNode> {
    let mut tree = tree();
    resource_tree::mark_followed(&mut tree, &followed());
    tree
}

/// Groupes de la démonstration ; l'étudiant suit son TD et son TP.
pub(crate) fn groups() -> Vec<TimetableGroup> {
    resource_tree::groups(&tree(), &followed())
}

fn grade(value: f64) -> Grade {
    Grade {
        value,
        out_of: 20.0,
    }
}

fn evaluation(
    id: &str,
    name: &str,
    date: (i32, u32, u32),
    value: Option<f64>,
    coefficient: f64,
) -> Evaluation {
    Evaluation {
        id: id.to_owned(),
        name: name.to_owned(),
        date: NaiveDate::from_ymd_opt(date.0, date.1, date.2),
        grade: value.map(grade),
        coefficient: Some(coefficient),
        status: if value.is_none() {
            Some("ABS".to_owned())
        } else {
            None
        },
        ..Evaluation::default()
    }
}

pub(crate) fn grades() -> GradeReport {
    GradeReport {
        semesters: vec![SemesterGrades {
            id: "demo-s3".into(),
            name: "Semestre 3".into(),
            average: Some(grade(13.42)),
            rank: Some("12/84".into()),
            units: vec![
                UnitGrade {
                    code: "UE3.1".into(),
                    name: "Réaliser un développement d'application".into(),
                    average: Some(grade(14.1)),
                    ects: Some(5.0),
                    ..UnitGrade::default()
                },
                UnitGrade {
                    code: "UE3.2".into(),
                    name: "Optimiser des applications".into(),
                    average: Some(grade(12.75)),
                    ects: Some(5.0),
                    ..UnitGrade::default()
                },
            ],
            modules: vec![
                ModuleGrades {
                    code: "R3.01".into(),
                    name: "Développement web".into(),
                    average: Some(grade(15.0)),
                    evaluations: vec![
                        evaluation(
                            "demo-e1",
                            "Contrôle HTML/CSS",
                            (2026, 9, 25),
                            Some(16.0),
                            1.0,
                        ),
                        evaluation("demo-e2", "Projet front", (2026, 10, 9), Some(14.0), 2.0),
                    ],
                    ..ModuleGrades::default()
                },
                ModuleGrades {
                    code: "R3.07".into(),
                    name: "Bases de données".into(),
                    average: Some(grade(11.5)),
                    evaluations: vec![
                        evaluation("demo-e3", "QCM SQL", (2026, 9, 30), Some(11.5), 1.0),
                        evaluation("demo-e4", "Partiel", (2026, 10, 7), None, 3.0),
                    ],
                    ..ModuleGrades::default()
                },
            ],
            number: Some(3),
            ..SemesterGrades::default()
        }],
        available: vec![SemesterRef {
            id: "demo-s3".into(),
            name: "Semestre 3".into(),
            number: Some(3),
            year: None,
        }],
    }
}

pub(crate) fn absences(now: DateTime<Utc>) -> AbsenceReport {
    let day = |days_ago: i64, hour: u32, length: u32| {
        let date = (now - Duration::days(days_ago))
            .with_timezone(&Paris)
            .date_naive();
        (paris(date, hour, 0), paris(date, hour + length, 0))
    };
    let (s1, e1) = day(9, 8, 2);
    let (s2, e2) = day(3, 14, 3);
    AbsenceReport {
        absences: vec![
            Absence {
                id: "demo-a1".into(),
                start: s1,
                end: e1,
                justified: true,
                module: Some("Développement web".into()),
                module_id: None,
                teacher: None,
            },
            Absence {
                id: "demo-a2".into(),
                start: s2,
                end: e2,
                justified: false,
                module: Some("Projet tutoré (SAÉ 3.01)".into()),
                module_id: None,
                teacher: None,
            },
        ],
        totals: AbsenceTotals {
            justified: 1,
            unjustified: 1,
        },
        lates: Vec::new(),
        visible: None,
        unit: None,
        unit_totals: None,
        notice: None,
    }
}

pub(crate) fn homework(now: DateTime<Utc>) -> Vec<Homework> {
    let due = |days: i64| to_paris(now + Duration::days(days));
    vec![
        Homework {
            id: "demo-h1".into(),
            title: "Rendu maquette (SAÉ 3.01)".into(),
            course: Some("Projet tutoré".into()),
            due: due(2),
            description: Some("Déposer l'archive du projet et le lien de la maquette.".into()),
            url: Some("https://www.example.org/moodle/mod/assign/view.php?id=1".into()),
            submitted: Some(false),
            kind: Some("assign".into()),
        },
        Homework {
            id: "demo-h2".into(),
            title: "TP noté SQL".into(),
            course: Some("Bases de données".into()),
            due: due(6),
            description: None,
            url: Some("https://www.example.org/moodle/mod/assign/view.php?id=2".into()),
            submitted: Some(false),
            kind: Some("assign".into()),
        },
        Homework {
            id: "demo-h3".into(),
            title: "Compte rendu réseaux".into(),
            course: Some("Réseaux".into()),
            due: due(-1),
            description: None,
            url: None,
            submitted: Some(true),
            kind: Some("assign".into()),
        },
    ]
}

pub(crate) fn inbox(now: DateTime<Utc>) -> MailInbox {
    let message =
        |id: &str, subject: &str, name: &str, address: &str, hours_ago: i64, unread: bool| {
            MailMessage {
                id: id.to_owned(),
                subject: subject.to_owned(),
                from: MailAddress {
                    name: Some(name.to_owned()),
                    address: address.to_owned(),
                },
                date: to_paris(now - Duration::hours(hours_ago)),
                unread,
                preview: Some("Bonjour à toutes et à tous,".to_owned()),
            }
        };
    MailInbox {
        unread: 2,
        messages: vec![
            message(
                "demo-m1",
                "Changement de salle jeudi",
                "Scolarité",
                "scolarite@example.org",
                2,
                true,
            ),
            message(
                "demo-m2",
                "Sujet du projet tutoré",
                "M. Martin",
                "martin@example.org",
                26,
                true,
            ),
            message(
                "demo-m3",
                "Inscriptions aux associations",
                "BDE",
                "bde@example.org",
                72,
                false,
            ),
        ],
        webmail_url: Some("https://www.example.org/webmail".into()),
    }
}

pub(crate) fn exams(now: DateTime<Utc>) -> Vec<Exam> {
    let day = |offset: i64, hour: u32| {
        let date = (now + Duration::days(offset)).date_naive();
        Paris
            .from_local_datetime(&date.and_hms_opt(hour, 0, 0).unwrap_or_default())
            .earliest()
            .map(|d| d.fixed_offset())
    };
    vec![
        Exam {
            id: "S1-DEMO-EP1".to_owned(),
            title: "Algorithmique".to_owned(),
            start: day(14, 9),
            duration_minutes: Some(120),
            room: Some("Amphi A".to_owned()),
            building: Some("Bâtiment A".to_owned()),
            session: Some("Session 1".to_owned()),
            courses: vec!["DEMO-11 - Algorithmique".to_owned()],
            notice: None,
        },
        Exam {
            id: "S1-DEMO-EP2".to_owned(),
            title: "Anglais".to_owned(),
            start: day(15, 14),
            duration_minutes: Some(90),
            room: Some("B204".to_owned()),
            building: Some("Bâtiment B".to_owned()),
            session: Some("Session 1".to_owned()),
            courses: vec!["DEMO-14 - Anglais".to_owned()],
            notice: Some("Dictionnaire papier autorisé.".to_owned()),
        },
    ]
}

/// Fichier PDF minimal, identique pour chaque document de démonstration.
const DEMO_PDF: &[u8] = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[]/Count 0>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";

pub(crate) fn documents() -> Vec<Document> {
    vec![
        Document {
            id: "certificat".to_owned(),
            title: "Certificat de scolarité".to_owned(),
            kind: DocumentKind::Certificate,
            year: None,
            session: None,
            downloadable: false,
            url: Some("https://www.example.org/certificat".to_owned()),
        },
        Document {
            id: "sem.2025_S1_1".to_owned(),
            title: "Semestre 1".to_owned(),
            kind: DocumentKind::SemesterTranscript,
            year: Some("2025".to_owned()),
            session: Some("1".to_owned()),
            downloadable: true,
            url: None,
        },
    ]
}

pub(crate) fn document(id: &str) -> Result<Download, Problem> {
    if id != "sem.2025_S1_1" {
        return Err(Problem::new(ErrorCode::NotFound).detail("document inconnu"));
    }
    Ok(Download {
        content_type: "application/pdf".to_owned(),
        file_name: "sem-2025_S1_1.pdf".to_owned(),
        bytes: DEMO_PDF.to_vec(),
    })
}

pub(crate) fn apps() -> Vec<App> {
    let app = |id: &str, title: &str, category: &str| App {
        id: id.to_owned(),
        title: title.to_owned(),
        description: None,
        url: format!("https://www.example.org/{id}"),
        icon_url: None,
        category: Some(category.to_owned()),
        favorite: false,
    };
    vec![
        app("moodle", "Moodle", "Pédagogie"),
        app("webmail", "Messagerie", "Communication"),
        app("dossier", "Mon dossier étudiant", "Scolarité"),
        app("bibliotheque", "Bibliothèque", "Ressources"),
    ]
}

pub(crate) fn profile(username: &str) -> Profile {
    Profile {
        username: username.to_owned(),
        display_name: Some("Camille Demo".into()),
        first_name: Some("Camille".into()),
        last_name: Some("Demo".into()),
        email: Some("camille.demo@example.org".into()),
        student_number: Some("00000000".into()),
        program: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semaine_de_cours_en_heure_de_paris() {
        // Lundi 12 octobre 2026 (heure d'été, +02:00) → dimanche 18.
        let from = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 10, 18).unwrap();
        let courses = timetable(from, to);
        assert_eq!(courses.len(), 9);
        assert_eq!(courses[0].start.to_rfc3339(), "2026-10-12T08:30:00+02:00");
        // Après le passage à l'heure d'hiver.
        let winter = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        assert_eq!(
            timetable(winter, winter)[0].start.to_rfc3339(),
            "2026-11-02T08:30:00+01:00"
        );
    }

    #[test]
    fn identifiants_demo_seulement() {
        assert!(check_credentials("demo", "demo").is_ok());
        assert_eq!(
            check_credentials("demo", "x").unwrap_err().code,
            ErrorCode::InvalidCredentials
        );
    }
}
