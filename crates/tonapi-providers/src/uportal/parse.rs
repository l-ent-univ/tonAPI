//! Analyse pure des réponses d'uPortal, sans réseau : fragments de portlet, uPortal 4 avec ESUP
//! ProlongationENT (`layout.jsp`, `api/session.json`, `api/people`) et uPortal 5 (`layout.json`,
//! `userinfo`).

use std::collections::HashMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use tonapi_core::{App, Problem, Profile};
use url::Url;

use super::{PROVIDER, lost};
use crate::common::str_field;
use crate::net::upstream;

/// Identifiant du compte invité d'uPortal.
const GUEST: &str = "guest";

/// Dossiers techniques d'uPortal : `Column` n'est pas un titre de section.
const TECHNICAL_FOLDERS: [&str; 1] = ["column"];

/// Extensions d'une icône, par opposition à une page d'application.
const IMAGE_EXTENSIONS: [&str; 7] = [".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".webp"];

/// Lien sans trace de l'étudiant : sans fragment, sans son identifiant, sans mot évoquant un
/// ticket, un jeton ou une session, et sans segment de chemin ni valeur de paramètre qui
/// ressemble à un numéro ou un jeton (flux iCal, numéro étudiant, lien de capacité).
///
/// Un refus ne coûte qu'une requête de plus : dans le doute, le lien n'est pas partagé.
pub(super) fn shareable(link: &str, account: &str) -> bool {
    let Ok(url) = Url::parse(link) else {
        return false;
    };
    if url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    // Paramètres courts seulement (`?v=1`) : une valeur longue peut être propre à l'étudiant.
    let personal_value = url
        .query_pairs()
        .any(|(_, value)| value.len() > 8 || token_like(&value));
    let personal_segment = url
        .path_segments()
        .into_iter()
        .flatten()
        .flat_map(|segment| segment.split(['.', ';', ',']))
        .any(token_like);
    if personal_value || personal_segment {
        return false;
    }
    let lower = link.to_ascii_lowercase();
    let personal = [
        "ticket", "token", "jeton", "session", "sid=", "key=", "auth",
    ]
    .iter()
    .any(|word| lower.contains(word));
    !personal && (account.is_empty() || !lower.contains(&account.to_ascii_lowercase()))
}

/// Numéro (4 chiffres ou plus), code hexadécimal ou UUID (6 caractères ou plus, avec un
/// chiffre), ou longue chaîne avec des chiffres : ce qui ressemble à un identifiant personnel.
fn token_like(part: &str) -> bool {
    let len = part.len();
    let digits = part.bytes().filter(u8::is_ascii_digit).count();
    (len >= 4 && digits == len)
        || (len >= 6 && digits > 0 && part.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
        || (len >= 16 && digits > 0)
}

/// Lien d'un fragment de portlet : la source de son `<iframe>`, sinon son premier lien `<a>`
/// absolu hors du portail. Les fragments d'erreur ne pointent que vers le portail : ignorés.
pub(crate) fn fragment_link(html: &str, base: &Url) -> Option<Url> {
    let lower = html.to_ascii_lowercase();
    // `tag` : le nom de la balise avec son chevron (`<a`). Ce qui le suit est un blanc, quel
    // qu'il soit : uPortal écrit `<a` puis un retour à la ligne avant les attributs. [vu] Rennes,
    // 26/11/2026.
    let attribute = |tag: &str, name: &str| -> Vec<String> {
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(at) = lower.get(from..).and_then(|rest| rest.find(tag)) {
            let start = from + at;
            let named = lower
                .as_bytes()
                .get(start + tag.len())
                .is_some_and(u8::is_ascii_whitespace);
            if !named {
                from = start + tag.len();
                continue;
            }
            let end = lower
                .get(start..)
                .and_then(|rest| rest.find('>'))
                .map_or(lower.len(), |e| start + e);
            if let Some(value) = html.get(start..end).and_then(|t| attr(t, name)) {
                out.push(value);
            }
            from = end;
        }
        out
    };
    let outside = |raw: &str| {
        let url = resolve(base, &raw.replace("&amp;", "&"))?;
        let inside = url.host_str() == base.host_str() && url.path().starts_with(base.path());
        (!inside).then_some(url)
    };
    attribute("<iframe", "src")
        .iter()
        .chain(attribute("<a", "href").iter())
        .find_map(|raw| outside(raw))
}

/// Valeur d'un attribut entre guillemets dans une balise ouvrante.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower.get(from..)?.find(name) {
        let start = from + at;
        from = start + name.len();
        let before_ok = start == 0
            || lower
                .as_bytes()
                .get(start - 1)
                .is_some_and(u8::is_ascii_whitespace);
        let rest = tag.get(from..)?.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let quote = rest.chars().next()?;
        if !before_ok || (quote != '"' && quote != '\'') {
            continue;
        }
        let value = rest.get(1..)?;
        return value.find(quote).map(|end| value[..end].to_owned());
    }
    None
}

// Aides communes à l'analyse

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
pub(super) fn resolve(base: &Url, raw: &str) -> Option<Url> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let url = base.join(raw).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(url)
}

/// Page du portail qui ouvre le portlet `fname` (`<portail>/p/<fname>`).
pub(super) fn portal_page(base: &Url, fname: &str) -> Option<Url> {
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
            existing.favorite |= app.favorite;
        } else {
            self.index.insert(app.id.clone(), self.apps.len());
            self.apps.push(app);
        }
    }
}

// uPortal 4 + ProlongationENT

/// Retire l'enveloppe JSONP : le texte entre la première `(` et la dernière `)`. Un JSON nu est
/// rendu tel quel.
pub(super) fn strip_jsonp(body: &str) -> &str {
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
    if str_field(&root, "user").is_none_or(is_guest) {
        return Err(lost());
    }
    let folders = root
        .pointer("/layout/folders")
        .and_then(Value::as_array)
        .ok_or_else(|| upstream(PROVIDER, "`layout.folders` absent de layout.jsp"))?;

    let mut collected = Collected::default();
    for folder in folders {
        let category = str_field(folder, "title");
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
    let fname = str_field(portlet, "fname")?;
    let icon = str_field(portlet, "iconUrl").and_then(|raw| resolve(base, raw));
    let (target, icon) = match icon {
        Some(icon) if !is_image(&icon) => (Some(icon), None),
        icon => (
            str_field(portlet, "url").and_then(|raw| resolve(base, raw)),
            icon,
        ),
    };
    let url = target.or_else(|| portal_page(base, fname))?;
    Some(App {
        id: fname.to_owned(),
        title: str_field(portlet, "title").unwrap_or(fname).to_owned(),
        description: str_field(portlet, "description").map(str::to_owned),
        url: url.into(),
        icon_url: icon.map(Into::into),
        category: category.map(str::to_owned),
        favorite: false,
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
    let username = str_field(person, "userName")
        .ok_or_else(|| upstream(PROVIDER, "`person.userName` absent de session.json"))?;
    if is_guest(username) {
        return Err(lost());
    }
    Ok(V4Session {
        username: username.to_owned(),
        display_name: str_field(person, "displayName").map(str::to_owned),
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

// uPortal 5

/// Applications de `api/v4-3/dlm/layout.json` : les portlets des onglets (et de leurs dossiers),
/// sans doublon. La section « Favoris » est lue en dernier pour ne pas passer devant le reste ;
/// ce qui s'y trouve est marqué `favorite`, où que l'application soit listée par ailleurs.
///
/// Le lien de lancement est, dans l'ordre : un paramètre du portlet (`alternativeMaximizedLink`,
/// `launchUrl`, `url`), sinon la page du portail `<portail>/p/<fname>`. Le vrai lien de beaucoup de
/// portlets n'est que dans leur fragment HTML, lu ensuite par [`direct_links`].
pub(crate) fn parse_uportal5_layout(body: &str, base: &Url) -> Result<Vec<App>, Problem> {
    let root: Value = serde_json::from_str(body.trim())
        .map_err(|_| upstream(PROVIDER, "layout.json n'est pas du JSON"))?;
    match flag(root.get("authenticated")) {
        Some(true) => {}
        Some(false) => return Err(lost()),
        None => return Err(upstream(PROVIDER, "`authenticated` absent de layout.json")),
    }
    if str_field(&root, "user").is_some_and(is_guest) {
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
        let name = str_field(tab, "name");
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
            let is_portlet = match str_field(node, "_objectType") {
                Some(kind) => kind == "portlet",
                None => str_field(node, "fname").is_some(),
            };
            if is_portlet {
                // Dans « Favoris », le dossier n'est pas une catégorie : on garde l'onglet.
                let category = if favorite { tab } else { section.or(tab) };
                if let Some(mut app) = uportal5_app(node, category, self.base) {
                    app.favorite = favorite;
                    if favorite {
                        self.favorites.push(app);
                    } else {
                        self.regular.push(app);
                    }
                }
            } else if let Some(content) = content {
                let name = str_field(node, "name");
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
    let fname = str_field(node, "fname")?;
    let parameters = node.get("parameters");
    let target = ["alternativeMaximizedLink", "launchUrl", "url"]
        .iter()
        .find_map(|key| {
            let raw = str_field(parameters?, key)?;
            resolve(base, raw)
        });
    let url = target.or_else(|| portal_page(base, fname))?;
    Some(App {
        id: fname.to_owned(),
        title: str_field(node, "title")
            .or_else(|| str_field(node, "name"))
            .unwrap_or(fname)
            .to_owned(),
        description: str_field(node, "description").map(str::to_owned),
        url: url.into(),
        icon_url: str_field(node, "iconUrl")
            .and_then(|raw| resolve(base, raw))
            .map(Into::into),
        category: category.map(str::to_owned),
        favorite: false,
    })
}

/// Profil d'après `api/v5-1/userinfo` : un JWT dont on lit la charge utile (le portail est joint
/// directement en TLS, la signature n'ajoute rien ici) ; du JSON nu est accepté aussi. Le JWT de
/// l'invité (`sub: "guest"`) est une session perdue. `account` est l'identifiant de connexion,
/// utilisé si le jeton ne porte ni `preferred_username` ni `sub`.
pub(crate) fn parse_userinfo(body: &str, account: &str) -> Result<Profile, Problem> {
    let claims = userinfo_claims(body)?;
    let sub = str_field(&claims, "sub");
    let preferred = str_field(&claims, "preferred_username");
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
