//! Fournisseur `ade-ical-link` : emploi du temps ADE lu dans le flux iCal personnel de l'étudiant.
//!
//! Chaîne suivie (`Docs/Montpellier/lien-ade.md` et `emploi-du-temps.md` §9) :
//! 1. la page `link-page` de la configuration, derrière le CAS de l'université, contient le lien du
//!    flux `…/direct_cal.jsp?data=<hex>,1` ;
//! 2. ce lien se télécharge sans cookie, mais uniquement sur `feed-host` : un lien lu sur une page
//!    qui vise un autre hôte est refusé ;
//! 3. le flux iCal (VEVENT, dates UTC) est converti en [`Course`] dans le fuseau de l'université.
//!
//! Le lien donne l'emploi du temps sans connexion : c'est un secret. Il est gardé dans
//! `ctx.session.tokens["ade.ical"]` (session chiffrée) pour ne pas rouvrir la page à chaque appel,
//! et n'apparaît jamais dans une erreur. Quand le flux ne renvoie plus un calendrier (lien périmé
//! après un changement de groupes, ou générateur désactivé), la page est relue **une seule fois**.
//!
//! `feed-host` s'écrit `hôte` (port par défaut) ou `hôte:port`.

use chrono::{
    DateTime, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, TimeZone,
    Utc,
};
use chrono_tz::Tz;
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode};
use tonapi_core::{Course, ErrorCode, Problem};
use url::Url;

use crate::Context;
use crate::net::{Net, upstream};

const PROVIDER: &str = "ade-ical-link";
/// Clé de [`tonapi_session::SessionData::tokens`] qui garde le lien du flux (secret).
const TOKEN_KEY: &str = "ade.ical";
/// Page ADE qui sert le flux.
const FEED_PAGE: &str = "direct_cal.jsp";
/// Vue voisine de prose par laquelle on passe pour que la vue iCal renvoie son lien.
const OTHER_VIEW: &str = "protected/resume";

// ---------------------------------------------------------------------------------------------
// Récupération
// ---------------------------------------------------------------------------------------------

/// Emploi du temps de l'étudiant entre `from` et `to` inclus.
pub(crate) async fn fetch_timetable(
    ctx: &mut Context<'_>,
    link_page: &Url,
    feed_host: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Course>, Problem> {
    let tz = university_tz(ctx)?;
    let https_only = ctx.net.https_only;
    let stored = ctx
        .session
        .tokens
        .get(TOKEN_KEY)
        .and_then(|raw| feed_link(raw, feed_host, https_only));

    let mut net = Net::open(ctx, PROVIDER)?;
    let feed = read_feed(&mut net, link_page, feed_host, https_only, stored).await;
    net.save(ctx);

    let (ics, link) = feed?;
    ctx.session
        .tokens
        .insert(TOKEN_KEY.to_owned(), link.to_string());
    parse_timetable(&ics, tz, from, to)
}

/// `timetable_groups` : la liste des groupes n'est pas lisible avec ce fournisseur.
///
/// La sélection de groupes vit dans prose-etudiant (pages `preferences` et `resume`, appli Vaadin
/// dont la grille se charge par des échanges UIDL non documentés) ; le flux iCal, lui, ne porte
/// que les séances, sans identifiant de groupe ni arbre. Dériver les groupes des séances
/// donnerait des groupes de séance (CM, TD parents…), pas la sélection de l'étudiant.
pub(crate) fn groups_unsupported() -> Problem {
    Problem::new(ErrorCode::NotImplemented).detail(format!(
        "le fournisseur `{PROVIDER}` ne peut pas lister les groupes : la sélection se fait sur le \
         portail de l'université, et le flux iCal ne contient que les séances"
    ))
}

fn university_tz(ctx: &Context<'_>) -> Result<Tz, Problem> {
    ctx.university
        .university
        .timezone
        .parse::<Tz>()
        .map_err(|_| {
            Problem::new(ErrorCode::Internal).detail("fuseau horaire de l'université inconnu")
        })
}

/// Résultat d'un téléchargement du flux.
enum Feed {
    Calendar(String),
    /// Réponse qui n'est pas un calendrier : lien périmé ou fonction désactivée.
    Stale,
}

/// Lit le flux avec le lien gardé ; s'il est périmé (ou absent), relit la page une seule fois.
async fn read_feed(
    net: &mut Net,
    link_page: &Url,
    feed_host: &str,
    https_only: bool,
    stored: Option<Url>,
) -> Result<(String, Url), Problem> {
    if let Some(link) = stored
        && let Feed::Calendar(ics) = download(net, &link).await?
    {
        return Ok((ics, link));
    }
    let link = discover(net, link_page, feed_host, https_only).await?;
    match download(net, &link).await? {
        Feed::Calendar(ics) => Ok((ics, link)),
        Feed::Stale => Err(upstream(
            PROVIDER,
            "le lien lu sur la page ne renvoie pas un calendrier",
        )),
    }
}

/// Ouvre la page du lien derrière le CAS et en extrait le lien du flux.
async fn discover(
    net: &mut Net,
    link_page: &Url,
    feed_host: &str,
    https_only: bool,
) -> Result<Url, Problem> {
    let mut page = net.open_protected(link_page.clone()).await?;
    // Vaadin redirige vers `/invalidSession` quand sa session vient de naître : un nouvel essai.
    if page.url.path().ends_with("/invalidSession") {
        page = net.open_protected(link_page.clone()).await?;
    }
    if page.status == StatusCode::UNAUTHORIZED || page.status == StatusCode::FORBIDDEN {
        return Err(Problem::new(ErrorCode::SessionExpired));
    }
    if page.status.is_server_error() {
        return Err(unavailable(page.status));
    }
    if !page.status.is_success() {
        return Err(upstream(
            PROVIDER,
            format!("la page du lien répond HTTP {}", page.status),
        ));
    }
    if let Some(link) = find_feed_link(&page.body, feed_host, https_only) {
        return Ok(link);
    }
    // Prose (Vaadin Flow) ne met le lien dans la page que si la vue a déjà été affichée dans la
    // session ; sinon il l'envoie quand le client revient sur la vue depuis une autre. On rejoue
    // ce clic : « Groupes sélectionnés » puis « iCal ». [vu] 27/10/2026 : naviguer directement
    // vers la vue déjà affichée ne renvoie pas le lien.
    if let Some(mut vaadin) = VaadinPage::read(&page.url, &page.body) {
        let here = vaadin.location.clone();
        for location in [OTHER_VIEW, here.as_str()] {
            let body = vaadin.navigate(net, location).await?;
            if location == here
                && let Some(link) = find_feed_link(&body, feed_host, https_only)
            {
                return Ok(link);
            }
            if tracing::enabled!(tracing::Level::DEBUG) {
                tracing::debug!(
                    "{PROVIDER} : navigation Vaadin vers {location} : {} octets, marqueurs {:?}",
                    body.len(),
                    markers(&body)
                );
            }
        }
    }
    if tracing::enabled!(tracing::Level::DEBUG) {
        tracing::debug!(
            "{PROVIDER} : page sans lien de flux, marqueurs {:?}, liens {:?}",
            markers(&page.body),
            page_links(&page.body)
        );
    }
    Err(upstream(PROVIDER, "aucun lien de flux iCal sur la page"))
}

/// État d'une page Vaadin Flow nécessaire pour lui envoyer des événements (`?v-r=uidl`) : clé
/// anti-CSRF (`Vaadin-Security-Key` de l'état initial), numéro de l'UI et compteurs d'échange.
struct VaadinPage {
    endpoint: Url,
    href: Url,
    location: String,
    csrf: String,
    sync_id: u64,
    client_id: u64,
}

impl VaadinPage {
    fn read(page: &Url, body: &str) -> Option<Self> {
        let mut segments = page.path_segments()?.filter(|s| !s.is_empty());
        let context = segments.next()?;
        let location = segments.collect::<Vec<_>>().join("/");
        if location.is_empty() {
            return None;
        }
        let csrf = json_string_after(body, "\"Vaadin-Security-Key\"")?;
        let ui_id = json_number_after(body, "\"v-uiId\"").unwrap_or(0);
        let mut endpoint = page.clone();
        endpoint.set_path(&format!("/{context}/"));
        endpoint.set_query(None);
        endpoint
            .query_pairs_mut()
            .append_pair("v-r", "uidl")
            .append_pair("v-uiId", &ui_id.to_string());
        Some(VaadinPage {
            endpoint,
            href: page.clone(),
            location,
            csrf,
            sync_id: json_number_after(body, "\"syncId\"").unwrap_or(0),
            client_id: json_number_after(body, "\"clientId\"").unwrap_or(0),
        })
    }

    /// Événement `navigation` vers `location`, comme un clic sur un lien de menu. Renvoie le corps
    /// de la réponse (`for(;;);[{…}]`) et met les compteurs à jour.
    async fn navigate(&mut self, net: &mut Net, location: &str) -> Result<String, Problem> {
        let mut href = self.href.clone();
        if let Ok(path) = self.endpoint.join(location) {
            href.set_path(path.path());
        }
        let body = serde_json::json!({
            "csrfToken": self.csrf,
            "rpc": [{
                "type": "navigation",
                "location": location,
                "state": { "href": href.as_str(), "scrollPositionX": 0, "scrollPositionY": 0 },
                "link": 1
            }],
            "syncId": self.sync_id,
            "clientId": self.client_id,
        });
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=UTF-8"),
        );
        headers.insert(header::ACCEPT, HeaderValue::from_static("*/*"));
        let (status, _, text) = net
            .request(
                Method::POST,
                self.endpoint.clone(),
                headers,
                Some(body.to_string()),
            )
            .await?;
        if !status.is_success() {
            return Err(upstream(
                PROVIDER,
                format!("navigation Vaadin refusée (HTTP {status})"),
            ));
        }
        self.client_id += 1;
        if let Some(sync) = json_number_after(&text, "\"syncId\"") {
            self.sync_id = sync;
        }
        Ok(text)
    }
}

/// Chaîne JSON qui suit `key:` dans un texte (sans échappement à l'intérieur).
fn json_string_after(text: &str, key: &str) -> Option<String> {
    let rest = text.get(text.find(key)? + key.len()..)?.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let value = &rest[..rest.find('"')?];
    (!value.is_empty() && !value.contains('\\')).then(|| value.to_owned())
}

/// Entier JSON qui suit `key:` dans un texte.
fn json_number_after(text: &str, key: &str) -> Option<u64> {
    let rest = text.get(text.find(key)? + key.len()..)?.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Mots-clés présents dans une réponse, pour le diagnostic (jamais de valeur).
fn markers(body: &str) -> Vec<&'static str> {
    [
        "uidl",
        "v-r=init",
        "initApplication",
        "Vaadin.Flow",
        "appConfig",
        "direct_cal",
        "proseconsult",
        "anonymous_cal",
        "ical",
        "invalidSession",
    ]
    .into_iter()
    .filter(|marker| body.contains(marker))
    .collect()
}

/// Adresses de la page, décrites sans valeur (hôte, chemin masqué, noms de paramètres).
fn page_links(body: &str) -> std::collections::BTreeSet<String> {
    let text = body
        .replace("\\/", "/")
        .replace("\\u0026", "&")
        .replace("&amp;", "&");
    text.match_indices("http")
        .filter_map(|(start, _)| {
            let candidate = text.get(start..)?;
            let end = candidate
                .find(|c: char| {
                    c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '\\' | '`')
                })
                .unwrap_or(candidate.len());
            Url::parse(candidate.get(..end)?).ok()
        })
        .map(|url| tonapi_http::describe(&url))
        .take(40)
        .collect()
}

fn unavailable(status: StatusCode) -> Problem {
    Problem::new(ErrorCode::ServiceUnavailable)
        .detail(format!("{PROVIDER} : le serveur ADE répond HTTP {status}"))
}

/// `GET` du flux, sans suivre de redirection (ADE répond 302 pour un lien invalide).
async fn download(net: &mut Net, link: &Url) -> Result<Feed, Problem> {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("text/calendar, text/*;q=0.9"),
    );
    // Le flux part sans aucun cookie de la session, et le `JSESSIONID` du serveur ADE ne la
    // rejoint pas.
    let jar = std::mem::take(&mut net.browser.jar);
    let reply = net.request(Method::GET, link.clone(), headers, None).await;
    net.browser.jar = jar;
    let (status, _, body) = reply?;
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        return Err(unavailable(status));
    }
    if status.is_success() && is_calendar(&body) {
        Ok(Feed::Calendar(body))
    } else {
        Ok(Feed::Stale)
    }
}

fn is_calendar(body: &str) -> bool {
    body.trim_start_matches('\u{feff}')
        .trim_start()
        .get(..15)
        .is_some_and(|head| head.eq_ignore_ascii_case("BEGIN:VCALENDAR"))
}

// ---------------------------------------------------------------------------------------------
// Lien du flux
// ---------------------------------------------------------------------------------------------

/// Premier lien `direct_cal.jsp?data=…` de la page qui vise `feed_host`.
///
/// La page est de l'HTML Vaadin avec un état JSON : le lien y est en clair, mais on défait les
/// échappements usuels (`\/`, `&amp;`, `&`) au cas où le rendu change.
fn find_feed_link(body: &str, feed_host: &str, https_only: bool) -> Option<Url> {
    let text = body
        .replace("\\/", "/")
        .replace("\\u0026", "&")
        .replace("&amp;", "&");
    text.match_indices("http").find_map(|(start, _)| {
        let candidate = text.get(start..)?;
        let end = candidate
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '\\' | '`'))
            .unwrap_or(candidate.len());
        feed_link(candidate.get(..end)?, feed_host, https_only)
    })
}

/// Valide un lien de flux : https (http dans les tests), sans identifiants, sur `feed_host`, page
/// `direct_cal.jsp`, paramètre `data` composé de chiffres hexadécimaux et de virgules.
fn feed_link(raw: &str, feed_host: &str, https_only: bool) -> Option<Url> {
    let url = Url::parse(raw.trim()).ok()?;
    let scheme_ok = url.scheme() == "https" || (!https_only && url.scheme() == "http");
    let page_ok = url.path().rsplit('/').next() == Some(FEED_PAGE);
    let data_ok = url.query_pairs().any(|(key, value)| {
        key == "data"
            && !value.is_empty()
            && value.chars().all(|c| c.is_ascii_hexdigit() || c == ',')
    });
    (scheme_ok
        && url.username().is_empty()
        && url.password().is_none()
        && page_ok
        && data_ok
        && host_matches(&url, feed_host))
    .then_some(url)
}

fn host_matches(url: &Url, feed_host: &str) -> bool {
    let expected = feed_host.trim().to_ascii_lowercase();
    let Some(host) = url.host_str() else {
        return false;
    };
    match url.port() {
        Some(port) => expected == format!("{host}:{port}"),
        None => expected == host,
    }
}

// ---------------------------------------------------------------------------------------------
// Analyse iCal (sans réseau)
// ---------------------------------------------------------------------------------------------

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
    resolve_local(tz, date.and_time(NaiveTime::MIN)).map(|d| d.fixed_offset())
}

/// Heure locale → instant. Heure répétée (retour à l'heure d'hiver) : la première occurrence.
/// Heure inexistante (passage à l'heure d'été) : décalage d'avant la coupure, soit une heure plus
/// tard sur l'horloge (RFC 5545 §3.3.5).
fn resolve_local(tz: Tz, naive: NaiveDateTime) -> Option<DateTime<Tz>> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(date) | LocalResult::Ambiguous(date, _) => Some(date),
        LocalResult::None => tz
            .from_local_datetime(&naive.checked_add_signed(TimeDelta::hours(1))?)
            .earliest(),
    }
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

// ---------------------------------------------------------------------------------------------
// Sens des champs ADE
//
// Heuristiques tirées de `Docs/Montpellier/emploi-du-temps.md` §5 (code d'iCalFormater, observées
// à Montpellier) : [déduit] du seul flux de l'UM, à ne pas supposer valables ailleurs.
// ---------------------------------------------------------------------------------------------

/// Salles : `LOCATION` découpé sur `,` et `;`, sans doublon.
fn split_rooms(location: &str) -> Vec<String> {
    let mut rooms: Vec<String> = Vec::new();
    for room in location.split([',', ';']).map(str::trim) {
        if !room.is_empty() && !rooms.iter().any(|r| r == room) {
            rooms.push(room.to_owned());
        }
    }
    rooms
}

/// Contenu de `DESCRIPTION`, ligne par ligne. L'ordre des lignes n'est pas garanti : on les
/// classe par leur forme.
#[derive(Default)]
struct Described {
    teachers: Vec<String>,
    groups: Vec<String>,
    /// Lignes non reconnues ; la première est le libellé du cours.
    other: Vec<String>,
    cancelled: bool,
}

enum Teacher {
    Name(String),
    /// Enseignant fictif d'ADE (`A DEFINIR`, `ZZ …`) : ni enseignant ni libellé.
    Placeholder,
    Unrelated,
}

fn describe(description: &str) -> Described {
    let mut described = Described::default();
    for line in description.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.starts_with("(Exported") {
            continue;
        }
        if let Some(status) = status_of(line) {
            described.cancelled |= status.starts_with("annul");
            continue;
        }
        match teacher(line) {
            Teacher::Name(name) => push_unique(&mut described.teachers, name),
            Teacher::Placeholder => {}
            Teacher::Unrelated if looks_like_group(line) => {
                push_unique(&mut described.groups, collapse_spaces(line));
            }
            Teacher::Unrelated => described.other.push(collapse_spaces(line)),
        }
    }
    described
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

fn collapse_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Statut de la séance (`A valider`, `Confirmé`…), en minuscules.
fn status_of(line: &str) -> Option<String> {
    const STATUSES: [&str; 10] = [
        "a valider",
        "à valider",
        "validé",
        "confirmé",
        "annulé",
        "annule",
        "prévisionnel",
        "previsionnel",
        "réservé",
        "reserve",
    ];
    let lower = line.to_lowercase();
    STATUSES.contains(&lower.as_str()).then_some(lower)
}

/// Enseignant : `NOM   PRÉNOM`, tout en majuscules, séparés par au moins deux espaces, sans chiffre,
/// six mots au plus. Rendu `Prénom Nom`.
fn teacher(line: &str) -> Teacher {
    const PLACEHOLDERS: [&str; 4] = ["A DEFINIR", "NON AFFECT", "INCONNU", "A PRECISER"];
    let Some((surname, first)) = line.split_once("  ") else {
        return Teacher::Unrelated;
    };
    let (surname, first) = (collapse_spaces(surname), collapse_spaces(first));
    let shaped = !surname.is_empty()
        && !first.is_empty()
        && line.split_whitespace().count() <= 6
        && !line.chars().any(|c| c.is_ascii_digit())
        && line
            .chars()
            .filter(|c| c.is_alphabetic())
            .all(char::is_uppercase);
    if !shaped {
        return Teacher::Unrelated;
    }
    let upper = line.to_uppercase();
    if PLACEHOLDERS.iter().any(|p| upper.contains(p)) || upper.starts_with("ZZ") {
        return Teacher::Placeholder;
    }
    Teacher::Name(format!("{} {}", title_case(&first), title_case(&surname)))
}

fn title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut capitalize = true;
    for c in text.chars() {
        if c.is_alphabetic() {
            if capitalize {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            capitalize = false;
        } else {
            out.push(c);
            capitalize = matches!(c, ' ' | '-' | '\'');
        }
    }
    out
}

/// Groupe : `Gr A`, `GRF`, `L1`, `info`, `portail`, `série`, `TDB`, `TP1`, `année`.
fn looks_like_group(line: &str) -> bool {
    line.split_whitespace().any(|token| {
        let token = token
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        let chars: Vec<char> = token.chars().collect();
        match token.as_str() {
            "gr" | "groupe" | "portail" | "info" | "série" | "serie" | "année" | "annee" | "td"
            | "tp" => true,
            _ => {
                let short = |prefix: &str, len: std::ops::RangeInclusive<usize>| {
                    token.starts_with(prefix)
                        && len.contains(&chars.len())
                        && chars.iter().all(|c| c.is_alphanumeric())
                };
                short("gr", 3..=3) || short("td", 3..=4) || short("tp", 3..=4) || is_level(&chars)
            }
        }
    })
}

/// `L1`, `L2`, `L3` (niveaux de licence).
fn is_level(chars: &[char]) -> bool {
    matches!(chars, ['l', digit] if digit.is_ascii_digit())
}

/// Retire le préfixe `DDMMYYYY - ` des événements ponctuels.
fn strip_date_prefix(summary: &str) -> &str {
    let digits = summary.chars().take_while(char::is_ascii_digit).count();
    if digits == 8
        && let Some(rest) = summary.get(8..)
        && let Some(rest) = rest.trim_start().strip_prefix('-')
    {
        return rest.trim_start();
    }
    summary
}

/// Type de séance : suffixe explicite (`…-CM`), examen, puis salles ; « Événement » sans code de
/// module ni libellé. `None` quand rien ne permet de conclure.
fn session_kind(summary: &str, label: Option<&str>, rooms: &[String]) -> Option<String> {
    if let Some((_, suffix)) = summary.rsplit_once('-') {
        let suffix = suffix.trim().to_uppercase();
        if matches!(suffix.as_str(), "CM" | "TD" | "TP") {
            return Some(suffix);
        }
    }
    if is_exam(summary) || label.is_some_and(is_exam) {
        return Some("Examen".to_owned());
    }
    let tokens: Vec<String> = rooms
        .iter()
        .flat_map(|room| room.split(|c: char| !c.is_alphanumeric()))
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect();
    let has = |wanted: &dyn Fn(&str) -> bool| tokens.iter().any(|t| wanted(t.as_str()));
    if has(&|t| t.starts_with("amphi")) {
        return Some("CM".to_owned());
    }
    if has(&|t| matches!(t, "tdinfo" | "tpinfo" | "labo" | "tp")) || has_salle_info(&tokens) {
        return Some("TP".to_owned());
    }
    if has(&|t| t == "salle" || t == "banalisée" || is_td_token(t)) {
        return Some("TD".to_owned());
    }
    let has_code = summary.split_whitespace().any(is_module_code);
    (label.is_none() && !has_code).then(|| "Événement".to_owned())
}

fn has_salle_info(tokens: &[String]) -> bool {
    tokens
        .windows(2)
        .any(|pair| matches!(pair, [a, b] if a == "salle" && b == "info"))
}

/// `td`, `td4`…
fn is_td_token(token: &str) -> bool {
    token
        .strip_prefix("td")
        .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
}

fn is_exam(text: &str) -> bool {
    let tokens: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect();
    tokens
        .iter()
        .enumerate()
        .any(|(i, token)| match token.as_str() {
            "examen" | "partiel" | "ds" | "cc" | "contrôle" | "controle" | "épreuve"
            | "epreuve" | "rattrapage" | "soutenance" => true,
            "session" => tokens
                .get(i + 1)
                .is_some_and(|next| next.chars().all(|c| c.is_ascii_digit())),
            _ => false,
        })
}

/// Code de module : 2 à 4 lettres majuscules, 3 chiffres, une lettre facultative (`HAI102I`).
fn is_module_code(token: &str) -> bool {
    let chars: Vec<char> = token.chars().collect();
    let letters = chars.iter().take_while(|c| c.is_ascii_uppercase()).count();
    let digits = chars
        .iter()
        .skip(letters)
        .take_while(|c| c.is_ascii_digit())
        .count();
    let tail = chars.len() - letters - digits;
    (2..=4).contains(&letters)
        && digits == 3
        && (tail == 0 || (tail == 1 && chars.last().is_some_and(char::is_ascii_uppercase)))
}

#[cfg(test)]
mod tests {
    use chrono_tz::Europe::Paris;

    use super::*;

    const SEMAINE: &str = include_str!("../tests/fixtures/ade_ical/semaine.ics");
    const CHANGEMENT_HEURE: &str = include_str!("../tests/fixtures/ade_ical/changement-heure.ics");
    const INCOMPLET: &str = include_str!("../tests/fixtures/ade_ical/evenement-incomplet.ics");
    const PAGE: &str = include_str!("../tests/fixtures/ade_ical/page-ical.html");
    const HOTE: &str = "ade.example.test";

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
    fn etat_d_une_page_vaadin() {
        let page = Url::parse("https://app.example.test/prose/protected/ical").unwrap();
        let body = r#"var uidl = {"syncId":0,"clientId":0,"changes":[],"Vaadin-Security-Key":"cle-de-test"};
            var config = {"heartbeatInterval":300,"v-uiId":1};"#;
        let vaadin = VaadinPage::read(&page, body).unwrap();
        assert_eq!(
            vaadin.endpoint.as_str(),
            "https://app.example.test/prose/?v-r=uidl&v-uiId=1"
        );
        assert_eq!(vaadin.location, "protected/ical");
        assert_eq!(vaadin.csrf, "cle-de-test");
        assert_eq!((vaadin.sync_id, vaadin.client_id), (0, 0));
        assert!(VaadinPage::read(&page, "<html></html>").is_none());
        assert!(
            VaadinPage::read(&Url::parse("https://app.example.test/prose").unwrap(), body)
                .is_none()
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

    #[test]
    fn lien_extrait_de_la_page() {
        let link = find_feed_link(PAGE, HOTE, true).unwrap();
        assert_eq!(link.host_str(), Some(HOTE));
        assert!(link.path().ends_with("/direct_cal.jsp"));
        // Le lien « Visualiser le planning » (`direct/?data=`, interface GWT) n'est pas le flux.
        assert!(!link.path().contains("/direct/"));
        // Autre hôte, ou schéma http : refusé.
        assert!(find_feed_link(PAGE, "autre.example.test", true).is_none());
        let http = PAGE.replace("https://", "http://");
        assert!(find_feed_link(&http, HOTE, true).is_none());
        assert!(find_feed_link(&http, HOTE, false).is_some());
        assert!(find_feed_link("<html>rien</html>", HOTE, true).is_none());
    }

    #[test]
    fn lien_tolere_les_echappements_du_rendu() {
        let page = r#"{"value":"https:\/\/ade.example.test\/jsp\/custom\/modules\/plannings\/direct_cal.jsp?data=0a1b2c,1"}"#;
        assert!(find_feed_link(page, HOTE, true).is_some());
        let page = r#"<a href="https://ade.example.test/jsp/direct_cal.jsp?data=0a1b&amp;x=1">"#;
        assert!(find_feed_link(page, HOTE, true).is_some());
    }

    #[test]
    fn lien_invalide_refuse() {
        let ok = "https://ade.example.test/jsp/custom/modules/plannings/direct_cal.jsp?data=0a1b,1";
        assert!(feed_link(ok, HOTE, true).is_some());
        for bad in [
            // identifiants dans l'URL
            "https://u:p@ade.example.test/jsp/direct_cal.jsp?data=0a1b,1",
            // autre page
            "https://ade.example.test/jsp/direct/?data=0a1b,1",
            "https://ade.example.test/jsp/direct_cal.jsp.evil?data=0a1b,1",
            // données qui ne sont pas de l'hexadécimal
            "https://ade.example.test/jsp/direct_cal.jsp?data=zz,1",
            "https://ade.example.test/jsp/direct_cal.jsp?data=",
            "https://ade.example.test/jsp/direct_cal.jsp",
            // port non prévu par la configuration
            "https://ade.example.test:8443/jsp/direct_cal.jsp?data=0a1b,1",
            // hôte voisin
            "https://ade.example.test.evil.fr/jsp/direct_cal.jsp?data=0a1b,1",
            "ftp://ade.example.test/jsp/direct_cal.jsp?data=0a1b,1",
            "pas une url",
        ] {
            assert!(feed_link(bad, HOTE, true).is_none(), "{bad}");
        }
        // Hôte configuré avec un port.
        let with_port = "https://ade.example.test:8443/jsp/direct_cal.jsp?data=0a1b,1";
        assert!(feed_link(with_port, "ade.example.test:8443", true).is_some());
        // La casse de la configuration est sans effet.
        assert!(feed_link(ok, "ADE.example.test", true).is_some());
    }

    #[test]
    fn types_de_seance() {
        let kind = |summary: &str, label: Option<&str>, rooms: &[&str]| {
            let rooms: Vec<String> = rooms.iter().map(|r| (*r).to_owned()).collect();
            session_kind(summary, label, &rooms)
        };
        assert_eq!(
            kind("SAE103 Design Grphk-CM", None, &[]).as_deref(),
            Some("CM")
        );
        assert_eq!(
            kind("HAI102I Maths", Some("Examen de mathématiques"), &["TD 4"]).as_deref(),
            Some("Examen")
        );
        assert_eq!(
            kind("HAI102I Maths", None, &["Amphi B"]).as_deref(),
            Some("CM")
        );
        assert_eq!(
            kind("HAI102I Maths", None, &["Salle info 12"]).as_deref(),
            Some("TP")
        );
        assert_eq!(
            kind("HAI102I Maths", None, &["TD 4"]).as_deref(),
            Some("TD")
        );
        assert_eq!(kind("HAI102I Maths", None, &["Bât 36"]), None);
        assert_eq!(kind("Réunion", None, &[]).as_deref(), Some("Événement"));
    }

    #[test]
    fn enseignants_et_groupes() {
        let d = describe(
            "\n\nINFO GR A - L1 Portail\nBases\nA valider\nDEMO OGER   CAMILLE\nZZ FDS   A DEFINIR\n\
             MMI  1ère année BUT\nTDB\n(Exported :26/08/2026 14:25)\n",
        );
        assert_eq!(d.teachers, ["Camille Demo Oger"]);
        assert_eq!(
            d.groups,
            ["INFO GR A - L1 Portail", "MMI 1ère année BUT", "TDB"]
        );
        assert_eq!(d.other, ["Bases"]);
        assert!(!d.cancelled);
        assert!(is_module_code("HAI102I") && is_module_code("SAE103") && !is_module_code("Info"));
    }

    #[test]
    fn groupes_non_pris_en_charge() {
        let problem = groups_unsupported();
        assert_eq!(problem.code, ErrorCode::NotImplemented);
        assert!(problem.detail.unwrap().contains("groupes"));
    }
}
