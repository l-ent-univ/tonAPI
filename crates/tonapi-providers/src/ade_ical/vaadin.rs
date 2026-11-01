//! Page Vaadin Flow de prose-etudiant : état initial (clé anti-CSRF, numéro d'UI, compteurs) et
//! événement de navigation qui fait renvoyer le lien du flux.

use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderValue};
use tonapi_core::Problem;
use url::Url;

use super::PROVIDER;
use crate::net::{Net, upstream};

/// État d'une page Vaadin Flow nécessaire pour lui envoyer des événements (`?v-r=uidl`) : clé
/// anti-CSRF (`Vaadin-Security-Key` de l'état initial), numéro de l'UI et compteurs d'échange.
pub(super) struct VaadinPage {
    endpoint: Url,
    href: Url,
    pub(super) location: String,
    csrf: String,
    sync_id: u64,
    client_id: u64,
}

impl VaadinPage {
    pub(super) fn read(page: &Url, body: &str) -> Option<Self> {
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
    pub(super) async fn navigate(
        &mut self,
        net: &mut Net,
        location: &str,
    ) -> Result<String, Problem> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
