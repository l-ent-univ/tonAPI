//! Catalogue des fournisseurs, lu dans le JSON Schema pour ne jamais diverger des types.

use serde::Serialize;
use serde_json::Value;

use crate::schema::university_schema;
use crate::services::ServiceKind;

/// Un fournisseur utilisable pour un service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    /// Valeur de `provider`.
    pub id: String,
    /// Clé du service dans `[services]`.
    pub service: String,
    pub description: String,
    /// Paramètres obligatoires, en plus de `provider`.
    pub required: Vec<String>,
    /// Paramètres facultatifs.
    pub optional: Vec<String>,
}

/// Tous les fournisseurs, service par service, dans l'ordre de [`ServiceKind::ALL`].
#[must_use]
pub fn providers() -> Vec<ProviderInfo> {
    let schema = university_schema();
    let defs = &schema["$defs"];
    let mut out = Vec::new();
    for kind in ServiceKind::ALL {
        let property = &defs["Services"]["properties"][kind.key()];
        let Some(def) = find_ref(property).and_then(|name| defs.get(name)) else {
            continue;
        };
        for variant in def["oneOf"].as_array().into_iter().flatten() {
            let Some(id) = variant["properties"]["provider"]["const"].as_str() else {
                continue;
            };
            let required: Vec<String> = strings(&variant["required"])
                .filter(|k| k != "provider")
                .collect();
            let optional = variant["properties"]
                .as_object()
                .into_iter()
                .flat_map(|props| props.keys())
                .filter(|k| *k != "provider" && !required.contains(k))
                .cloned()
                .collect();
            out.push(ProviderInfo {
                id: id.to_owned(),
                service: kind.key().to_owned(),
                description: variant["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                required,
                optional,
            });
        }
    }
    out
}

/// Nom de la définition visée par un `$ref`, éventuellement dans un `anyOf` (champ facultatif).
fn find_ref(value: &Value) -> Option<&str> {
    if let Some(r) = value["$ref"].as_str() {
        return r.strip_prefix("#/$defs/");
    }
    value["anyOf"].as_array()?.iter().find_map(find_ref)
}

fn strings(value: &Value) -> impl Iterator<Item = String> + '_ {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_service_a_ses_fournisseurs() {
        let list = providers();
        for kind in ServiceKind::ALL {
            let ids: Vec<_> = list
                .iter()
                .filter(|p| p.service == kind.key())
                .map(|p| p.id.as_str())
                .collect();
            assert!(ids.contains(&"none"), "{}: {ids:?}", kind.key());
        }
        let scodoc = list
            .iter()
            .find(|p| p.service == "grades" && p.id == "scodoc-gateway")
            .expect("scodoc-gateway");
        assert_eq!(scodoc.required, ["base-url"]);
        assert!(scodoc.optional.contains(&"sign-in".to_owned()));
    }
}
