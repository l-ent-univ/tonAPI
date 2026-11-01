//! Entrée Shibboleth de `moodle-ajax` : fournisseur de service → WAYF → IdP → CAS (TGC) → SAML →
//! Moodle.

use tonapi_config::SamlConfig;
use tonapi_core::{ErrorCode, Problem};
use tonapi_http::{Page, forms};
use url::Url;

use super::parse::{PageState, decode_entities, page_state};
use super::{AJAX, on_moodle};
use crate::net::{Net, http_problem, upstream};

const MAX_SAML_STEPS: usize = 25;

/// Hôte autorisé pendant la fédération : Moodle, le CAS de l'université, ou un suffixe configuré.
pub(super) fn host_allowed(
    url: &Url,
    base: &Url,
    cas_host: Option<&str>,
    saml: &SamlConfig,
) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    on_moodle(url, base)
        || cas_host == Some(host)
        || saml
            .allowed_hosts
            .iter()
            .any(|suffix| host == suffix || host.ends_with(&format!(".{suffix}")))
}

fn resolve_action(from: &Url, action: Option<&str>) -> Result<Url, Problem> {
    match action.filter(|a| !a.is_empty()) {
        Some(action) => from
            .join(action)
            .map_err(|_| upstream(AJAX, "formulaire de connexion fédérée illisible")),
        None => Ok(from.clone()),
    }
}

/// Mène la chaîne SAML jusqu'à une page Moodle connectée. Aucune donnée n'est postée hors des
/// hôtes autorisés, et jamais de mot de passe : si un formulaire en demande un, la session de
/// l'université est perdue.
pub(super) async fn follow_saml(
    net: &mut Net,
    base: &Url,
    saml: Option<&SamlConfig>,
    mut page: Page,
) -> Result<Page, Problem> {
    let Some(saml) = saml else {
        return Err(Problem::new(ErrorCode::Internal)
            .detail("entrée `shibboleth` sans `sso.saml` dans la configuration de l'université"));
    };
    let cas_host = net
        .cas
        .as_ref()
        .and_then(|cas| cas.base.host_str())
        .map(str::to_owned);
    let allowed = |url: &Url| host_allowed(url, base, cas_host.as_deref(), saml);

    for _ in 0..MAX_SAML_STEPS {
        if !allowed(&page.url) {
            return Err(upstream(AJAX, "la fédération sort des hôtes autorisés"));
        }
        if page.stopped_at.is_some() {
            return Err(upstream(AJAX, "redirection hors HTTP pendant la connexion"));
        }
        if on_moodle(&page.url, base) && matches!(page_state(&page.body), PageState::LoggedIn(_)) {
            return Ok(page);
        }
        let forms = forms::forms(&page.body);
        if forms.iter().any(|f| f.has_password) {
            return Err(Problem::new(ErrorCode::SessionExpired)
                .detail("l'université redemande le mot de passe : se reconnecter"));
        }

        let (target, fields) = if let Some(form) = forms.iter().find(|f| f.has("user_idp")) {
            let mut fields: Vec<(String, String)> = form
                .fields
                .iter()
                .filter(|(name, _)| name != "user_idp")
                .cloned()
                .collect();
            fields.push(("user_idp".to_owned(), saml.idp_entity_id.clone()));
            fields.push(
                form.buttons
                    .iter()
                    .find(|(name, _)| name == "Select")
                    .cloned()
                    .unwrap_or_else(|| ("Select".to_owned(), "Sélection".to_owned())),
            );
            (resolve_action(&page.url, form.action.as_deref())?, fields)
        } else if let Some(form) = forms
            .iter()
            .find(|f| f.has("SAMLResponse") || f.has("SAMLRequest"))
        {
            (
                resolve_action(&page.url, form.action.as_deref())?,
                form.fields.clone(),
            )
        } else if let Some(next) = html_redirect(&page.body, &page.url) {
            if !allowed(&next) {
                return Err(upstream(AJAX, "la fédération sort des hôtes autorisés"));
            }
            page = net
                .browser
                .get(next)
                .await
                .map_err(|e| http_problem(AJAX, &e))?;
            continue;
        } else {
            return Err(upstream(
                AJAX,
                "page inattendue pendant la connexion fédérée",
            ));
        };
        if !allowed(&target) {
            return Err(upstream(AJAX, "la fédération sort des hôtes autorisés"));
        }
        page = net
            .browser
            .post_form(target, &fields, &page.url)
            .await
            .map_err(|e| http_problem(AJAX, &e))?;
    }
    Err(upstream(AJAX, "trop d'étapes dans la connexion fédérée"))
}

/// Redirection écrite dans la page : `<meta http-equiv="refresh">` ou `location.href = "…"`.
pub(super) fn html_redirect(body: &str, from: &Url) -> Option<Url> {
    let target = meta_refresh(body).or_else(|| script_redirect(body))?;
    from.join(&decode_entities(target)).ok()
}

fn meta_refresh(body: &str) -> Option<&str> {
    let lower = body.to_ascii_lowercase();
    for (start, _) in lower.match_indices("<meta") {
        // Sans `>` plus loin, aucune balise suivante ne se ferme non plus : on s'arrête (sinon
        // chaque `<meta` relirait toute la fin de la page).
        let Some(end) = lower
            .get(start..)
            .and_then(|rest| rest.find('>'))
            .map(|e| start + e)
        else {
            break;
        };
        let Some(tag) = lower.get(start..end) else {
            continue;
        };
        if !tag.contains("refresh") {
            continue;
        }
        let Some(at) = tag.find("url=") else {
            continue;
        };
        // `lower` et `body` ont les mêmes positions : seule la casse ASCII diffère.
        let value = body
            .get(start + at + 4..end)?
            .trim_start_matches(['\'', '"'])
            .split(['\'', '"'])
            .next()?
            .trim();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

fn script_redirect(body: &str) -> Option<&str> {
    for needle in ["location.replace(", "location.href", "window.location"] {
        let Some(at) = body.find(needle) else {
            continue;
        };
        let rest = body.get(at + needle.len()..)?.trim_start();
        let rest = rest.trim_start_matches(['=', '(']).trim_start();
        let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let inner = rest.get(1..)?;
        let value = inner.split(quote).next()?.trim();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}
