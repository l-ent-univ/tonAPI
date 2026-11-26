//! Fournisseur `portal` pour un portail à tuiles (`portal.kind = "tiles-portal"`).
//!
//! Le portail a son propre client CAS : `cas/login?next=/` renvoie au CAS de l'université, qui
//! revient sur `cas/callback`. Ensuite, deux routes JSON du même hôte :
//! - `api/cas-attributes` : attributs CAS de l'étudiant (profil, composante) ;
//! - `api/tiles` : applications proposées.
//!
//! Le format exact de ces réponses n'est pas documenté [inconnu] : l'analyse accepte les formes
//! usuelles (attributs en chaîne ou en liste, titres traduits `{fr, en}`) et la trace de
//! diagnostic (`RUST_LOG=tonapi_providers=debug`) décrit leur structure, jamais leurs valeurs.

use reqwest::Method;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use serde_json::{Map, Value};
use tonapi_config::{PortalConfig, Signal, SignalSource};
use tonapi_core::{App, ErrorCode, Problem, Profile};
use url::Url;

use crate::Context;
use crate::common::root;
use crate::net::{Net, upstream};

const PROVIDER: &str = "tiles-portal";

/// Attributs dont la valeur décrit l'affectation ou l'inscription de l'étudiant.
const ENROLLMENT_HINTS: [&str; 7] = [
    "affectation",
    "inscription",
    "composante",
    "etape",
    "entite",
    "department",
    "ou",
];

fn base(ctx: &Context<'_>) -> Result<Url, Problem> {
    match &ctx.university.university.portal {
        PortalConfig::TilesPortal { base_url } => Ok(root(base_url)),
        _ => Err(Problem::new(ErrorCode::ServiceAbsent)
            .detail("aucun portail à tuiles n'est déclaré pour cette université")),
    }
}

// Récupération

pub(crate) async fn fetch_profile(ctx: &mut Context<'_>) -> Result<Profile, Problem> {
    let attributes = get_json(ctx, "api/cas-attributes").await?;
    Ok(parse_profile(&attributes, &ctx.session.username))
}

pub(crate) async fn fetch_apps(ctx: &mut Context<'_>) -> Result<Vec<App>, Problem> {
    let base = base(ctx)?;
    let tiles = get_json(ctx, "api/tiles").await?;
    parse_tiles(&tiles, &base)
}

/// Signaux de rattachement lus dans les attributs CAS.
pub(crate) async fn fetch_signals(ctx: &mut Context<'_>) -> Result<Vec<Signal>, Problem> {
    let attributes = get_json(ctx, "api/cas-attributes").await?;
    Ok(signals(&attributes))
}

/// `GET` JSON sur le portail. Sans session (401, redirection), entre par le CAS une seule fois.
async fn get_json(ctx: &mut Context<'_>, path: &str) -> Result<Value, Problem> {
    let base = base(ctx)?;
    let url = base
        .join(path)
        .map_err(|_| upstream(PROVIDER, "adresse du portail illisible"))?;
    let mut net = Net::open(ctx, PROVIDER)?;
    let result = get_with_entry(&mut net, &base, &url).await;
    net.save(ctx);
    let value = result?;
    if tracing::enabled!(tracing::Level::DEBUG) {
        tracing::debug!("{PROVIDER} {path} : {}", shape(&value, 0));
        tracing::debug!("{PROVIDER} {path} : codes {:?}", organization_codes(&value));
    }
    Ok(value)
}

async fn get_with_entry(net: &mut Net, base: &Url, url: &Url) -> Result<Value, Problem> {
    // Sans session, le portail répond 401 en JSON ou redirige (corps non JSON) : dans les deux
    // cas, on entre par le CAS.
    match net
        .json(Method::GET, url.clone(), HeaderMap::new(), None)
        .await
    {
        Ok((status, value)) if status.is_success() => return Ok(value),
        Ok((status, _)) if status.is_server_error() => {
            return Err(Problem::new(ErrorCode::ServiceUnavailable)
                .detail(format!("{PROVIDER} : le portail répond HTTP {status}")));
        }
        Err(problem) if problem.code == ErrorCode::ServiceUnavailable => return Err(problem),
        _ => {}
    }
    // Session du portail absente ou perdue : entrée par le CAS de l'université, puis un essai.
    let entry = base
        .join("cas/login?next=%2F")
        .map_err(|_| upstream(PROVIDER, "adresse du portail illisible"))?;
    net.open_protected(entry).await?;
    let (status, value) = net
        .json(Method::GET, url.clone(), HeaderMap::new(), None)
        .await?;
    match status {
        s if s.is_success() => Ok(value),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            Err(Problem::new(ErrorCode::SessionExpired)
                .detail("le portail refuse la session après le passage par le CAS"))
        }
        s if s.is_server_error() => Err(Problem::new(ErrorCode::ServiceUnavailable)
            .detail(format!("{PROVIDER} : le portail répond HTTP {s}"))),
        s => Err(upstream(PROVIDER, format!("réponse HTTP {s} inattendue"))),
    }
}

// Analyse (sans réseau)

/// Attributs de l'étudiant : objet `userAttributes`, `attributes` ou la racine.
fn attributes(value: &Value) -> Option<&Map<String, Value>> {
    let root = value.as_object()?;
    for key in ["userAttributes", "attributes", "user"] {
        if let Some(inner) = root.get(key).and_then(Value::as_object) {
            return Some(inner);
        }
    }
    Some(root)
}

/// Valeurs texte d'un attribut : chaîne seule ou liste de chaînes.
fn texts(value: &Value) -> Vec<&str> {
    match value {
        Value::String(s) => vec![s.as_str()],
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
    .into_iter()
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .collect()
}

/// Premier texte du premier attribut présent parmi `names` (casse ignorée).
fn first(attrs: &Map<String, Value>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        attrs
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .and_then(|(_, value)| texts(value).first().map(|s| (*s).to_owned()))
    })
}

pub(crate) fn parse_profile(value: &Value, username: &str) -> Profile {
    let Some(attrs) = attributes(value) else {
        return Profile {
            username: username.to_owned(),
            display_name: None,
            first_name: None,
            last_name: None,
            email: None,
            student_number: None,
            program: None,
        };
    };
    Profile {
        username: first(attrs, &["uid", "user", "login"]).unwrap_or_else(|| username.to_owned()),
        display_name: first(attrs, &["displayName", "cn"]),
        first_name: first(attrs, &["givenName"]),
        last_name: first(attrs, &["sn", "surname"]),
        email: first(attrs, &["mail", "email"]),
        student_number: first(attrs, &["supannEtuId", "studentNumber"]),
        program: None,
    }
}

pub(crate) fn signals(value: &Value) -> Vec<Signal> {
    let Some(attrs) = attributes(value) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, value) in attrs {
        let lower = key.to_ascii_lowercase();
        let source = if lower.contains("postaladdress") {
            SignalSource::PostalAddress
        } else if ENROLLMENT_HINTS
            .iter()
            .any(|hint| lower == *hint || (hint.len() > 2 && lower.contains(hint)))
        {
            SignalSource::EnrollmentComponent
        } else {
            continue;
        };
        for text in texts(value) {
            out.push(Signal {
                source,
                value: text.to_owned(),
            });
        }
    }
    out
}

/// Texte éventuellement traduit : chaîne, ou objet `{fr, en, …}` (le français d'abord).
fn localized(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Value::Object(map) => ["fr", "en"]
            .iter()
            .find_map(|lang| map.get(*lang).and_then(Value::as_str))
            .or_else(|| map.values().find_map(Value::as_str))
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
        _ => None,
    }
}

fn field<'a>(tile: &'a Map<String, Value>, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|name| tile.get(*name))
}

pub(crate) fn parse_tiles(value: &Value, base: &Url) -> Result<Vec<App>, Problem> {
    let list = match value {
        Value::Array(items) => items,
        Value::Object(map) => ["tiles", "items", "data"]
            .iter()
            .find_map(|key| map.get(*key).and_then(Value::as_array))
            .ok_or_else(|| upstream(PROVIDER, "liste des tuiles absente"))?,
        _ => return Err(upstream(PROVIDER, "liste des tuiles absente")),
    };
    let mut apps: Vec<App> = Vec::new();
    for tile in list.iter().filter_map(Value::as_object) {
        let id = match field(tile, &["id", "fname", "key"]) {
            Some(Value::String(s)) if !s.is_empty() => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            _ => continue,
        };
        let Some(title) = field(tile, &["title", "name", "label"]).and_then(localized) else {
            continue;
        };
        let Some(url) = field(tile, &["url", "href", "link"])
            .and_then(Value::as_str)
            .and_then(|raw| base.join(raw.trim()).ok())
            .filter(|url| matches!(url.scheme(), "https" | "http"))
        else {
            continue;
        };
        let category =
            field(tile, &["category", "categories", "categoryKey"]).and_then(|c| match c {
                Value::Array(items) => items.iter().find_map(|item| {
                    localized(item).or_else(|| item.get("key").and_then(localized))
                }),
                other => localized(other).or_else(|| other.get("key").and_then(localized)),
            });
        let icon_url = field(tile, &["iconUrl", "icon", "image"])
            .and_then(Value::as_str)
            .and_then(|raw| base.join(raw.trim()).ok())
            .filter(|url| matches!(url.scheme(), "https" | "http"))
            .map(String::from);
        // Un portail réel en a une quarantaine ; au-delà, la liste est suspecte et tronquée.
        if apps.len() >= 500 {
            break;
        }
        if apps.iter().any(|app| app.id == id) {
            continue;
        }
        apps.push(App {
            id,
            title,
            description: field(tile, &["description", "summary"]).and_then(localized),
            url: url.into(),
            icon_url,
            category,
            favorite: false,
        });
    }
    Ok(apps)
}

/// Codes d'organisation (composante, affectation, type de compte) et règles des tuiles qui les
/// utilisent : ce sont des réglages de l'établissement, pas des données personnelles. Sert à
/// écrire les règles de rattachement.
fn organization_codes(value: &Value) -> Vec<String> {
    const KEYS: [&str; 4] = [
        "umCodeComposante",
        "supannEntiteAffectation",
        "supannEtablissement",
        "eduPersonPrimaryAffiliation",
    ];
    let mut out = Vec::new();
    if let Some(attrs) = attributes(value) {
        for key in KEYS {
            if let Some(v) = attrs.get(key) {
                out.push(format!("{key}={:?}", texts(v)));
            }
        }
    }
    if let Some(tiles) = value.get("tiles").and_then(Value::as_array) {
        for tile in tiles {
            let conditions = tile
                .get("ruleConditions")
                .map(Value::to_string)
                .unwrap_or_default();
            if KEYS.iter().any(|k| conditions.contains(k)) {
                let title = tile.get("title").and_then(localized).unwrap_or_default();
                out.push(format!("tuile {title:?} : {conditions}"));
            }
        }
    }
    out
}

/// Structure d'une valeur JSON pour le diagnostic : clés et types, jamais de valeur.
fn shape(value: &Value, depth: usize) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(_) => "bool".to_owned(),
        Value::Number(_) => "nombre".to_owned(),
        Value::String(_) => "texte".to_owned(),
        Value::Array(items) => match items.first() {
            Some(first) if depth < 4 => format!("[{} × {}]", items.len(), shape(first, depth + 1)),
            Some(_) => format!("[{} éléments]", items.len()),
            None => "[]".to_owned(),
        },
        Value::Object(map) if depth < 4 => {
            let fields: Vec<String> = map
                .iter()
                .take(60)
                .map(|(key, value)| format!("{key}: {}", shape(value, depth + 1)))
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
        Value::Object(map) => format!("{{{} clés}}", map.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tonapi_config::Affiliation;

    fn base() -> Url {
        Url::parse("https://ent.example.test/").unwrap()
    }

    #[test]
    fn profil_depuis_des_attributs_en_liste() {
        let value = json!({
            "userAttributes": {
                "uid": ["e00000000000"],
                "displayName": "Camille Demo",
                "givenName": ["Camille"],
                "sn": ["Demo"],
                "mail": ["camille.demo@example.org"],
            },
            "isAdmin": false
        });
        let profile = parse_profile(&value, "camille.demo@example.org");
        assert_eq!(profile.username, "e00000000000");
        assert_eq!(profile.display_name.as_deref(), Some("Camille Demo"));
        assert_eq!(profile.last_name.as_deref(), Some("Demo"));
        assert_eq!(profile.student_number, None);
    }

    #[test]
    fn signaux_de_composante_et_adresse() {
        let value = json!({
            "supannEntiteAffectationPrincipale": "I.U.T. de Béziers",
            "postalAddress": ["3 place du Test$34500$BEZIERS"],
            "mail": "camille.demo@example.org",
            "ou": ["IUT Béziers"],
        });
        let signals = signals(&value);
        assert_eq!(signals.len(), 3);
        assert!(signals.iter().all(|s| !s.value.contains('@')));

        let rules: Affiliation = serde_json::from_value(json!({
            "rules": [
                { "signal": "enrollment-component", "contains": "I.U.T. de Béziers", "weight": 3 },
                { "signal": "postal-address", "suffix": "$BEZIERS" }
            ]
        }))
        .unwrap();
        assert_eq!(rules.score(&signals), 4);
    }

    #[test]
    fn tuiles_traduites_relatives_et_doublons() {
        let value = json!({ "tiles": [
            { "id": "moodle", "title": { "fr": "Moodle", "en": "Moodle" }, "url": "https://moodle.example.test/", "categories": ["pedagogie"] },
            { "fname": "mail", "label": "Messagerie", "href": "/go/mail", "description": { "fr": "Courriel" } },
            { "id": "moodle", "title": "Doublon", "url": "https://autre.example.test/" },
            { "id": "piege", "title": "JS", "url": "javascript:alert(1)" },
            { "title": "Sans identifiant", "url": "https://x.example.test/" }
        ]});
        let apps = parse_tiles(&value, &base()).unwrap();
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].category.as_deref(), Some("pedagogie"));
        assert_eq!(apps[1].url, "https://ent.example.test/go/mail");
        assert_eq!(apps[1].description.as_deref(), Some("Courriel"));
        assert!(parse_tiles(&json!({"autre": 1}), &base()).is_err());
    }

    #[test]
    fn la_forme_ne_contient_aucune_valeur() {
        let value = json!({ "uid": "e00000000000", "mail": ["camille.demo@example.org"], "n": 3 });
        let text = shape(&value, 0);
        assert!(!text.contains("e0000") && !text.contains("camille"));
        assert!(text.contains("uid: texte") && text.contains("mail: [1 × texte]"));
    }
}
