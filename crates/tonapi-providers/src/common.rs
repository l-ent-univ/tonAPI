//! Petites fonctions partagées par plusieurs fournisseurs : racine d'un service, clé de
//! `SessionData::tokens`, fuseau de l'université, heure locale et lecture de champs JSON.

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeDelta, TimeZone};
use chrono_tz::Tz;
use serde_json::Value;
use tonapi_core::{ErrorCode, Problem};
use url::Url;

use crate::Context;

/// Racine d'un service avec sa barre finale, sans requête ni fragment : `…/moodle` +
/// `login/token.php` donnerait sinon `…/login/token.php`.
pub(crate) fn root(base_url: &Url) -> Url {
    let mut base = base_url.clone();
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base.set_query(None);
    base.set_fragment(None);
    base
}

/// Clé de `SessionData::tokens` : `<prefix>.<hôte>[:port]`, le port seulement s'il est explicite
/// et différent du port par défaut du schéma. Ces clés sont stockées dans les jetons chiffrés
/// gardés par les clients : en changer un octet forcerait une reconnexion.
pub(crate) fn token_key(prefix: &str, base: &Url) -> String {
    let host = base.host_str().unwrap_or_default();
    match base.port() {
        Some(port) => format!("{prefix}.{host}:{port}"),
        None => format!("{prefix}.{host}"),
    }
}

pub(crate) fn university_tz(ctx: &Context<'_>) -> Result<Tz, Problem> {
    parse_tz(&ctx.university.university.timezone)
}

fn parse_tz(name: &str) -> Result<Tz, Problem> {
    name.parse::<Tz>().map_err(|_| {
        Problem::new(ErrorCode::Internal).detail("fuseau horaire de l'université illisible")
    })
}

/// Heure locale → instant. Heure répétée (retour à l'heure d'hiver) : la première occurrence.
/// Heure inexistante (passage à l'heure d'été) : une heure plus tard sur l'horloge, comme
/// RFC 5545 §3.3.5.
pub(crate) fn resolve_local(tz: Tz, naive: NaiveDateTime) -> Option<DateTime<FixedOffset>> {
    tz.from_local_datetime(&naive)
        .earliest()
        .or_else(|| {
            naive
                .checked_add_signed(TimeDelta::hours(1))
                .and_then(|later| tz.from_local_datetime(&later).earliest())
        })
        .map(|instant| instant.fixed_offset())
}

/// Texte d'une chaîne (blancs compactés) ou d'un nombre ; `None` si vide ou d'un autre type.
pub(crate) fn text_or_number(value: Option<&Value>) -> Option<String> {
    let text = match value? {
        Value::String(s) => s.split_whitespace().collect::<Vec<_>>().join(" "),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

/// Identifiant envoyé comme nombre ou comme chaîne.
pub(crate) fn id_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Chaîne non vide (après retrait des blancs) du champ `key`.
pub(crate) fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use chrono_tz::Europe::Paris;
    use serde_json::json;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn racine_avec_barre_finale() {
        assert_eq!(
            root(&url("https://moodle.exemple.test/moodle")).as_str(),
            "https://moodle.exemple.test/moodle/"
        );
        assert_eq!(
            root(&url("https://moodle.exemple.test/moodle/")).as_str(),
            "https://moodle.exemple.test/moodle/"
        );
    }

    #[test]
    fn racine_sans_requete_ni_fragment() {
        assert_eq!(
            root(&url("https://ent.exemple.test/x?q#f")).as_str(),
            "https://ent.exemple.test/x/"
        );
    }

    #[test]
    fn cle_avec_et_sans_port() {
        assert_eq!(
            token_key("scodoc", &url("https://notes.exemple.test/x")),
            "scodoc.notes.exemple.test"
        );
        assert_eq!(
            token_key("scodoc", &url("https://notes.exemple.test:8443/x")),
            "scodoc.notes.exemple.test:8443"
        );
        assert_eq!(
            token_key("scodoc", &url("http://127.0.0.1:4321/")),
            "scodoc.127.0.0.1:4321"
        );
        // Port par défaut du schéma : non écrit.
        assert_eq!(
            token_key("moodle.token", &url("https://moodle.exemple.test:443/")),
            "moodle.token.moodle.exemple.test"
        );
    }

    #[test]
    fn fuseau_illisible() {
        assert!(parse_tz("Europe/Paris").is_ok());
        assert_eq!(
            parse_tz("Mars/Olympus").unwrap_err().code,
            ErrorCode::Internal
        );
    }

    fn naive(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    #[test]
    fn heure_sautee_avance_d_une_heure() {
        // 29/03/2026 : 02:00 → 03:00 à Paris, 02:30 n'existe pas.
        let instant = resolve_local(Paris, naive(2026, 3, 29, 2, 30)).unwrap();
        assert_eq!(instant.to_rfc3339(), "2026-03-29T03:30:00+02:00");
    }

    #[test]
    fn heure_repetee_prend_la_premiere() {
        // 25/10/2026 : 03:00 → 02:00 à Paris, 02:30 existe deux fois.
        let instant = resolve_local(Paris, naive(2026, 10, 25, 2, 30)).unwrap();
        assert_eq!(instant.to_rfc3339(), "2026-10-25T02:30:00+02:00");
    }

    #[test]
    fn texte_ou_nombre() {
        assert_eq!(
            text_or_number(Some(&json!("  a \n b "))),
            Some("a b".to_owned())
        );
        assert_eq!(text_or_number(Some(&json!(12))), Some("12".to_owned()));
        assert_eq!(text_or_number(Some(&json!("  "))), None);
        assert_eq!(text_or_number(Some(&json!(true))), None);
        assert_eq!(text_or_number(None), None);
    }
}
