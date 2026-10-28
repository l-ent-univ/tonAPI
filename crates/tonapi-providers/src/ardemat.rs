//! Fournisseur `ardemat` : attestations de réussite dématérialisées (Spring Boot, Swagger public).
//!
//! L'API ne garde pas de session : elle valide elle-même un ticket CAS émis pour l'application
//! (`service-url`) et passé dans le chemin. On demande donc au CAS, avec le TGC de la session, un
//! ticket qu'on ne consomme pas, puis :
//! - `GET <base>api/etudiants/<ticket>` → `EtudiantDto{codeEtu, codeInd, nom, email, estInterdit,
//!   attestations: [AttestationDto]}` ;
//! - `POST <base>api/attestations/download` avec l'`AttestationDto` reçu (dont `attestationToken`
//!   et `etudiantToken`) → le PDF.
//!
//! Un ticket ne sert qu'une fois : chaque appel en redemande un. `api/attestations/mail` (envoi
//! par courriel) n'est jamais appelé.
//!
//! [vu] 13/10/2026 : Swagger `/ardemat-api/v2/api-docs`, appel du front, liste vide pour un BUT1.
//! [vu] 28/10/2026 : service CAS `https://app.umontpellier.fr/ardemat/`. Une attestation réelle et
//! son téléchargement n'ont pas été observés : ce que `download` exige en plus des jetons
//! [inconnu].

use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode};
use serde_json::Value;
use tonapi_core::{Document, DocumentKind, ErrorCode, Problem};
use url::Url;

use crate::net::{Net, upstream};
use crate::{Context, Download};

const PROVIDER: &str = "ardemat";

fn root(base_url: &Url) -> Url {
    let mut base = base_url.clone();
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base.set_query(None);
    base.set_fragment(None);
    base
}

pub(crate) async fn fetch_documents(
    ctx: &mut Context<'_>,
    base_url: &Url,
    service_url: &Url,
) -> Result<Vec<Document>, Problem> {
    let base = root(base_url);
    let mut net = Net::open(ctx, PROVIDER)?;
    let result = student(&mut net, &base, service_url).await;
    net.save(ctx);
    parse_documents(&result?)
}

/// PDF d'une attestation : le dossier est relu pour retrouver l'attestation et ses jetons.
pub(crate) async fn download(
    ctx: &mut Context<'_>,
    base_url: &Url,
    service_url: &Url,
    id: &str,
) -> Result<Download, Problem> {
    if id.is_empty()
        || id.len() > 120
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(unknown());
    }
    let base = root(base_url);
    let mut net = Net::open(ctx, PROVIDER)?;
    let result = async {
        let student = student(&mut net, &base, service_url).await?;
        if student.get("estInterdit").and_then(Value::as_bool) == Some(true) {
            return Err(blocked());
        }
        let dto = attestations(&student)?
            .iter()
            .find(|a| document_id(a) == id)
            .cloned()
            .ok_or_else(unknown)?;
        post_download(&mut net, &base, &dto, id).await
    }
    .await;
    net.save(ctx);
    result
}

fn unknown() -> Problem {
    Problem::new(ErrorCode::NotFound).detail("document inconnu")
}

fn blocked() -> Problem {
    Problem::new(ErrorCode::AccountActionRequired)
        .detail("l'université bloque la délivrance des attestations pour ce compte")
}

/// `EtudiantDto` de l'étudiant connecté, avec un ticket neuf.
async fn student(net: &mut Net, base: &Url, service_url: &Url) -> Result<Value, Problem> {
    let ticket = net.service_ticket(service_url).await?;
    let mut url = base
        .join("api/etudiants/")
        .map_err(|_| upstream(PROVIDER, "adresse de l'API illisible"))?;
    url.path_segments_mut()
        .map_err(|()| upstream(PROVIDER, "adresse de l'API illisible"))?
        .pop_if_empty()
        .push(&ticket);
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    let (status, _, body) = net.request(Method::GET, url, headers, None).await?;
    match status {
        s if s.is_success() => {
            serde_json::from_str(&body).map_err(|_| upstream(PROVIDER, "dossier étudiant non JSON"))
        }
        // Étudiant absent de la base des attestations [déduit] : aucun document.
        StatusCode::NOT_FOUND => Ok(serde_json::json!({ "attestations": [] })),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            Err(Problem::new(ErrorCode::SessionExpired)
                .detail("ardemat refuse le ticket CAS : se reconnecter"))
        }
        s if s.is_server_error() => Err(Problem::new(ErrorCode::ServiceUnavailable)
            .detail(format!("{PROVIDER} : l'API répond HTTP {s}"))),
        s => Err(upstream(PROVIDER, format!("dossier étudiant : HTTP {s}"))),
    }
}

async fn post_download(
    net: &mut Net,
    base: &Url,
    dto: &Value,
    id: &str,
) -> Result<Download, Problem> {
    let url = base
        .join("api/attestations/download")
        .map_err(|_| upstream(PROVIDER, "adresse de l'API illisible"))?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("application/pdf, */*"),
    );
    let body = serde_json::to_string(dto).map_err(|_| unknown())?;
    let (status, _, bytes) = net.bytes(Method::POST, url, headers, Some(body)).await?;
    match status {
        s if s.is_success() && bytes.starts_with(b"%PDF") => Ok(Download {
            content_type: "application/pdf".to_owned(),
            file_name: format!("attestation-{id}.pdf"),
            bytes,
        }),
        s if s.is_success() => Err(upstream(PROVIDER, "le téléchargement n'est pas un PDF")),
        StatusCode::NOT_FOUND => Err(unknown()),
        s if s.is_server_error() => Err(Problem::new(ErrorCode::ServiceUnavailable)
            .detail(format!("{PROVIDER} : l'API répond HTTP {s}"))),
        s => Err(upstream(PROVIDER, format!("téléchargement : HTTP {s}"))),
    }
}

fn attestations(student: &Value) -> Result<&Vec<Value>, Problem> {
    match student.get("attestations") {
        Some(Value::Array(list)) => Ok(list),
        // Champ absent ou nul : aucune attestation.
        None | Some(Value::Null) => Ok(const { &Vec::new() }),
        Some(_) => Err(upstream(PROVIDER, "liste des attestations illisible")),
    }
}

fn text(value: &Value, key: &str) -> Option<String> {
    let text = match value.get(key)? {
        Value::String(s) => s.split_whitespace().collect::<Vec<_>>().join(" "),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

/// Identifiant stable, sans jeton : année, code, session, type et version, réduits à
/// `[A-Za-z0-9_-]`.
fn document_id(dto: &Value) -> String {
    ["annee", "code", "session", "type", "version"]
        .iter()
        .map(|k| text(dto, k).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn parse_documents(student: &Value) -> Result<Vec<Document>, Problem> {
    if !student.is_object() {
        return Err(upstream(PROVIDER, "dossier étudiant illisible"));
    }
    let blocked = student.get("estInterdit").and_then(Value::as_bool) == Some(true);
    Ok(attestations(student)?
        .iter()
        .map(|dto| Document {
            id: document_id(dto),
            title: text(dto, "libelle")
                .or_else(|| text(dto, "type"))
                .unwrap_or_else(|| "Attestation de réussite".to_owned()),
            kind: DocumentKind::Achievement,
            year: text(dto, "annee"),
            session: text(dto, "session"),
            downloadable: !blocked,
            url: None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn attestations_sans_jeton_dans_l_identifiant() {
        let student = json!({ "codeEtu": "00000000", "estInterdit": false, "attestations": [
            { "annee": "2025", "code": "L1/ARTS", "session": "1", "type": "AR", "version": 2,
              "libelle": " Licence 1  Arts ", "attestationToken": "secret-a", "etudiantToken": "secret-e" }
        ] });
        let docs = parse_documents(&student).unwrap();
        assert_eq!(docs[0].id, "2025-L1_ARTS-1-AR-2");
        assert_eq!(docs[0].title, "Licence 1 Arts");
        assert_eq!(docs[0].kind, DocumentKind::Achievement);
        assert!(docs[0].downloadable);
        assert!(!docs[0].id.contains("secret"));
    }

    #[test]
    fn liste_vide_absente_ou_bloquee() {
        assert!(
            parse_documents(&json!({ "attestations": [] }))
                .unwrap()
                .is_empty()
        );
        assert!(
            parse_documents(&json!({ "attestations": null }))
                .unwrap()
                .is_empty()
        );
        let blocked = json!({ "estInterdit": true, "attestations": [{ "annee": "2025" }] });
        assert!(!parse_documents(&blocked).unwrap()[0].downloadable);
        assert!(parse_documents(&json!({ "attestations": {} })).is_err());
        assert!(parse_documents(&json!([])).is_err());
    }
}
