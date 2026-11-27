//! Réponses de TEMPO, sans réseau : les séances de `api/events` et `api/room/<salle>`, et l'arbre
//! des salles de la page `rooms`. L'analyse des séances est reprise de Tafac (`tempoApi.js`) ; les
//! formes ont été revues sur le service lui-même ([vu] 27/11/2026, `docs/recherche/rennes-2.md`).

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime};
use chrono_tz::Tz;
use serde_json::Value;
use std::collections::HashMap;

use tonapi_core::{Course, CourseLink, Problem, ResourceKind, ResourceNode};

use super::PROVIDER;
use crate::common::{html_to_text, id_string, resolve_local};
use crate::html::{inner, tags, text};
use crate::net::upstream;

/// Titre d'une séance que TEMPO envoie sans titre.
const UNTITLED: &str = "Cours";
/// Au-delà, ce n'est pas l'adresse d'une page.
const MAX_LINK_LENGTH: usize = 2048;

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
    Ok(merge(courses))
}

/// Séances triées par début, sans doublon. Tri stable : de deux séances de même identifiant, la
/// première reçue reste.
pub(crate) fn merge(mut courses: Vec<Course>) -> Vec<Course> {
    courses.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.end.cmp(&b.end)));
    let mut seen = std::collections::HashSet::new();
    courses.retain(|course| seen.insert(course.id.clone()));
    courses
}

/// Classe des en-têtes de l'arbre des salles (dossiers et salles).
const TREE_HEADER: &str = "tempo-tree-header";
/// Classe du bouton d'une salle, dans son en-tête.
const ROOM_BUTTON: &str = "tempo-room-btn";
/// Nœuds lus au plus dans la page des salles ([vu] 27/11/2026 : 642, dont 524 salles).
const MAX_ROOM_NODES: usize = 50_000;
/// Profondeur maximale de l'arbre des salles ([vu] : 6 niveaux).
const MAX_ROOM_DEPTH: usize = 32;

/// La page est-elle celle des salles ? Son conteneur porte la classe `tempo-rooms`, qu'il y ait
/// des salles ou non.
pub(crate) fn is_rooms_page(page: &str) -> bool {
    page.contains("tempo-rooms")
}

/// Nœud de la page des salles avant assemblage : son en-tête, celui de son parent, et lui-même.
struct Listed {
    heading: String,
    parent: Option<String>,
    node: ResourceNode,
}

/// Arbre des salles de la page `rooms`.
///
/// Chaque dossier et chaque salle a un en-tête `<h3 class="… tempo-tree-header" id="heading-N"
/// parentid="#heading-P">`, dans l'ordre de l'arbre, le parent avant ses enfants. L'en-tête d'un
/// dossier contient un bouton qui porte son nom ; celui d'une salle, un élément de classe
/// `tempo-room-btn` avec `value` (le numéro à passer à `api/room/`) et `ade-name`.
///
/// Les numéros des en-têtes et ceux des salles ne viennent pas de la même suite : l'identifiant
/// d'un dossier est préfixé (`dossier-N`), celui d'une salle est son numéro.
pub(crate) fn parse_rooms(page: &str) -> Result<Vec<ResourceNode>, Problem> {
    let mut listed: Vec<Listed> = Vec::new();
    for header in tags(page, "h3")
        .into_iter()
        .filter(|tag| tag.has_class(TREE_HEADER))
    {
        if listed.len() >= MAX_ROOM_NODES {
            return Err(upstream(PROVIDER, "arbre des salles trop grand"));
        }
        let heading = header
            .attr("id")
            .filter(|id| !id.is_empty())
            .ok_or_else(|| upstream(PROVIDER, "en-tête de l'arbre des salles sans `id`"))?;
        let parent = header
            .attr("parentid")
            .map(|id| id.trim_start_matches('#').to_owned())
            .filter(|id| !id.is_empty());
        let content = inner(page, &header, "h3")
            .ok_or_else(|| upstream(PROVIDER, "en-tête de l'arbre des salles sans fin"))?;
        let room = tags(content, "div")
            .into_iter()
            .chain(tags(content, "button"))
            .find(|tag| tag.has_class(ROOM_BUTTON));
        let (id, name, folder) = if let Some(button) = room {
            let number = button
                .attr("value")
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
                .ok_or_else(|| upstream(PROVIDER, "salle sans numéro dans l'arbre"))?;
            let name = button
                .attr("ade-name")
                .map(|name| text(&name))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| text(content));
            (number, name, false)
        } else {
            let number = heading.trim_start_matches("heading-");
            (format!("dossier-{number}"), text(content), true)
        };
        // Un nœud sans nom n'est pas affichable ; ce qu'il range remonte au premier niveau.
        if name.is_empty() {
            continue;
        }
        listed.push(Listed {
            heading,
            parent,
            node: ResourceNode {
                id,
                name,
                kind: ResourceKind::Room,
                folder,
                children: Vec::new(),
                selected: false,
            },
        });
    }
    assemble(listed)
}

/// Range chaque nœud sous son parent. Un parent n'est reconnu que s'il a été lu avant : un nœud
/// dont le parent manque est mis au premier niveau, et aucune boucle n'est possible.
fn assemble(listed: Vec<Listed>) -> Result<Vec<ResourceNode>, Problem> {
    let mut index: HashMap<String, usize> = HashMap::with_capacity(listed.len());
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); listed.len()];
    let mut roots = Vec::new();
    for (position, item) in listed.iter().enumerate() {
        match item.parent.as_ref().and_then(|parent| index.get(parent)) {
            Some(&parent) => {
                if let Some(siblings) = children.get_mut(parent) {
                    siblings.push(position);
                }
            }
            None => roots.push(position),
        }
        index.insert(item.heading.clone(), position);
    }
    let mut nodes: Vec<Option<ResourceNode>> =
        listed.into_iter().map(|item| Some(item.node)).collect();
    roots
        .into_iter()
        .map(|root| take(root, &children, &mut nodes, 0))
        .collect()
}

fn take(
    position: usize,
    children: &[Vec<usize>],
    nodes: &mut [Option<ResourceNode>],
    depth: usize,
) -> Result<ResourceNode, Problem> {
    if depth > MAX_ROOM_DEPTH {
        return Err(upstream(PROVIDER, "arbre des salles trop profond"));
    }
    let mut node = nodes
        .get_mut(position)
        .and_then(Option::take)
        .ok_or_else(|| upstream(PROVIDER, "arbre des salles incohérent"))?;
    for &child in children.get(position).map_or(&[][..], Vec::as_slice) {
        node.children.push(take(child, children, nodes, depth + 1)?);
    }
    Ok(node)
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
        link: link(event),
    })
}

/// Lien joint à la séance (`url`, `url_label`) : seulement une adresse en `http` ou `https`, qui
/// tienne en une adresse. Le libellé perd le deux-points qui le termine (`VOTRE PLANNING :`).
fn link(event: &Value) -> Option<CourseLink> {
    let url = event.get("url").and_then(Value::as_str)?.trim();
    let web = ["http://", "https://"].iter().any(|scheme| {
        url.get(..scheme.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(scheme))
    });
    if !web || url.len() > MAX_LINK_LENGTH || url.chars().any(char::is_whitespace) {
        return None;
    }
    let label = event
        .get("url_label")
        .and_then(Value::as_str)
        .map(html_to_text)
        .map(|label| label.trim_end_matches(':').trim_end().to_owned())
        .filter(|label| !label.is_empty());
    Some(CourseLink {
        url: url.to_owned(),
        label,
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
    const ROOMS: &str = include_str!("../../tests/fixtures/tempo/salles.html");
    const ROOM: &str = include_str!("../../tests/fixtures/tempo/salle.json");

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

    /// Page à la forme vue le 27/11/2026 : dossiers et salles en accordéon, chacun avec son
    /// en-tête et celui de son parent.
    #[test]
    fn arbre_des_salles() {
        assert!(is_rooms_page(ROOMS));
        let tree = parse_rooms(ROOMS).unwrap();
        let top: Vec<_> = tree
            .iter()
            .map(|n| (n.id.as_str(), n.name.as_str(), n.folder))
            .collect();
        assert_eq!(
            top,
            [
                ("dossier-9001", "01 - AMPHITHEATRES (2)", true),
                ("dossier-9010", "02 - SALLES DE COURS (2)", true),
            ]
        );
        // Une salle : son numéro pour `api/room/`, son nom d'ADE, entités décodées.
        let amphi = &tree[0].children[0];
        assert_eq!(
            (amphi.id.as_str(), amphi.name.as_str(), amphi.folder),
            ("6", "A1 Amphi (Démo) - 133 pl.", false)
        );
        assert!(amphi.children.is_empty());
        assert!(tree.iter().all(|n| n.kind == ResourceKind::Room));
        // Un dossier dans un dossier ; la salle 9001 n'est pas confondue avec le dossier 9001.
        let ufr = &tree[1].children[0];
        assert_eq!((ufr.name.as_str(), ufr.folder), ("01 - UFR DÉMO (2)", true));
        let rooms: Vec<_> = ufr.children.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(rooms, ["2204", "9001"]);
    }

    #[test]
    fn arbre_des_salles_incomplet() {
        let header = |attrs: &str, content: &str| {
            format!(
                r#"<div class="tempo-rooms"><h3 class="tempo-tree-header" {attrs}>{content}</h3></div>"#
            )
        };
        let room = |attrs: &str| format!(r#"<div class="tempo-room-btn" {attrs}>Salle</div>"#);
        // Sans salle : un arbre vide, pas une erreur.
        assert!(
            parse_rooms(r#"<div class="tempo-rooms"></div>"#)
                .unwrap()
                .is_empty()
        );
        // Une salle sans numéro, ou un en-tête sans `id` : le format a changé.
        for page in [
            header(r#"id="heading-1""#, &room(r#"ade-name="Salle""#)),
            header(r#"id="heading-1""#, &room(r#"value="abc""#)),
            header("", "<button>Dossier</button>"),
        ] {
            let error = parse_rooms(&page).unwrap_err();
            assert_eq!(error.code, ErrorCode::UpstreamChanged);
        }
        // Sans `ade-name`, le nom est le texte du bouton ; un parent inconnu : premier niveau.
        let page = header(
            r##"id="heading-2" parentid="#heading-404""##,
            &room(r#"value="12""#),
        );
        let tree = parse_rooms(&page).unwrap();
        assert_eq!(
            (tree[0].id.as_str(), tree[0].name.as_str()),
            ("12", "Salle")
        );
        assert!(!is_rooms_page("<html>Tempo - Mon emploi du temps</html>"));
    }

    #[test]
    fn seances_d_une_salle_et_fusion() {
        let room = parse(
            &serde_json::from_str(ROOM).unwrap(),
            day(2026, 11, 2),
            day(2026, 11, 8),
        );
        let ids: Vec<&str> = room.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["9910", "9920"]);
        // Toutes les salles de la séance, une par ligne ; les groupes en un seul texte.
        assert_eq!(room[1].rooms.len(), 2);
        assert_eq!(room[1].groups, ["Licence 3 Démo, Mineure L3, S5"]);
        assert_eq!(room[1].teachers, ["Exemple D.", "Demo C."]);
        // Deux salles demandées : la séance commune n'est rendue qu'une fois, dans l'ordre.
        let both = merge([fixture(day(2026, 11, 2), day(2026, 11, 8)), room].concat());
        let ids: Vec<&str> = both.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["9909", "9910", "9920"]);
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
        // Le lien joint à la séance, son libellé sans le deux-points final.
        let link = course.link.as_ref().unwrap();
        assert_eq!(link.url, "https://pix.exemple.test/planning");
        assert_eq!(link.label.as_deref(), Some("VOTRE PLANNING"));
    }

    #[test]
    fn lien_d_une_seance() {
        let with = |url: Value, label: Value| {
            one(&json!({"id": 1, "start": "2026-11-02 10:00", "url": url, "url_label": label})).link
        };
        // Sans adresse, ou avec autre chose qu'une adresse du web : pas de lien.
        for url in [
            json!(""),
            json!(null),
            json!(7),
            json!("javascript:alert(1)"),
            json!("/planning"),
            json!("https://exemple.test/a b"),
            json!(format!("https://exemple.test/{}", "a".repeat(2048))),
        ] {
            assert_eq!(with(url.clone(), json!("Lien")), None, "{url}");
        }
        // Le libellé est facultatif ; l'adresse est rendue telle quelle.
        let link = with(json!(" HTTPS://exemple.test/x?a=1&b=2 "), json!(" : ")).unwrap();
        assert_eq!(link.url, "HTTPS://exemple.test/x?a=1&b=2");
        assert_eq!(link.label, None);
        let link = with(
            json!("http://exemple.test/"),
            json!("Visio <b>du cours</b>"),
        )
        .unwrap();
        assert_eq!(link.label.as_deref(), Some("Visio du cours"));
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
