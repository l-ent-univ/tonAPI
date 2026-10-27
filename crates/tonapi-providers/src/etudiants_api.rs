//! Dossier étudiant `etudiants-api` : complète le profil du portail par le numéro étudiant et la
//! formation, lus dans `api/etudiants/notes` d'une application de scolarité derrière le CAS.
//!
//! [vu] Paul-Valéry, 27/10/2026 : application Angular, réponse JSON `{numero, nom, prenom,
//! etapes: [{libelle, codEtape, …}], inscription, …}`. Seuls le numéro et le libellé de l'étape sont
//! repris ; les notes de la même réponse ne sont pas lues ici.

use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde_json::Value;
use tonapi_core::{ErrorCode, Problem};
use url::Url;

use crate::Context;
use crate::net::{Net, upstream};

const PROVIDER: &str = "etudiants-api";
const NOTES: &str = "api/etudiants/notes";

/// Ce que le dossier ajoute au profil.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Record {
    pub student_number: Option<String>,
    pub program: Option<String>,
}

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

pub(crate) async fn fetch_record(ctx: &mut Context<'_>, base_url: &Url) -> Result<Record, Problem> {
    let base = root(base_url);
    let url = base
        .join(NOTES)
        .map_err(|_| upstream(PROVIDER, "adresse de l'application illisible"))?;
    let mut net = Net::open(ctx, PROVIDER)?;
    let result = read(&mut net, &base, url).await;
    net.save(ctx);
    parse_record(&result?)
}

/// La route JSON, après une entrée par le CAS si l'application n'a pas encore de session.
async fn read(net: &mut Net, base: &Url, url: Url) -> Result<Value, Problem> {
    if let Some(value) = get_json(net, url.clone()).await? {
        return Ok(value);
    }
    net.open_protected(base.clone()).await?;
    get_json(net, url).await?.ok_or_else(|| {
        Problem::new(ErrorCode::SessionExpired)
            .detail("l'application de scolarité refuse la session après le passage par le CAS")
    })
}

/// `None` quand l'application n'a pas de session (redirection, 401, 403 ou page HTML).
async fn get_json(net: &mut Net, url: Url) -> Result<Option<Value>, Problem> {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    let (status, _, body) = net.request(Method::GET, url, headers, None).await?;
    if status.is_server_error() {
        return Err(Problem::new(ErrorCode::ServiceUnavailable)
            .detail(format!("{PROVIDER} : l'application répond HTTP {status}")));
    }
    if !status.is_success() {
        return Ok(None);
    }
    Ok(serde_json::from_str(&body).ok())
}

fn text(value: Option<&Value>) -> Option<String> {
    let text = match value? {
        Value::String(s) => s.split_whitespace().collect::<Vec<_>>().join(" "),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

pub(crate) fn parse_record(value: &Value) -> Result<Record, Problem> {
    let root = value
        .as_object()
        .ok_or_else(|| upstream(PROVIDER, "dossier étudiant illisible"))?;
    // Plusieurs étapes possibles (double cursus) : leurs libellés, dans l'ordre, sans doublon.
    let mut programs: Vec<String> = Vec::new();
    for step in root
        .get("etapes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(label) = text(step.get("libelle"))
            && !programs.contains(&label)
        {
            programs.push(label);
        }
    }
    Ok(Record {
        student_number: text(root.get("numero")),
        program: (!programs.is_empty()).then(|| programs.join(" ; ")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numero_et_formation() {
        let value = json!({
            "numero": "21000000", "nom": "DEMO", "prenom": "Camille", "ajac": false,
            "etapes": [
                { "libelle": "Licence 1  Arts", "codEtape": "XX1", "semestres": {} },
                { "libelle": "Licence 1  Arts", "codEtape": "XX1", "semestres": {} },
                { "libelle": "Certificat demo", "codEtape": "XX2" }
            ],
            "epreuves": {}
        });
        let record = parse_record(&value).unwrap();
        assert_eq!(record.student_number.as_deref(), Some("21000000"));
        assert_eq!(
            record.program.as_deref(),
            Some("Licence 1 Arts ; Certificat demo")
        );
    }

    #[test]
    fn champs_absents_et_numero_entier() {
        assert_eq!(parse_record(&json!({})).unwrap(), Record::default());
        let record =
            parse_record(&json!({ "numero": 21_000_000, "etapes": [{ "codEtape": "X" }] }))
                .unwrap();
        assert_eq!(record.student_number.as_deref(), Some("21000000"));
        assert_eq!(record.program, None);
        assert!(parse_record(&json!([])).is_err());
    }
}
