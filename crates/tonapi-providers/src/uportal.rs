//! uPortal : applications et profil de l'étudiant, lus dans le portail déclaré dans `[portal]`.
//!
//! Deux variantes, choisies par la configuration de l'université :
//!
//! | Variante | Applications | Profil |
//! |---|---|---|
//! | `uportal4-prolongation` | `layout.jsp` (JSONP d'ESUP ProlongationENT) | `api/session.json` puis `api/people/<id>.json` |
//! | `uportal5` | `api/v4-3/dlm/layout.json` | `api/v5-1/userinfo` (JWT), sinon comme uPortal 4 |
//!
//! Le portail ne signale pas toujours une session perdue par une erreur : il redirige vers
//! `/Login`, ou répond 200 avec un compte invité (`guest`, `authenticated: "false"`). Les deux cas
//! donnent `session_expired` ; le fournisseur rouvre alors le portail par le CAS (TGC de la
//! session) et réessaie **une seule fois**.
//!
//! Sources : `Docs/Montpellier/ent.md` (uPortal 4.3.1, confirmé avec un compte réel),
//! `Docs/Rennes/apis/uportal5.md` (uPortal 5 : seule la réponse invitée a été observée, la forme
//! d'une session connectée est lue dans le code d'un client tiers).

use std::collections::HashMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode};
use serde_json::Value;
use tonapi_config::PortalConfig;
use tonapi_core::{App, ErrorCode, Problem, Profile};
use url::Url;

use crate::Context;
use crate::net::{Net, upstream};

/// Nom du logiciel dans les messages d'erreur.
const PROVIDER: &str = "uportal";

/// Identifiant du compte invité d'uPortal.
const GUEST: &str = "guest";

/// Dossiers techniques d'uPortal : `Column` n'est pas un titre de section.
const TECHNICAL_FOLDERS: [&str; 1] = ["column"];

/// Extensions d'une icône, par opposition à une page d'application.
const IMAGE_EXTENSIONS: [&str; 7] = [".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".webp"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variant {
    /// uPortal 4 avec ESUP ProlongationENT.
    Prolongation,
    /// uPortal 5.
    Uportal5,
}

/// Portail configuré pour l'université de l'étudiant.
struct Portal {
    variant: Variant,
    /// Racine du portail, avec une barre finale.
    base: Url,
    /// Identifiant de connexion de l'étudiant, si le portail n'en donne pas.
    account: String,
}

fn portal(ctx: &Context<'_>) -> Result<Portal, Problem> {
    let (variant, base_url) = match &ctx.university.university.portal {
        PortalConfig::Uportal4Prolongation { base_url } => (Variant::Prolongation, base_url),
        PortalConfig::Uportal5 { base_url } => (Variant::Uportal5, base_url),
        PortalConfig::TilesPortal { .. } | PortalConfig::None => {
            return Err(Problem::new(ErrorCode::ServiceAbsent)
                .detail("aucun portail n'est déclaré pour cette université"));
        }
    };
    Ok(Portal {
        variant,
        base: with_trailing_slash(base_url),
        account: ctx.session.username.clone(),
    })
}

fn with_trailing_slash(base: &Url) -> Url {
    let mut base = base.clone();
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base.set_query(None);
    base.set_fragment(None);
    base
}

// ---------------------------------------------------------------------------------------------
// Récupération
// ---------------------------------------------------------------------------------------------

/// Applications de l'étudiant, lues dans le layout du portail.
pub(crate) async fn fetch_apps(ctx: &mut Context<'_>) -> Result<Vec<App>, Problem> {
    with_portal(ctx, async |net: &mut Net, portal: &Portal| {
        apps_once(net, portal).await
    })
    .await
}

/// Profil de l'étudiant, lu dans le portail.
pub(crate) async fn fetch_profile(ctx: &mut Context<'_>) -> Result<Profile, Problem> {
    with_portal(ctx, async |net: &mut Net, portal: &Portal| {
        profile_once(net, portal).await
    })
    .await
}

/// Ouvre le réseau, exécute `attempt` avec la session mémorisée, et la rouvre une fois si le
/// portail dit qu'elle est perdue. Les cookies repartent toujours dans la session de l'étudiant.
async fn with_portal<T>(
    ctx: &mut Context<'_>,
    mut attempt: impl AsyncFnMut(&mut Net, &Portal) -> Result<T, Problem>,
) -> Result<T, Problem> {
    let portal = portal(ctx)?;
    let mut net = Net::open(ctx, PROVIDER)?;
    let entry = entry_url(&net, &portal.base);
    let result = async {
        match attempt(&mut net, &portal).await {
            Err(problem) if problem.code == ErrorCode::SessionExpired => {
                // Le CAS redemande le mot de passe : `open_protected` rend `session_expired`
                // sans nouvel essai.
                net.open_protected(entry).await?;
                attempt(&mut net, &portal).await
            }
            other => other,
        }
    }
    .await;
    net.save(ctx);
    result
}

/// Page qui ouvre une session portail : le CAS avec le service `<portail>/Login`, comme le bouton
/// « Connexion » ; la racine d'un uPortal peut afficher une page invitée au lieu de rediriger.
fn entry_url(net: &Net, base: &Url) -> Url {
    let login = base.join("Login").unwrap_or_else(|_| base.clone());
    match &net.cas {
        Some(cas) => cas.login_url(Some(&login)),
        None => login,
    }
}

async fn apps_once(net: &mut Net, portal: &Portal) -> Result<Vec<App>, Problem> {
    match portal.variant {
        Variant::Prolongation => {
            let url = route(&portal.base, "layout.jsp?callback=x")?;
            let body = get(
                net,
                url,
                "application/javascript, application/json, */*",
                &portal.base,
            )
            .await?;
            parse_prolongation_layout(&body, &portal.base)
        }
        Variant::Uportal5 => {
            let url = route(&portal.base, "api/v4-3/dlm/layout.json")?;
            let body = get(net, url, "application/json", &portal.base).await?;
            parse_uportal5_layout(&body, &portal.base)
        }
    }
}

async fn profile_once(net: &mut Net, portal: &Portal) -> Result<Profile, Problem> {
    match portal.variant {
        Variant::Prolongation => profile_people(net, portal).await,
        Variant::Uportal5 => {
            let url = route(&portal.base, "api/v5-1/userinfo")?;
            let (status, body) = fetch(
                net,
                url,
                "application/jwt, application/json, */*",
                &portal.base,
            )
            .await?;
            // Certains uPortal servent le layout v4-3 sans `userinfo` (404) mais gardent
            // `session.json` et `people` : on les lit à la place. [vu] Paul-Valéry, 27/10/2026.
            if status == StatusCode::NOT_FOUND {
                return profile_people(net, portal).await;
            }
            check_status(status)?;
            parse_userinfo(&body, &portal.account)
        }
    }
}

/// Profil par `api/session.json` puis `api/people/<identifiant>.json`.
async fn profile_people(net: &mut Net, portal: &Portal) -> Result<Profile, Problem> {
    let url = route(&portal.base, "api/session.json")?;
    let body = get(net, url, "application/json", &portal.base).await?;
    let session = parse_session(&body)?;
    // Le prénom, le nom et l'e-mail dépendent d'une permission du portail : un refus, une
    // erreur ou un portail muet laissent le profil réduit à l'identifiant et au nom
    // affiché. Seule une redirection vers `/Login` est une session perdue.
    let mut people = None;
    if let Some(url) = people_url(&portal.base, &session.username) {
        match fetch(net, url, "application/json", &portal.base).await {
            Ok((status, _)) if status.is_redirection() => return Err(lost()),
            Ok((status, body)) if status.is_success() => people = Some(body),
            Ok(_) | Err(_) => {}
        }
    }
    Ok(parse_profile_v4(&session, people.as_deref()))
}

fn route(base: &Url, path: &str) -> Result<Url, Problem> {
    base.join(path)
        .map_err(|_| Problem::new(ErrorCode::Internal).detail("adresse du portail illisible"))
}

/// `api/people/<identifiant>.json`, l'identifiant étant encodé comme un segment de chemin.
fn people_url(base: &Url, username: &str) -> Option<Url> {
    let mut url = route(base, "api/people/").ok()?;
    url.path_segments_mut()
        .ok()?
        .pop_if_empty()
        .push(&format!("{username}.json"));
    Some(url)
}

/// `GET` sans suivre de redirection : statut et corps.
async fn fetch(
    net: &mut Net,
    url: Url,
    accept: &'static str,
    referer: &Url,
) -> Result<(StatusCode, String), Problem> {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static(accept));
    if let Ok(referer) = HeaderValue::from_str(referer.as_str()) {
        headers.insert(header::REFERER, referer);
    }
    let (status, _, body) = net.request(Method::GET, url, headers, None).await?;
    Ok((status, body))
}

/// Corps d'une réponse 200 ; les autres statuts deviennent un problème (voir [`check_status`]).
async fn get(
    net: &mut Net,
    url: Url,
    accept: &'static str,
    referer: &Url,
) -> Result<String, Problem> {
    let (status, body) = fetch(net, url, accept, referer).await?;
    check_status(status)?;
    Ok(body)
}

/// Session perdue : le portail renvoie vers `/Login` ou refuse l'accès.
fn lost() -> Problem {
    Problem::new(ErrorCode::SessionExpired)
        .detail("le portail n'a pas de session ouverte pour ce compte")
}

fn check_status(status: StatusCode) -> Result<(), Problem> {
    if status.is_success() {
        Ok(())
    } else if status.is_redirection()
        || status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
    {
        Err(lost())
    } else if status.is_server_error() {
        Err(Problem::new(ErrorCode::ServiceUnavailable).detail(format!(
            "{PROVIDER} : le portail répond HTTP {}",
            status.as_u16()
        )))
    } else {
        Err(upstream(
            PROVIDER,
            format!("réponse HTTP {} inattendue", status.as_u16()),
        ))
    }
}

// ---------------------------------------------------------------------------------------------
// Aides communes à l'analyse
// ---------------------------------------------------------------------------------------------

/// Chaîne non vide (après retrait des blancs) du champ `key`.
fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Première chaîne non vide d'une valeur qui est soit une chaîne, soit un tableau de chaînes
/// (attributs d'annuaire, claims multivalués).
fn first_text(value: Option<&Value>) -> Option<&str> {
    match value? {
        Value::String(s) => Some(s.trim()).filter(|s| !s.is_empty()),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .find(|s| !s.is_empty()),
        _ => None,
    }
}

/// uPortal écrit ses booléens en chaînes (`"true"`) ; un vrai booléen est accepté aussi.
fn flag(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(b) => Some(*b),
        Value::String(s) if s.trim().eq_ignore_ascii_case("true") => Some(true),
        Value::String(s) if s.trim().eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

fn is_guest(user: &str) -> bool {
    user.trim().eq_ignore_ascii_case(GUEST)
}

/// URL absolue (http ou https) d'une adresse du portail, relative ou non.
fn resolve(base: &Url, raw: &str) -> Option<Url> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let url = base.join(raw).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(url)
}

/// Page du portail qui ouvre le portlet `fname` (`<portail>/p/<fname>`).
fn portal_page(base: &Url, fname: &str) -> Option<Url> {
    let mut url = base.clone();
    url.path_segments_mut()
        .ok()?
        .pop_if_empty()
        .push("p")
        .push(fname);
    Some(url)
}

fn is_image(url: &Url) -> bool {
    let path = url.path().to_ascii_lowercase();
    IMAGE_EXTENSIONS.iter().any(|ext| path.ends_with(ext)) || path.contains("/media/skins/icons/")
}

/// Applications sans doublon, dans l'ordre de première apparition. Une application présente dans
/// plusieurs dossiers garde le premier, complété par les champs que les autres renseignent.
#[derive(Default)]
struct Collected {
    apps: Vec<App>,
    index: HashMap<String, usize>,
}

impl Collected {
    fn add(&mut self, app: App) {
        if let Some(existing) = self.index.get(&app.id).and_then(|&i| self.apps.get_mut(i)) {
            existing.description = existing.description.take().or(app.description);
            existing.icon_url = existing.icon_url.take().or(app.icon_url);
            existing.category = existing.category.take().or(app.category);
        } else {
            self.index.insert(app.id.clone(), self.apps.len());
            self.apps.push(app);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// uPortal 4 + ProlongationENT
// ---------------------------------------------------------------------------------------------

/// Retire l'enveloppe JSONP : le texte entre la première `(` et la dernière `)`. Un JSON nu est
/// rendu tel quel.
fn strip_jsonp(body: &str) -> &str {
    let body = body.trim();
    if body.starts_with('{') {
        return body;
    }
    match (body.find('('), body.rfind(')')) {
        (Some(start), Some(end)) if start < end => body.get(start + 1..end).unwrap_or(body),
        _ => body,
    }
}

/// Applications de `layout.jsp`, sans doublon (une même application revient dans plusieurs
/// dossiers). `iconUrl` y contient en réalité l'adresse de l'application ; `url` est la page uPortal
/// du portlet, qui sert de repli (et `iconUrl` redevient une icône quand c'est une image).
pub(crate) fn parse_prolongation_layout(body: &str, base: &Url) -> Result<Vec<App>, Problem> {
    let root: Value = serde_json::from_str(strip_jsonp(body))
        .map_err(|_| upstream(PROVIDER, "layout.jsp n'est ni du JSON ni du JSONP"))?;
    // Sans `user`, ou avec l'invité : pas de session portail.
    if text(&root, "user").is_none_or(is_guest) {
        return Err(lost());
    }
    let folders = root
        .pointer("/layout/folders")
        .and_then(Value::as_array)
        .ok_or_else(|| upstream(PROVIDER, "`layout.folders` absent de layout.jsp"))?;

    let mut collected = Collected::default();
    for folder in folders {
        let category = text(folder, "title");
        let portlets = folder.get("portlets").and_then(Value::as_array);
        for portlet in portlets.into_iter().flatten() {
            if let Some(app) = prolongation_app(portlet, category, base) {
                collected.add(app);
            }
        }
    }
    Ok(collected.apps)
}

/// Un portlet sans `fname` n'a pas d'identifiant utilisable : il est ignoré.
fn prolongation_app(portlet: &Value, category: Option<&str>, base: &Url) -> Option<App> {
    let fname = text(portlet, "fname")?;
    let icon = text(portlet, "iconUrl").and_then(|raw| resolve(base, raw));
    let (target, icon) = match icon {
        Some(icon) if !is_image(&icon) => (Some(icon), None),
        icon => (
            text(portlet, "url").and_then(|raw| resolve(base, raw)),
            icon,
        ),
    };
    let url = target.or_else(|| portal_page(base, fname))?;
    Some(App {
        id: fname.to_owned(),
        title: text(portlet, "title").unwrap_or(fname).to_owned(),
        description: text(portlet, "description").map(str::to_owned),
        url: url.into(),
        icon_url: icon.map(Into::into),
        category: category.map(str::to_owned),
    })
}

/// Identité lue dans `api/session.json`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct V4Session {
    pub username: String,
    pub display_name: Option<String>,
}

/// `{"person":{"userName","displayName",…}}`. L'invité (`userName: "guest"`) est une session
/// perdue, pas un compte.
pub(crate) fn parse_session(body: &str) -> Result<V4Session, Problem> {
    let root: Value = serde_json::from_str(body.trim())
        .map_err(|_| upstream(PROVIDER, "session.json n'est pas du JSON"))?;
    let person = root
        .get("person")
        .filter(|p| p.is_object())
        .ok_or_else(|| upstream(PROVIDER, "`person` absent de session.json"))?;
    let username = text(person, "userName")
        .ok_or_else(|| upstream(PROVIDER, "`person.userName` absent de session.json"))?;
    if is_guest(username) {
        return Err(lost());
    }
    Ok(V4Session {
        username: username.to_owned(),
        display_name: text(person, "displayName").map(str::to_owned),
    })
}

/// Profil d'après `session.json` et, quand le portail le permet, `people/<id>.json` : chaque
/// attribut y est un tableau. Une réponse `people` absente, `{"person":null}` ou illisible laisse
/// le profil réduit à l'identifiant et au nom affiché.
pub(crate) fn parse_profile_v4(session: &V4Session, people: Option<&str>) -> Profile {
    let mut profile = Profile {
        username: session.username.clone(),
        display_name: session.display_name.clone(),
        first_name: None,
        last_name: None,
        email: None,
        student_number: None,
        program: None,
    };
    let people: Option<Value> = people.and_then(|body| serde_json::from_str(body.trim()).ok());
    if let Some(attributes) = people
        .as_ref()
        .and_then(|p| p.pointer("/person/attributes"))
    {
        let attribute = |name: &str| first_text(attributes.get(name)).map(str::to_owned);
        profile.first_name = attribute("givenName");
        profile.last_name = attribute("sn");
        profile.email = attribute("mail");
        profile.student_number = attribute("supannEtuId");
        profile.display_name = profile
            .display_name
            .take()
            .or_else(|| attribute("displayName"));
    }
    complete_display_name(&mut profile);
    profile
}

/// Sans nom affiché, « prénom nom » quand le portail donne les deux.
fn complete_display_name(profile: &mut Profile) {
    if profile.display_name.is_none()
        && let (Some(first), Some(last)) = (&profile.first_name, &profile.last_name)
    {
        profile.display_name = Some(format!("{first} {last}"));
    }
}

// ---------------------------------------------------------------------------------------------
// uPortal 5
// ---------------------------------------------------------------------------------------------

/// Applications de `api/v4-3/dlm/layout.json` : les portlets des onglets (et de leurs dossiers),
/// sans doublon. La section « Favoris » est lue en dernier pour ne pas passer devant le reste.
///
/// Le lien de lancement est, dans l'ordre : un paramètre du portlet (`alternativeMaximizedLink`,
/// `launchUrl`, `url`), sinon la page du portail `<portail>/p/<fname>`. Le vrai lien de beaucoup de
/// portlets n'est que dans leur fragment HTML, qu'on ne lit pas ici.
pub(crate) fn parse_uportal5_layout(body: &str, base: &Url) -> Result<Vec<App>, Problem> {
    let root: Value = serde_json::from_str(body.trim())
        .map_err(|_| upstream(PROVIDER, "layout.json n'est pas du JSON"))?;
    match flag(root.get("authenticated")) {
        Some(true) => {}
        Some(false) => return Err(lost()),
        None => return Err(upstream(PROVIDER, "`authenticated` absent de layout.json")),
    }
    if text(&root, "user").is_some_and(is_guest) {
        return Err(lost());
    }
    let tabs = root
        .pointer("/layout/navigation/tabs")
        .and_then(Value::as_array)
        .ok_or_else(|| upstream(PROVIDER, "`layout.navigation.tabs` absent de layout.json"))?;

    let mut walk = Walk {
        base,
        regular: Vec::new(),
        favorites: Vec::new(),
    };
    for tab in tabs.iter().filter(|t| flag(t.get("hidden")) != Some(true)) {
        let name = text(tab, "name");
        let content = tab.get("content").and_then(Value::as_array);
        walk.nodes(content.map_or(&[], Vec::as_slice), name, None, false);
    }
    if let Some(favorites) = root.pointer("/layout/favorites").and_then(Value::as_array) {
        walk.nodes(favorites, None, None, true);
    }

    let mut collected = Collected::default();
    for app in walk.regular.into_iter().chain(walk.favorites) {
        collected.add(app);
    }
    Ok(collected.apps)
}

/// Parcours de l'arbre onglet → dossiers → portlets.
struct Walk<'a> {
    base: &'a Url,
    regular: Vec<App>,
    favorites: Vec<App>,
}

impl Walk<'_> {
    fn nodes(&mut self, nodes: &[Value], tab: Option<&str>, section: Option<&str>, favorite: bool) {
        for node in nodes {
            if flag(node.get("hidden")) == Some(true) {
                continue;
            }
            let content = node.get("content").and_then(Value::as_array);
            let is_portlet = match text(node, "_objectType") {
                Some(kind) => kind == "portlet",
                None => text(node, "fname").is_some(),
            };
            if is_portlet {
                // Dans « Favoris », le dossier n'est pas une catégorie : on garde l'onglet.
                let category = if favorite { tab } else { section.or(tab) };
                if let Some(app) = uportal5_app(node, category, self.base) {
                    if favorite {
                        self.favorites.push(app);
                    } else {
                        self.regular.push(app);
                    }
                }
            } else if let Some(content) = content {
                let name = text(node, "name");
                let titled = name.filter(|n| !is_technical_folder(n));
                let in_favorites = favorite || name.is_some_and(is_favorites_section);
                self.nodes(content, tab, titled.or(section), in_favorites);
            }
        }
    }
}

fn is_technical_folder(name: &str) -> bool {
    let name = name.to_lowercase();
    TECHNICAL_FOLDERS.iter().any(|t| name.starts_with(t))
}

fn is_favorites_section(name: &str) -> bool {
    let name = name.to_lowercase();
    ["favori", "favorite", "bookmark"]
        .iter()
        .any(|word| name.contains(word))
}

fn uportal5_app(node: &Value, category: Option<&str>, base: &Url) -> Option<App> {
    let fname = text(node, "fname")?;
    let parameters = node.get("parameters");
    let target = ["alternativeMaximizedLink", "launchUrl", "url"]
        .iter()
        .find_map(|key| {
            let raw = text(parameters?, key)?;
            resolve(base, raw)
        });
    let url = target.or_else(|| portal_page(base, fname))?;
    Some(App {
        id: fname.to_owned(),
        title: text(node, "title")
            .or_else(|| text(node, "name"))
            .unwrap_or(fname)
            .to_owned(),
        description: text(node, "description").map(str::to_owned),
        url: url.into(),
        icon_url: text(node, "iconUrl")
            .and_then(|raw| resolve(base, raw))
            .map(Into::into),
        category: category.map(str::to_owned),
    })
}

/// Profil d'après `api/v5-1/userinfo` : un JWT dont on lit la charge utile (le portail est joint
/// directement en TLS, la signature n'ajoute rien ici) ; du JSON nu est accepté aussi. Le JWT de
/// l'invité (`sub: "guest"`) est une session perdue. `account` est l'identifiant de connexion,
/// utilisé si le jeton ne porte ni `preferred_username` ni `sub`.
pub(crate) fn parse_userinfo(body: &str, account: &str) -> Result<Profile, Problem> {
    let claims = userinfo_claims(body)?;
    let sub = text(&claims, "sub");
    let preferred = text(&claims, "preferred_username");
    if [sub, preferred].into_iter().flatten().any(is_guest) {
        return Err(lost());
    }
    let username = preferred
        .or(sub)
        .or_else(|| Some(account.trim()).filter(|a| !a.is_empty()))
        .ok_or_else(|| upstream(PROVIDER, "aucun identifiant dans userinfo"))?;
    let claim = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| first_text(claims.get(*name)))
            .map(str::to_owned)
    };
    let mut profile = Profile {
        username: username.to_owned(),
        display_name: claim(&["name", "display_name"]),
        first_name: claim(&["given_name"]),
        last_name: claim(&["family_name"]),
        email: claim(&["email", "mail"]),
        student_number: claim(&["supannEtuId", "studentNumber"]),
        program: None,
    };
    complete_display_name(&mut profile);
    Ok(profile)
}

fn userinfo_claims(body: &str) -> Result<Value, Problem> {
    let body = body.trim();
    let bytes = if body.starts_with('{') {
        body.as_bytes().to_vec()
    } else {
        let payload = body
            .split('.')
            .nth(1)
            .ok_or_else(|| upstream(PROVIDER, "userinfo n'est ni un JWT ni du JSON"))?;
        URL_SAFE_NO_PAD
            .decode(payload.trim_end_matches('='))
            .map_err(|_| upstream(PROVIDER, "charge utile du JWT userinfo illisible"))?
    };
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| upstream(PROVIDER, "claims userinfo illisibles"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYOUT_JSONP: &str = include_str!("../tests/fixtures/uportal/layout_prolongation.jsonp");
    const LAYOUT_V5: &str = include_str!("../tests/fixtures/uportal/layout_v5.json");
    const LAYOUT_V5_GUEST: &str = include_str!("../tests/fixtures/uportal/layout_v5_invite.json");
    const SESSION_V4: &str = include_str!("../tests/fixtures/uportal/session_v4.json");
    const SESSION_V4_GUEST: &str = include_str!("../tests/fixtures/uportal/session_v4_invite.json");
    const PEOPLE_V4: &str = include_str!("../tests/fixtures/uportal/people_v4.json");
    const USERINFO: &str = include_str!("../tests/fixtures/uportal/userinfo_v5.jwt");
    const USERINFO_GUEST: &str = include_str!("../tests/fixtures/uportal/userinfo_v5_invite.jwt");

    fn base4() -> Url {
        Url::parse("https://ent.exemple.test/uPortal/").unwrap()
    }

    fn base5() -> Url {
        Url::parse("https://services.exemple.test/").unwrap()
    }

    fn app<'a>(apps: &'a [App], id: &str) -> &'a App {
        apps.iter()
            .find(|a| a.id == id)
            .unwrap_or_else(|| panic!("application `{id}` absente"))
    }

    fn ids(apps: &[App]) -> Vec<&str> {
        apps.iter().map(|a| a.id.as_str()).collect()
    }

    fn code(result: Result<impl std::fmt::Debug, Problem>) -> ErrorCode {
        result.unwrap_err().code
    }

    // --- layout.jsp ---------------------------------------------------------------------

    #[test]
    fn layout_jsp_liste_sans_doublon_dans_l_ordre() {
        let apps = parse_prolongation_layout(LAYOUT_JSONP, &base4()).unwrap();
        assert_eq!(
            ids(&apps),
            [
                "compte-demo",
                "moodle-demo",
                "bibliotheque",
                "planning-demo",
                "sans-titre"
            ]
        );
    }

    #[test]
    fn layout_jsp_icon_url_donne_l_adresse_de_l_application() {
        let apps = parse_prolongation_layout(LAYOUT_JSONP, &base4()).unwrap();
        let compte = app(&apps, "compte-demo");
        assert_eq!(compte.title, "Compte informatique");
        assert_eq!(
            compte.url,
            "https://apps.exemple.test/compte/protected/authenticated"
        );
        assert_eq!(compte.icon_url, None);
        assert_eq!(
            compte.description.as_deref(),
            Some("Mot de passe, redirection du courrier")
        );
        assert_eq!(compte.category.as_deref(), Some("Accueil"));
    }

    #[test]
    fn layout_jsp_resout_les_urls_relatives_et_garde_l_icone() {
        let apps = parse_prolongation_layout(LAYOUT_JSONP, &base4()).unwrap();
        let biblio = app(&apps, "bibliotheque");
        // `iconUrl` est une image : l'adresse de l'application est la page uPortal du portlet.
        assert_eq!(
            biblio.url,
            "https://ent.exemple.test/uPortal/f/u1l1s1/p/bibliotheque.u1l1n3/detached/render.uP?pCm=view"
        );
        assert_eq!(
            biblio.icon_url.as_deref(),
            Some("https://ent.exemple.test/uPortal/media/skins/icons/mobile/bibliotheque.png")
        );
        // `iconUrl` vide : repli sur la page uPortal.
        let planning = app(&apps, "planning-demo");
        assert!(
            planning
                .url
                .starts_with("https://ent.exemple.test/uPortal/f/u1l1s2/p/planning-demo.")
        );
        assert_eq!(planning.icon_url, None);
    }

    #[test]
    fn layout_jsp_champs_facultatifs_absents() {
        let apps = parse_prolongation_layout(LAYOUT_JSONP, &base4()).unwrap();
        let moodle = app(&apps, "moodle-demo");
        // Description vide, dossier « Accueil » (premier) conservé.
        assert_eq!(moodle.description, None);
        assert_eq!(moodle.category.as_deref(), Some("Accueil"));
        // Sans titre : le `fname` en tient lieu.
        assert_eq!(app(&apps, "sans-titre").title, "sans-titre");
    }

    #[test]
    fn layout_jsp_ignore_une_entree_sans_fname() {
        let apps = parse_prolongation_layout(LAYOUT_JSONP, &base4()).unwrap();
        assert!(apps.iter().all(|a| a.title != "Entrée sans identifiant"));
    }

    #[test]
    fn layout_jsp_racine_sans_barre_finale() {
        let base = with_trailing_slash(&Url::parse("https://ent.exemple.test/uPortal").unwrap());
        let apps = parse_prolongation_layout(LAYOUT_JSONP, &base).unwrap();
        assert!(
            app(&apps, "bibliotheque")
                .url
                .starts_with("https://ent.exemple.test/uPortal/f/")
        );
    }

    #[test]
    fn layout_jsp_json_nu_et_jsonp_avec_point_virgule() {
        let nu = r#"{"user":"e00000000000","layout":{"folders":[{"title":"A","portlets":[{"fname":"x","title":"X","url":"/uPortal/p/x"}]}]}}"#;
        assert_eq!(parse_prolongation_layout(nu, &base4()).unwrap().len(), 1);
        let jsonp = format!("cb({nu});\n");
        let apps = parse_prolongation_layout(&jsonp, &base4()).unwrap();
        assert_eq!(apps[0].url, "https://ent.exemple.test/uPortal/p/x");
    }

    #[test]
    fn layout_jsp_sans_utilisateur_ou_invite_est_une_session_perdue() {
        let sans_user = r#"cb({"layout":{"folders":[]}})"#;
        assert_eq!(
            code(parse_prolongation_layout(sans_user, &base4())),
            ErrorCode::SessionExpired
        );
        let invite = r#"cb({"user":"guest","layout":{"folders":[]}})"#;
        assert_eq!(
            code(parse_prolongation_layout(invite, &base4())),
            ErrorCode::SessionExpired
        );
    }

    #[test]
    fn layout_jsp_format_inattendu_sans_donnee_dans_l_erreur() {
        let sans_dossiers = r#"cb({"user":"e00000000000","layout":{}})"#;
        let err = parse_prolongation_layout(sans_dossiers, &base4()).unwrap_err();
        assert_eq!(err.code, ErrorCode::UpstreamChanged);
        assert!(!err.to_string().contains("e00000000000"));

        let html = "<html>e00000000000</html>";
        let err = parse_prolongation_layout(html, &base4()).unwrap_err();
        assert_eq!(err.code, ErrorCode::UpstreamChanged);
        assert!(!err.to_string().contains("e00000000000"));
    }

    #[test]
    fn layout_jsp_dossier_vide_donne_une_liste_vide() {
        let vide = r#"cb({"user":"e00000000000","layout":{"folders":[{"title":"A"}]}})"#;
        assert!(
            parse_prolongation_layout(vide, &base4())
                .unwrap()
                .is_empty()
        );
    }

    // --- layout.json (uPortal 5) ------------------------------------------------------------

    #[test]
    fn layout_v5_liste_sans_doublon_favoris_en_dernier() {
        let apps = parse_uportal5_layout(LAYOUT_V5, &base5()).unwrap();
        assert_eq!(
            ids(&apps),
            [
                "webapp-planning",
                "webapp-courrier",
                "webapp-dossier",
                "webapp-assistance"
            ]
        );
    }

    #[test]
    fn layout_v5_liens_et_champs() {
        let apps = parse_uportal5_layout(LAYOUT_V5, &base5()).unwrap();
        let planning = app(&apps, "webapp-planning");
        assert_eq!(planning.title, "Mon emploi du temps");
        assert_eq!(planning.url, "https://planning.exemple.test/portal/");
        assert_eq!(
            planning.icon_url.as_deref(),
            Some("https://planning.exemple.test/icone.png")
        );
        assert_eq!(planning.category.as_deref(), Some("Scolarité"));

        // `parameters.url` relatif : résolu sur le portail.
        assert_eq!(
            app(&apps, "webapp-dossier").url,
            "https://services.exemple.test/mdw/"
        );

        // Aucun paramètre : page du portail pour ce portlet.
        let assistance = app(&apps, "webapp-assistance");
        assert_eq!(
            assistance.url,
            "https://services.exemple.test/p/webapp-assistance"
        );
        // Le dossier technique « Column » n'est pas une catégorie : l'onglet la remplace.
        assert_eq!(assistance.category.as_deref(), Some("Aide"));
        assert_eq!(assistance.description, None);
        assert_eq!(assistance.icon_url, None);
    }

    #[test]
    fn layout_v5_un_doublon_complete_le_premier() {
        let apps = parse_uportal5_layout(LAYOUT_V5, &base5()).unwrap();
        let courrier = app(&apps, "webapp-courrier");
        // Vu d'abord dans « Scolarité » (sans description ni icône), puis dans « Favoris ».
        assert_eq!(courrier.category.as_deref(), Some("Scolarité"));
        assert_eq!(courrier.description.as_deref(), Some("Boîte aux lettres"));
        assert_eq!(
            courrier.icon_url.as_deref(),
            Some("https://services.exemple.test/images/icons/courrier.svg")
        );
        assert_eq!(courrier.url, "https://courrier.exemple.test/");
    }

    #[test]
    fn layout_v5_ignore_les_portlets_masques_et_les_regions() {
        let apps = parse_uportal5_layout(LAYOUT_V5, &base5()).unwrap();
        assert!(
            apps.iter()
                .all(|a| a.id != "webapp-cache" && a.id != "portal-brand")
        );
    }

    #[test]
    fn layout_v5_favori_seul_garde_l_onglet_comme_categorie() {
        let body = r#"{"user":"e1","authenticated":"true","layout":{"navigation":{"tabs":[
            {"name":"Services","content":[{"_objectType":"folder","name":"Favoris","content":[
              {"_objectType":"portlet","fname":"seul","title":"Seul"}]}]}]}}}"#;
        let apps = parse_uportal5_layout(body, &base5()).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].category.as_deref(), Some("Services"));
    }

    #[test]
    fn layout_v5_invite_est_une_session_perdue() {
        assert_eq!(
            code(parse_uportal5_layout(LAYOUT_V5_GUEST, &base5())),
            ErrorCode::SessionExpired
        );
        let invite_sans_drapeau =
            r#"{"user":"guest","authenticated":true,"layout":{"navigation":{"tabs":[]}}}"#;
        assert_eq!(
            code(parse_uportal5_layout(invite_sans_drapeau, &base5())),
            ErrorCode::SessionExpired
        );
    }

    #[test]
    fn layout_v5_accepte_un_vrai_booleen() {
        let body = r#"{"user":"e1","authenticated":true,"layout":{"navigation":{"tabs":[]}}}"#;
        assert!(parse_uportal5_layout(body, &base5()).unwrap().is_empty());
    }

    #[test]
    fn layout_v5_champs_obligatoires_absents() {
        let sans_drapeau = r#"{"user":"e1","layout":{"navigation":{"tabs":[]}}}"#;
        assert_eq!(
            code(parse_uportal5_layout(sans_drapeau, &base5())),
            ErrorCode::UpstreamChanged
        );
        let sans_onglets = r#"{"user":"e1","authenticated":"true","layout":{}}"#;
        assert_eq!(
            code(parse_uportal5_layout(sans_onglets, &base5())),
            ErrorCode::UpstreamChanged
        );
        assert_eq!(
            code(parse_uportal5_layout("<html>", &base5())),
            ErrorCode::UpstreamChanged
        );
    }

    // --- profil uPortal 4 --------------------------------------------------------------------

    #[test]
    fn profil_v4_complet() {
        let session = parse_session(SESSION_V4).unwrap();
        let profile = parse_profile_v4(&session, Some(PEOPLE_V4));
        assert_eq!(
            profile,
            Profile {
                username: "e00000000000".into(),
                display_name: Some("Camille Demo".into()),
                first_name: Some("Camille".into()),
                last_name: Some("Demo".into()),
                email: Some("camille.demo@etu.exemple.test".into()),
                student_number: None,
                program: None,
            }
        );
    }

    #[test]
    fn profil_v4_sans_people_ou_person_nul_reste_valide() {
        let session = parse_session(SESSION_V4).unwrap();
        for people in [None, Some(r#"{"person":null}"#), Some("pas du json")] {
            let profile = parse_profile_v4(&session, people);
            assert_eq!(profile.username, "e00000000000");
            assert_eq!(profile.display_name.as_deref(), Some("Camille Demo"));
            assert_eq!(profile.first_name, None);
            assert_eq!(profile.last_name, None);
            assert_eq!(profile.email, None);
            assert_eq!(profile.student_number, None);
        }
    }

    #[test]
    fn profil_v4_nom_affiche_deduit_du_prenom_et_du_nom() {
        let session = V4Session {
            username: "e00000000000".into(),
            display_name: None,
        };
        let people = r#"{"person":{"attributes":{"givenName":["Camille"],"sn":["Demo"],"mail":[],"supannEtuId":["00000000"]}}}"#;
        let profile = parse_profile_v4(&session, Some(people));
        assert_eq!(profile.display_name.as_deref(), Some("Camille Demo"));
        assert_eq!(profile.email, None);
        assert_eq!(profile.student_number.as_deref(), Some("00000000"));
    }

    #[test]
    fn session_v4_invitee_ou_incomplete() {
        assert_eq!(
            code(parse_session(SESSION_V4_GUEST)),
            ErrorCode::SessionExpired
        );
        assert_eq!(
            code(parse_session(r#"{"person":{"displayName":"X"}}"#)),
            ErrorCode::UpstreamChanged
        );
        assert_eq!(code(parse_session("{}")), ErrorCode::UpstreamChanged);
        assert_eq!(code(parse_session("<html>")), ErrorCode::UpstreamChanged);
    }

    #[test]
    fn url_people_encode_l_identifiant() {
        let url = people_url(&base4(), "e00000000000").unwrap();
        assert_eq!(
            url.as_str(),
            "https://ent.exemple.test/uPortal/api/people/e00000000000.json"
        );
        let url = people_url(&base4(), "a b/../c").unwrap();
        assert_eq!(url.host_str(), Some("ent.exemple.test"));
        assert!(url.path().starts_with("/uPortal/api/people/"));
        assert!(!url.path().contains("/../"));
    }

    // --- profil uPortal 5 --------------------------------------------------------------------

    #[test]
    fn userinfo_jwt_complet() {
        let profile = parse_userinfo(USERINFO, "autre-compte").unwrap();
        assert_eq!(
            profile,
            Profile {
                username: "e00000000000".into(),
                display_name: Some("Camille Demo".into()),
                first_name: Some("Camille".into()),
                last_name: Some("Demo".into()),
                email: Some("camille.demo@etu.exemple.test".into()),
                student_number: Some("00000000".into()),
                program: None,
            }
        );
    }

    #[test]
    fn userinfo_json_nu_avec_champs_manquants() {
        let body = r#"{"sub":"e00000000000"}"#;
        let profile = parse_userinfo(body, "autre-compte").unwrap();
        assert_eq!(profile.username, "e00000000000");
        assert_eq!(profile.display_name, None);
        assert_eq!(profile.email, None);
        assert_eq!(profile.student_number, None);
    }

    #[test]
    fn userinfo_sans_identifiant_utilise_le_compte_de_connexion() {
        let body = r#"{"name":"Camille Demo","mail":"camille.demo@etu.exemple.test"}"#;
        let profile = parse_userinfo(body, "e00000000000").unwrap();
        assert_eq!(profile.username, "e00000000000");
        assert_eq!(
            profile.email.as_deref(),
            Some("camille.demo@etu.exemple.test")
        );
        assert_eq!(code(parse_userinfo("{}", " ")), ErrorCode::UpstreamChanged);
    }

    #[test]
    fn userinfo_invite_est_une_session_perdue() {
        assert_eq!(
            code(parse_userinfo(USERINFO_GUEST, "e00000000000")),
            ErrorCode::SessionExpired
        );
    }

    #[test]
    fn userinfo_illisible() {
        // Vide, ni JWT ni JSON, base64 invalide, charge utile qui n'est pas un objet.
        for body in ["", "pas un jwt", "a.!!!.c", "[]", "a.W10.c"] {
            assert_eq!(
                code(parse_userinfo(body, "e00000000000")),
                ErrorCode::UpstreamChanged,
                "{body}"
            );
        }
    }

    #[test]
    fn userinfo_base64_avec_remplissage() {
        // `{"sub":"x"}` encodé avec `=` final : accepté.
        let profile = parse_userinfo("a.eyJzdWIiOiJ4In0=.c", "compte").unwrap();
        assert_eq!(profile.username, "x");
    }

    // --- aides ---------------------------------------------------------------------------------

    #[test]
    fn statuts_http() {
        assert!(check_status(StatusCode::OK).is_ok());
        for status in [
            StatusCode::FOUND,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
        ] {
            assert_eq!(code(check_status(status)), ErrorCode::SessionExpired);
        }
        assert_eq!(
            code(check_status(StatusCode::BAD_GATEWAY)),
            ErrorCode::ServiceUnavailable
        );
        assert_eq!(
            code(check_status(StatusCode::NOT_FOUND)),
            ErrorCode::UpstreamChanged
        );
    }

    #[test]
    fn jsonp_et_adresses() {
        assert_eq!(strip_jsonp("cb({\"a\":(1)});"), "{\"a\":(1)}");
        assert_eq!(strip_jsonp("{\"a\":\"(\"}"), "{\"a\":\"(\"}");
        assert_eq!(strip_jsonp("sans parenthèses"), "sans parenthèses");
        assert!(resolve(&base4(), "javascript:alert(1)").is_none());
        assert!(resolve(&base4(), "   ").is_none());
        assert_eq!(
            resolve(&base4(), "/mdw/").unwrap().as_str(),
            "https://ent.exemple.test/mdw/"
        );
        let page = portal_page(&base5(), "a b").unwrap();
        assert_eq!(page.as_str(), "https://services.exemple.test/p/a%20b");
    }
}
