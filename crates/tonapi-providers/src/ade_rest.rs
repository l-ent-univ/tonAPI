//! Emploi du temps ADE, API REST du portail (`<portail>/RestApi/`), derrière le CAS de l'université.
//!
//! Parcours, d'après `Docs/recherche/Rennes/apis/ade-portal-rest.md` :
//!
//! 1. `POST sso/accessToken` donne un jeton SSO court ; s'il est vide (HTTP 200 + `errMsg`), on
//!    traverse le CAS avec le TGC de la session ([`Net::open_protected`]) puis on réessaie ;
//! 2. `POST RestApi/portal/ssologin` (Bearer = jeton SSO) donne le jeton d'API (JWT, 30 min), le
//!    jeton de rafraîchissement et le numéro de projet ;
//! 3. les données se lisent sous `RestApi/portal/projects/<projet>/` avec le jeton d'API. Sur 401 (ou
//!    redirection), le jeton est renouvelé (`refreshtoken`, sinon nouvelle ouverture de session) et
//!    la requête n'est rejouée **qu'une fois**.
//!
//! À Rennes, ADE n'attribue aucun groupe à l'étudiant : les ressources à interroger viennent de sa
//! sélection personnelle (`PortalResourcesAllSelection`). Les jetons ADE restent dans
//! `session.tokens["ade-rest.<hôte>"]`, jamais le mot de passe.
//!
//! Faits non confirmés par la documentation, marqués `[inconnu]` ci-dessous : format exact de la
//! sélection, champs de l'arbre au-delà de `id`/`name`/`branch`/`leaf`, type de séance.

use std::collections::HashSet;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use chrono_tz::Tz;
use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tonapi_core::{Course, ErrorCode, Problem, TimetableGroup};
use url::Url;

use crate::Context;
use crate::common::{resolve_local, token_key};
use crate::net::{Net, upstream};

/// Nom du fournisseur dans les messages d'erreur.
const PROVIDER: &str = "ade-portal-rest";
/// Profondeur maximale d'un arbre de ressources lu depuis le service.
const MAX_DEPTH: usize = 32;
/// Nombre maximal de ressources sélectionnées interrogées pour une même plage (une requête chacune).
const MAX_RESOURCES: usize = 30;

/// Jetons ADE gardés dans la session de l'étudiant (`ade-rest.<hôte>`).
#[derive(Clone, Serialize, Deserialize)]
struct Tokens {
    /// Jeton d'API (JWT) pour les appels de données.
    access: String,
    /// Jeton de rafraîchissement (UUID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh: Option<String>,
    /// Projet ADE de l'année (`authData.projectId`).
    project: i64,
}

// Analyse pure

/// Les réponses sont enveloppées dans `{"data": …}` ; retire un niveau s'il y en a un.
fn unwrap_data(value: &Value) -> &Value {
    value.get("data").unwrap_or(value)
}

fn non_empty_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Identifiant ADE : nombre ou chaîne.
fn id_string(value: &Value) -> Option<String> {
    match value {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        _ => None,
    }
}

/// Jeton SSO de `sso/accessToken` (`token` ou `data.token`). Vide quand ADE n'a pas de session :
/// la réponse est alors un HTTP 200 avec `errMsg`.
fn parse_sso_token(value: &Value) -> Option<String> {
    non_empty_str(value, "token")
        .or_else(|| value.get("data").and_then(|d| non_empty_str(d, "token")))
        .map(str::to_owned)
}

/// Réponse de `ssologin`. `authData` est lu tel que `data.authData.authData` ou `data.authData` ;
/// le projet est `authData.projectId` (le `data.projectId` observé vaut 0).
fn parse_ssologin(value: &Value) -> Result<Tokens, Problem> {
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
    let access = non_empty_str(auth, "accessToken")
        .ok_or_else(|| upstream(PROVIDER, "ssologin sans jeton d'API"))?;
    Ok(Tokens {
        access: access.to_owned(),
        refresh: non_empty_str(auth, "refreshToken").map(str::to_owned),
        project,
    })
}

/// Réponse de `refreshtoken` : nouveau jeton d'API et, si présent, nouveau jeton de
/// rafraîchissement.
fn parse_refresh(value: &Value) -> Option<(String, Option<String>)> {
    let data = unwrap_data(value);
    let access = non_empty_str(data, "accessToken")?.to_owned();
    Some((
        access,
        non_empty_str(data, "refreshToken").map(str::to_owned),
    ))
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
fn parse_selection(value: &Value) -> Result<Vec<i64>, Problem> {
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
    let id = node
        .get("id")
        .and_then(id_string)
        .ok_or_else(|| upstream(PROVIDER, "nœud de l'arbre sans `id`"))?;
    let name = non_empty_str(node, "name")
        .ok_or_else(|| upstream(PROVIDER, "nœud de l'arbre sans `name`"))?;
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
fn parse_groups(tree: &Value, selected: &[i64]) -> Result<Vec<TimetableGroup>, Problem> {
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
        match non_empty_str(item, "category") {
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
fn parse_offset(raw: &str) -> Option<FixedOffset> {
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
fn parse_clock(raw: &str) -> Option<(NaiveTime, Option<FixedOffset>)> {
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
        let Some(category) = non_empty_str(participant, "category") else {
            continue;
        };
        if !categories.contains(&category.to_ascii_lowercase().as_str()) {
            continue;
        }
        if names.len() >= 50 {
            break;
        }
        if let Some(name) = non_empty_str(participant, "name")
            && !names.iter().any(|n| n == name)
        {
            names.push(name.to_owned());
        }
    }
    names
}

fn parse_event(event: &Value, tz: Tz) -> Result<Course, Problem> {
    let id = event
        .get("id")
        .and_then(id_string)
        .or_else(|| event.get("activityId").and_then(id_string))
        .ok_or_else(|| upstream(PROVIDER, "événement sans `id`"))?;
    let date = non_empty_str(event, "date")
        .ok_or_else(|| upstream(PROVIDER, "événement sans `date`"))
        .and_then(|d| {
            NaiveDate::parse_from_str(d, "%d/%m/%Y")
                .map_err(|_| upstream(PROVIDER, "date d'événement illisible"))
        })?;
    let start_raw = non_empty_str(event, "startHour")
        .ok_or_else(|| upstream(PROVIDER, "événement sans `startHour`"))?;
    let start = instant(date, start_raw, tz)?;
    // Fin absente : séance sans durée connue ; une fin avant le début est ramenée au début.
    let end = match non_empty_str(event, "endHour") {
        Some(raw) => instant(date, raw, tz)?.max(start),
        None => start,
    };
    let title = non_empty_str(event, "name")
        .or_else(|| {
            let fields = event.get("fields")?;
            non_empty_str(fields, "Nom").or_else(|| non_empty_str(fields, "NAME"))
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
fn normalize(mut courses: Vec<Course>) -> Vec<Course> {
    courses.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.id.cmp(&b.id)));
    let mut seen = HashSet::new();
    courses.retain(|c| seen.insert(c.id.clone()));
    courses
}

/// Cours de `from` à `to` (inclus, jours du fuseau de l'université) d'une réponse
/// `resourcesEventsPortal`.
fn parse_events(
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
fn events_filter(resource: i64, from: NaiveDate, to: NaiveDate) -> String {
    format!(
        "resources;{resource}/startdate;{}/enddate;{}/level;=8/field;Nom;CODEZ_RESOURCE;OWNER;",
        from.format("%m%d%Y"),
        to.format("%m%d%Y"),
    )
}

// Récupération

/// Réponse d'ADE : des données, ou une session perdue (401 ou redirection, p. ex. vers le CAS).
enum Reply {
    Data(Value),
    Lost,
}

struct Client {
    net: Net,
    /// Racine du portail, avec une barre finale.
    base: Url,
    timezone: String,
    tokens: Option<Tokens>,
    /// Les jetons viennent d'être obtenus : un 401 ne justifie pas de les renouveler.
    fresh: bool,
    key: String,
}

impl Client {
    fn open(ctx: &Context<'_>, base_url: &Url) -> Result<(Self, Tz), Problem> {
        let timezone = ctx.university.university.timezone.clone();
        let tz: Tz = timezone.parse().map_err(|_| {
            Problem::new(ErrorCode::Internal).detail(format!(
                "fuseau horaire `{timezone}` inconnu dans la configuration"
            ))
        })?;
        let mut base = base_url.clone();
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        let key = token_key("ade-rest", &base);
        let tokens = ctx
            .session
            .tokens
            .get(&key)
            .and_then(|raw| serde_json::from_str::<Tokens>(raw).ok());
        let net = Net::open(ctx, PROVIDER)?;
        Ok((
            Client {
                net,
                base,
                timezone,
                tokens,
                fresh: false,
                key,
            },
            tz,
        ))
    }

    /// Remet jetons et cookies dans la session de l'étudiant.
    fn finish(self, ctx: &mut Context<'_>) {
        match self.tokens.and_then(|t| serde_json::to_string(&t).ok()) {
            Some(raw) => {
                ctx.session.tokens.insert(self.key, raw);
            }
            None => {
                ctx.session.tokens.remove(&self.key);
            }
        }
        self.net.save(ctx);
    }

    fn url(&self, path: &str) -> Result<Url, Problem> {
        self.base
            .join(path)
            .map_err(|_| upstream(PROVIDER, "adresse du portail inutilisable"))
    }

    fn headers(
        &self,
        bearer: Option<&str>,
        body: bool,
        timezone: bool,
    ) -> Result<HeaderMap, Problem> {
        let invalid = || upstream(PROVIDER, "en-tête de requête inutilisable");
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(
            header::REFERER,
            HeaderValue::from_str(self.base.as_str()).map_err(|_| invalid())?,
        );
        if body {
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
        }
        if timezone {
            headers.insert(
                HeaderName::from_static("clienttimezone"),
                HeaderValue::from_str(&self.timezone).map_err(|_| invalid())?,
            );
        }
        if let Some(token) = bearer {
            let mut value =
                HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| invalid())?;
            value.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, value);
        }
        Ok(headers)
    }

    /// Une requête. 401 et redirections donnent [`Reply::Lost`] ; les autres statuts d'erreur et les
    /// corps non JSON donnent une erreur qui ne cite ni corps ni jeton.
    async fn send(
        &mut self,
        method: Method,
        url: Url,
        bearer: Option<&str>,
        body: Option<&str>,
        timezone: bool,
    ) -> Result<Reply, Problem> {
        let headers = self.headers(bearer, body.is_some(), timezone)?;
        let (status, _, text) = self
            .net
            .request(method, url, headers, body.map(str::to_owned))
            .await?;
        if status == reqwest::StatusCode::UNAUTHORIZED || status.is_redirection() {
            return Ok(Reply::Lost);
        }
        if !status.is_success() {
            return Err(Problem::new(ErrorCode::ServiceUnavailable)
                .detail(format!("{PROVIDER} : réponse HTTP {status}")));
        }
        serde_json::from_str(&text)
            .map(Reply::Data)
            .map_err(|_| upstream(PROVIDER, format!("réponse non JSON (HTTP {status})")))
    }

    /// Jeton SSO, ou `None` si ADE n'a pas de session pour ce navigateur.
    async fn sso_token(&mut self) -> Result<Option<String>, Problem> {
        let url = self.url("sso/accessToken")?;
        match self
            .send(Method::POST, url, None, Some("{}"), false)
            .await?
        {
            Reply::Data(value) => Ok(parse_sso_token(&value)),
            Reply::Lost => Ok(None),
        }
    }

    /// Ouvre la session ADE : jeton SSO (après le CAS si besoin), puis `ssologin`.
    async fn login(&mut self) -> Result<(), Problem> {
        let mut sso = self.sso_token().await?;
        if sso.is_none() {
            // Pas de session sur le portail : la chaîne CAS silencieuse (TGC) la crée.
            self.net.open_protected(self.base.clone()).await?;
            sso = self.sso_token().await?;
        }
        let Some(sso) = sso else {
            return Err(upstream(PROVIDER, "jeton SSO vide après le CAS"));
        };
        let url = self.url("RestApi/portal/ssologin")?;
        match self
            .send(Method::POST, url, Some(&sso), Some("{}"), false)
            .await?
        {
            Reply::Data(value) => {
                self.tokens = Some(parse_ssologin(&value)?);
                self.fresh = true;
                Ok(())
            }
            Reply::Lost => Err(Problem::new(ErrorCode::SessionExpired)
                .detail("ADE a refusé le jeton SSO : se reconnecter")),
        }
    }

    /// Renouvelle le jeton d'API : `refreshtoken`, sinon nouvelle ouverture de session.
    async fn renew(&mut self) -> Result<(), Problem> {
        if let Some(tokens) = self.tokens.clone()
            && let Some(refresh) = tokens.refresh.as_deref()
        {
            let mut url = self.url("RestApi/portal/refreshtoken")?;
            url.query_pairs_mut().append_pair("refresh_token", refresh);
            // Sans `Authorization`, comme le front.
            if let Ok(Reply::Data(value)) = self.send(Method::GET, url, None, None, true).await
                && let Some((access, new_refresh)) = parse_refresh(&value)
            {
                self.tokens = Some(Tokens {
                    access,
                    refresh: new_refresh.or(tokens.refresh),
                    project: tokens.project,
                });
                self.fresh = true;
                return Ok(());
            }
        }
        self.login().await
    }

    /// `GET` d'une ressource du projet, avec le contrat « un seul nouvel essai sur 401 ».
    async fn get_project(&mut self, path: &str) -> Result<Value, Problem> {
        if self.tokens.is_none() {
            self.login().await?;
        }
        let url = |client: &Self| -> Result<Url, Problem> {
            let project = client.tokens.as_ref().map_or(0, |t| t.project);
            client.url(&format!("RestApi/portal/projects/{project}/{path}"))
        };
        let access = |client: &Self| client.tokens.as_ref().map(|t| t.access.clone());

        let first = url(self)?;
        if let Reply::Data(value) = self
            .send(Method::GET, first, access(self).as_deref(), None, true)
            .await?
        {
            self.fresh = false;
            return Ok(value);
        }
        let expired = || {
            Problem::new(ErrorCode::SessionExpired).detail("ADE refuse le jeton : se reconnecter")
        };
        if self.fresh {
            self.tokens = None;
            return Err(expired());
        }
        self.renew().await?;
        let second = url(self)?;
        match self
            .send(Method::GET, second, access(self).as_deref(), None, true)
            .await?
        {
            Reply::Data(value) => {
                self.fresh = false;
                Ok(value)
            }
            Reply::Lost => {
                self.tokens = None;
                Err(expired())
            }
        }
    }

    async fn selection(&mut self) -> Result<Vec<i64>, Problem> {
        let value = self
            .get_project("user/properties/name/PortalResourcesAllSelection")
            .await?;
        parse_selection(&value)
    }
}

async fn timetable_with(
    client: &mut Client,
    tz: Tz,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Course>, Problem> {
    if from > to {
        return Err(Problem::new(ErrorCode::InvalidRequest)
            .detail("la date de début est après la date de fin"));
    }
    let resources = client.selection().await?;
    if resources.len() > MAX_RESOURCES {
        return Err(Problem::new(ErrorCode::InvalidRequest).detail(format!(
            "plus de {MAX_RESOURCES} groupes sélectionnés dans ADE : en retirer"
        )));
    }
    // Le front envoie une requête par ressource ; on fait de même.
    let mut courses = Vec::new();
    for resource in resources {
        let path = format!(
            "resourcesEventsPortal/{}",
            events_filter(resource, from, to)
        );
        let response = client.get_project(&path).await?;
        courses.extend(parse_events(&response, tz, from, to)?);
    }
    Ok(normalize(courses))
}

async fn groups_with(client: &mut Client) -> Result<Vec<TimetableGroup>, Problem> {
    let selected = client.selection().await?;
    let tree = client.get_project("resourcestree").await?;
    parse_groups(&tree, &selected)
}

/// Cours de la plage `from..=to` pour les groupes sélectionnés par l'étudiant dans ADE.
pub(crate) async fn fetch_timetable(
    ctx: &mut Context<'_>,
    base_url: &Url,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Course>, Problem> {
    let (mut client, tz) = Client::open(ctx, base_url)?;
    let result = timetable_with(&mut client, tz, from, to).await;
    client.finish(ctx);
    result
}

/// Groupes d'étudiants de l'arbre ADE, ceux de la sélection de l'étudiant marqués `selected`.
pub(crate) async fn fetch_groups(
    ctx: &mut Context<'_>,
    base_url: &Url,
) -> Result<Vec<TimetableGroup>, Problem> {
    let (mut client, _) = Client::open(ctx, base_url)?;
    let result = groups_with(&mut client).await;
    client.finish(ctx);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Europe::Paris;
    use serde_json::json;

    const EVENTS: &str = include_str!("../tests/fixtures/ade_rest/events.json");
    const EVENTS_DST: &str = include_str!("../tests/fixtures/ade_rest/events_dst.json");
    const TREE: &str = include_str!("../tests/fixtures/ade_rest/resourcestree.json");
    const SSOLOGIN: &str = include_str!("../tests/fixtures/ade_rest/ssologin.json");
    const SELECTION: &str = include_str!("../tests/fixtures/ade_rest/selection.json");

    fn json_of(raw: &str) -> Value {
        serde_json::from_str(raw).expect("fixture JSON")
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("date valide")
    }

    fn code<T>(result: Result<T, Problem>) -> ErrorCode {
        result.err().expect("une erreur était attendue").code
    }

    #[test]
    fn evenements_cas_normal() {
        let courses =
            parse_events(&json_of(EVENTS), Paris, day(2026, 9, 1), day(2026, 9, 30)).unwrap();
        assert_eq!(courses.len(), 2, "doublon entre ressources retiré");
        let first = &courses[0];
        assert_eq!(first.id, "501");
        assert_eq!(first.title, "Développement web");
        assert_eq!(first.start.to_rfc3339(), "2026-09-02T08:00:00+02:00");
        assert_eq!(first.end.to_rfc3339(), "2026-09-02T10:00:00+02:00");
        assert_eq!(first.teachers, ["Mme Dupont"]);
        assert_eq!(first.rooms, ["Salle 12"]);
        assert_eq!(first.groups, ["TP 1"]);
        assert_eq!(first.kind, None);
        // Sans décalage : heure de l'université ; sans titre : « Cours » ; sans fin : durée nulle.
        let second = &courses[1];
        assert_eq!(second.id, "502");
        assert_eq!(second.title, "Cours");
        assert_eq!(second.start.to_rfc3339(), "2026-09-03T14:00:00+02:00");
        assert_eq!(second.end, second.start);
        assert!(second.rooms.is_empty() && second.teachers.is_empty());
    }

    #[test]
    fn evenements_filtres_sur_la_plage() {
        let courses =
            parse_events(&json_of(EVENTS), Paris, day(2026, 9, 3), day(2026, 9, 3)).unwrap();
        assert_eq!(courses.len(), 1);
        assert_eq!(courses[0].id, "502");
    }

    #[test]
    fn evenements_double_enveloppe_et_a_plat() {
        let nested = json!({"data": {"data": [{"id": 1, "events": [
            {"id": 7, "date": "02/09/2026", "startHour": "08:00+0200"}
        ]}]}});
        let flat = json!([{"id": 8, "date": "02/09/2026", "startHour": "09:00"}]);
        let a = parse_events(&nested, Paris, day(2026, 9, 2), day(2026, 9, 2)).unwrap();
        let b = parse_events(&flat, Paris, day(2026, 9, 2), day(2026, 9, 2)).unwrap();
        assert_eq!((a[0].id.as_str(), b[0].id.as_str()), ("7", "8"));
        assert!(
            parse_events(
                &json!({"data": []}),
                Paris,
                day(2026, 9, 2),
                day(2026, 9, 2)
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn evenement_champ_obligatoire_manquant() {
        let sans = |champ: &str| {
            let mut event = json!({"id": 1, "date": "02/09/2026", "startHour": "08:00+0200"});
            event.as_object_mut().unwrap().remove(champ);
            parse_events(&json!([event]), Paris, day(2026, 9, 1), day(2026, 9, 30))
        };
        for champ in ["id", "date", "startHour"] {
            assert_eq!(code(sans(champ)), ErrorCode::UpstreamChanged, "{champ}");
        }
        // `activityId` remplace `id`.
        let event = json!({"activityId": 42, "date": "02/09/2026", "startHour": "08:00"});
        let courses =
            parse_events(&json!([event]), Paris, day(2026, 9, 1), day(2026, 9, 30)).unwrap();
        assert_eq!(courses[0].id, "42");
    }

    #[test]
    fn evenement_valeurs_illisibles() {
        for (date, hour) in [
            ("2026-09-02", "08:00"),
            ("02/09/2026", "8h"),
            ("02/09/2026", "25:00"),
            ("02/09/2026", "08:00+99"),
        ] {
            let event = json!({"id": 1, "date": date, "startHour": hour});
            assert_eq!(
                code(parse_events(
                    &json!([event]),
                    Paris,
                    day(2026, 1, 1),
                    day(2026, 12, 31)
                )),
                ErrorCode::UpstreamChanged,
                "{date} {hour}"
            );
        }
    }

    #[test]
    fn passage_heure_d_ete_et_d_hiver() {
        let courses = parse_events(
            &json_of(EVENTS_DST),
            Paris,
            day(2026, 3, 1),
            day(2026, 11, 1),
        )
        .unwrap();
        let by_id = |id: &str| courses.iter().find(|c| c.id == id).expect(id);
        // Veille du passage à l'heure d'été : +01:00 ; le jour même, après 3 h : +02:00.
        assert_eq!(by_id("1").start.to_rfc3339(), "2026-03-28T10:00:00+01:00");
        assert_eq!(by_id("2").start.to_rfc3339(), "2026-03-29T10:00:00+02:00");
        assert_eq!(by_id("3").start.to_rfc3339(), "2026-03-29T03:00:00+02:00");
        // Passage à l'heure d'hiver (25 octobre) : 02:30 existe deux fois, la première est gardée.
        assert_eq!(by_id("4").start.to_rfc3339(), "2026-10-25T02:30:00+02:00");
        assert_eq!(by_id("5").start.to_rfc3339(), "2026-10-25T08:00:00+01:00");
        // Un décalage `Z` est converti à l'heure de Paris.
        assert_eq!(by_id("6").start.to_rfc3339(), "2026-10-25T08:00:00+01:00");
        // Une séance qui traverse le saut : 01:30+01:00 → 03:30+02:00, soit 60 minutes réelles.
        assert_eq!((by_id("7").end - by_id("7").start).num_minutes(), 60);
    }

    #[test]
    fn heure_inexistante_sans_decalage() {
        let event = json!({"id": 1, "date": "29/03/2026", "startHour": "02:30"});
        let courses = parse_events(&json!([event]), Paris, day(2026, 3, 29), day(2026, 3, 29))
            .expect("heure sautée décalée d'une heure");
        let start = courses.first().expect("une séance").start;
        assert_eq!(start.to_rfc3339(), "2026-03-29T03:30:00+02:00");
    }

    #[test]
    fn heure_sans_decalage_suit_le_fuseau_de_l_universite() {
        let event = json!({"id": 1, "date": "02/09/2026", "startHour": "08:00"});
        let tz: Tz = "America/Martinique".parse().unwrap();
        let courses = parse_events(&json!([event]), tz, day(2026, 9, 2), day(2026, 9, 2)).unwrap();
        assert_eq!(courses[0].start.to_rfc3339(), "2026-09-02T08:00:00-04:00");
    }

    #[test]
    fn decalages_acceptes() {
        for (raw, secs) in [
            ("+0200", 7200),
            ("+02:00", 7200),
            ("-0330", -12600),
            ("+02", 7200),
        ] {
            assert_eq!(
                parse_offset(raw).map(|o| o.local_minus_utc()),
                Some(secs),
                "{raw}"
            );
        }
        assert_eq!(parse_offset("Z").map(|o| o.local_minus_utc()), Some(0));
        assert!(parse_offset("+2").is_none() && parse_offset("0200").is_none());
        assert!(parse_clock("08:00:30+0100").is_some());
    }

    #[test]
    fn filtre_d_evenements_mois_d_abord() {
        assert_eq!(
            events_filter(14961, day(2026, 9, 2), day(2026, 9, 3)),
            "resources;14961/startdate;09022026/enddate;09032026/level;=8/field;Nom;CODEZ_RESOURCE;OWNER;"
        );
    }

    #[test]
    fn ssologin_cas_normal_et_projet() {
        let tokens = parse_ssologin(&json_of(SSOLOGIN)).unwrap();
        // `data.projectId` vaut 0 : le projet est `authData.projectId`.
        assert_eq!(tokens.project, 1);
        assert_eq!(tokens.access, "jeton-api-synthetique");
        assert_eq!(
            tokens.refresh.as_deref(),
            Some("00000000-0000-4000-8000-000000000001")
        );
        // Forme `data.authData.authData`.
        let nested =
            json!({"data": {"authData": {"authData": {"projectId": 3, "accessToken": "a"}}}});
        let tokens = parse_ssologin(&nested).unwrap();
        assert_eq!((tokens.project, tokens.refresh), (3, None));
    }

    #[test]
    fn ssologin_incomplet() {
        for value in [
            json!({"data": {}}),
            json!({"data": {"authData": {"projectId": 1}}}),
            json!({"data": {"authData": {"accessToken": "a"}}}),
            json!({"data": {"authData": {"projectId": 1, "accessToken": " "}}}),
        ] {
            assert_eq!(code(parse_ssologin(&value)), ErrorCode::UpstreamChanged);
        }
    }

    #[test]
    fn jeton_sso_vide_quand_pas_de_session() {
        let vide = json!({"token": "", "errMsg": "No session allowed!"});
        assert_eq!(parse_sso_token(&vide), None);
        assert_eq!(
            parse_sso_token(&json!({"token": "t"})).as_deref(),
            Some("t")
        );
        assert_eq!(
            parse_sso_token(&json!({"data": {"token": "t"}})).as_deref(),
            Some("t")
        );
    }

    #[test]
    fn rafraichissement() {
        let (access, refresh) =
            parse_refresh(&json!({"data": {"accessToken": "a2", "refreshToken": "r2"}})).unwrap();
        assert_eq!((access.as_str(), refresh.as_deref()), ("a2", Some("r2")));
        assert!(parse_refresh(&json!({"data": {}})).is_none());
    }

    #[test]
    fn selection_formats() {
        assert!(parse_selection(&json!({"data": ""})).unwrap().is_empty());
        assert!(parse_selection(&json!({"data": null})).unwrap().is_empty());
        assert_eq!(
            parse_selection(&json_of(SELECTION)).unwrap(),
            [14961, 14962]
        );
        assert_eq!(
            parse_selection(&json!({"data": "14961;14962 -1"})).unwrap(),
            [14961, 14962]
        );
        assert_eq!(
            parse_selection(&json!({"data": "[3,\"4\",3]"})).unwrap(),
            [3, 4]
        );
        assert_eq!(
            parse_selection(&json!({"data": [{"id": 5, "name": "2024"}]})).unwrap(),
            [5]
        );
        // Non vide mais sans id : un format inconnu, pas une sélection vide.
        assert_eq!(
            code(parse_selection(&json!({"data": "n/a"}))),
            ErrorCode::UpstreamChanged
        );
    }

    #[test]
    fn groupes_de_l_arbre() {
        let groups = parse_groups(&json_of(TREE), &[14962]).unwrap();
        let names: Vec<_> = groups
            .iter()
            .map(|g| (g.name.as_str(), g.selected))
            .collect();
        // Enseignants et salles ignorés ; seules les feuilles d'étudiants sont proposées.
        assert_eq!(names, [("TP 1", false), ("TP 2", true), ("Amphi", false)]);
        assert_eq!(groups[0].id, "14961");
        assert_eq!(
            groups[0].path,
            [
                "Département Démo",
                "BUT Informatique 2",
                "Groupe TD 1",
                "TP 1"
            ]
        );
        // Une branche sélectionnée apparaît aussi.
        let groups = parse_groups(&json_of(TREE), &[111]).unwrap();
        assert!(groups.iter().any(|g| g.id == "111" && g.selected));
    }

    #[test]
    fn arbre_incomplet_ou_trop_profond() {
        let sans_nom =
            json!({"data": {"category": [{"category": "trainee", "leaf": [{"id": 1}]}]}});
        assert_eq!(
            code(parse_groups(&sans_nom, &[])),
            ErrorCode::UpstreamChanged
        );
        assert_eq!(
            code(parse_groups(&json!({"data": 3}), &[])),
            ErrorCode::UpstreamChanged
        );
        let mut node = json!({"id": 1, "name": "x"});
        for _ in 0..(MAX_DEPTH + 2) {
            node = json!({"id": 1, "name": "x", "branch": [node]});
        }
        let deep = json!({"data": {"category": [{"category": "trainee", "branch": [node]}]}});
        assert_eq!(code(parse_groups(&deep, &[])), ErrorCode::UpstreamChanged);
    }

    #[test]
    fn erreurs_sans_donnees_de_l_etudiant() {
        let secret = json!({"id": 1, "date": "SECRET-123", "startHour": "08:00"});
        let error =
            parse_events(&json!([secret]), Paris, day(2026, 1, 1), day(2026, 12, 31)).unwrap_err();
        assert!(!error.to_string().contains("SECRET"));
    }
}
