//! Zimbra (`zimbra-rest`) : boîte de réception de l'étudiant.
//!
//! Parcours : la page d'entrée du webmail passe par le CAS (TGC de la session, sans mot de passe) et
//! finit par la préauthentification Zimbra, qui pose le cookie `ZM_AUTH_TOKEN`. La boîte se lit
//! ensuite en SOAP au format JSON (`/service/soap/SearchRequest`, `GetFolderRequest`), avec ce seul
//! cookie. Sources : `Docs/recherche/Montpellier/ent.md` (« Mail : Zimbra »), `Docs/recherche/Rennes/mail-et-autres.md`.
//!
//! - Cookie absent de la session, ou refusé (`AUTH_REQUIRED`, `AUTH_EXPIRED`, HTTP 401) : la chaîne
//!   de connexion est rejouée **une seule fois**.
//! - Le CAS redemande le mot de passe ou un second facteur (souvent à l'ouverture du mail) :
//!   `session_expired`, l'étudiant doit se reconnecter. Quand le CAS exige le second facteur pour
//!   le mail seulement, `[sso.mfa] step-up-url` le fait demander dès la connexion.
//! - Un webmail derrière une fédération SAML (formulaire `SAMLResponse`) n'est pas pris en charge :
//!   `not_implemented`.

use chrono::{DateTime, TimeZone};
use chrono_tz::Tz;
use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use tonapi_core::{ErrorCode, MailAddress, MailInbox, MailMessage, Problem};
use tonapi_http::forms;
use url::Url;

use crate::Context;
use crate::common::{str_field, university_tz};
use crate::net::{Net, upstream};

const PROVIDER: &str = "zimbra-rest";
/// Cookie de session Zimbra posé par la préauthentification.
const TOKEN_COOKIE: &str = "ZM_AUTH_TOKEN";
/// Nombre de messages demandés (Zimbra honore `limit` en SOAP).
const MAX_MESSAGES: u32 = 20;
/// Longueur maximale de l'aperçu, en caractères.
const PREVIEW_CHARS: usize = 280;

/// Pourquoi une lecture a échoué : session perdue (une nouvelle connexion peut aider) ou autre.
#[derive(Debug)]
enum Failure {
    Expired,
    Problem(Problem),
}

impl From<Problem> for Failure {
    fn from(problem: Problem) -> Self {
        Failure::Problem(problem)
    }
}

// Analyse pure

/// Code d'erreur Zimbra d'une réponse en faute (`Body.Fault.Detail.Error.Code`), nettoyé pour qu'il
/// puisse figurer dans un message : des lettres, chiffres, points et soulignés seulement.
fn fault_code(body: &Value) -> Option<String> {
    let code = body
        .pointer("/Body/Fault/Detail/Error/Code")
        .and_then(Value::as_str)?;
    let clean = code.len() <= 64
        && !code.is_empty()
        && code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_');
    Some(if clean { code } else { "inconnu" }.to_owned())
}

/// Interprète la réponse HTTP d'une requête SOAP : JSON valide, sans faute.
///
/// Zimbra répond souvent HTTP 500 avec une faute JSON : la faute prime donc sur le statut.
fn parse_envelope(status: u16, text: &str) -> Result<Value, Failure> {
    let value: Option<Value> = serde_json::from_str(text).ok();
    if let Some(value) = &value
        && value.pointer("/Body/Fault").is_some()
    {
        return Err(match fault_code(value) {
            Some(code) if code.contains("AUTH_REQUIRED") || code.contains("AUTH_EXPIRED") => {
                Failure::Expired
            }
            Some(code) => Failure::Problem(upstream(PROVIDER, format!("faute Zimbra ({code})"))),
            None => Failure::Problem(upstream(PROVIDER, "faute Zimbra sans code")),
        });
    }
    // Pas de faute lisible : un refus d'accès, ou une redirection vers la page de connexion.
    if status == 401 || (300..400).contains(&status) {
        return Err(Failure::Expired);
    }
    if !(200..300).contains(&status) {
        return Err(Failure::Problem(
            Problem::new(ErrorCode::ServiceUnavailable)
                .detail(format!("{PROVIDER} : réponse HTTP {status}")),
        ));
    }
    value.ok_or_else(|| {
        Failure::Problem(upstream(
            PROVIDER,
            format!("réponse non JSON (HTTP {status})"),
        ))
    })
}

/// `Body.<name>` d'une enveloppe SOAP.
fn response_body<'a>(envelope: &'a Value, name: &str) -> Result<&'a Value, Problem> {
    envelope
        .get("Body")
        .and_then(|body| body.get(name))
        .ok_or_else(|| upstream(PROVIDER, format!("`Body.{name}` absent")))
}

/// Identifiant : Zimbra l'envoie en chaîne, un nombre est accepté.
fn message_id(m: &Value) -> Option<String> {
    match m.get("id")? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Date de réception : millisecondes depuis l'epoch UTC, en nombre ou en chaîne.
fn received_at(m: &Value) -> Option<i64> {
    match m.get("d")? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Début du texte : espaces compactés, 280 caractères au plus ; vide → `None`.
fn preview(m: &Value) -> Option<String> {
    let compact = m
        .get("fr")
        .and_then(Value::as_str)?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let cut: String = compact.chars().take(PREVIEW_CHARS).collect();
    (!cut.is_empty()).then_some(cut)
}

/// Expéditeur : l'adresse de type `f` (from) de `e[]`. Nom : `p` (nom complet), sinon `d`.
/// Sans expéditeur, adresse vide et nom absent.
fn sender(m: &Value) -> MailAddress {
    let from = m.get("e").and_then(Value::as_array).and_then(|list| {
        list.iter()
            .find(|e| e.get("t").and_then(Value::as_str) == Some("f"))
    });
    match from {
        Some(e) => MailAddress {
            name: str_field(e, "p")
                .or_else(|| str_field(e, "d"))
                .map(str::to_owned),
            address: str_field(e, "a").unwrap_or_default().to_owned(),
        },
        None => MailAddress {
            name: None,
            address: String::new(),
        },
    }
}

/// Un message de `SearchResponse.m[]`. `id` et `d` sont obligatoires ; objet, expéditeur, drapeaux
/// et aperçu sont facultatifs (Zimbra omet les attributs vides).
fn parse_message(index: usize, m: &Value, tz: Tz) -> Result<MailMessage, Problem> {
    let id = message_id(m)
        .ok_or_else(|| upstream(PROVIDER, format!("message {index} : champ `id` absent")))?;
    let millis = received_at(m)
        .ok_or_else(|| upstream(PROVIDER, format!("message {index} : champ `d` absent")))?;
    let date = DateTime::from_timestamp_millis(millis)
        .map(|instant| tz.from_utc_datetime(&instant.naive_utc()).fixed_offset())
        .ok_or_else(|| upstream(PROVIDER, format!("message {index} : date hors limites")))?;
    Ok(MailMessage {
        id,
        subject: str_field(m, "su").unwrap_or_default().to_owned(),
        from: sender(m),
        date,
        // Drapeau `u` : non lu.
        unread: m
            .get("f")
            .and_then(Value::as_str)
            .is_some_and(|flags| flags.contains('u')),
        preview: preview(m),
    })
}

/// Messages d'une `SearchResponse`, du plus récent au plus ancien. Zimbra omet `m` quand la boîte
/// est vide.
fn parse_messages(search: &Value, tz: Tz) -> Result<Vec<MailMessage>, Problem> {
    let mut messages = match search.get("m") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(list)) => list
            .iter()
            .enumerate()
            .map(|(index, m)| parse_message(index, m, tz))
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(upstream(PROVIDER, "`m` n'est pas une liste")),
    };
    messages.sort_by_key(|m| std::cmp::Reverse(m.date));
    Ok(messages)
}

/// Nombre de non lus du dossier (`folder[0].u`). Zimbra omet `u` quand il vaut zéro ; `None` si le
/// dossier lui-même est absent.
fn parse_unread(folder_response: &Value) -> Option<u32> {
    let folder = folder_response.get("folder")?.as_array()?.first()?;
    Some(
        folder
            .get("u")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0),
    )
}

/// Boîte complète d'après les deux réponses. Sans réponse de dossier exploitable, le nombre de non
/// lus est celui des messages reçus (un minimum, pas le total de la boîte).
fn build_inbox(
    search: &Value,
    folder: Option<&Value>,
    tz: Tz,
    webmail: &Url,
) -> Result<MailInbox, Problem> {
    let messages = parse_messages(search, tz)?;
    let unread = folder.and_then(parse_unread).unwrap_or_else(|| {
        u32::try_from(messages.iter().filter(|m| m.unread).count()).unwrap_or(u32::MAX)
    });
    Ok(MailInbox {
        unread,
        messages,
        webmail_url: Some(webmail.to_string()),
    })
}

// Réseau

/// Requête SOAP (JSON) vers la racine du serveur : `/service/soap/<nom>`.
async fn soap(net: &mut Net, base: &Url, name: &str, request: Value) -> Result<Value, Failure> {
    let url = base
        .join(&format!("/service/soap/{name}"))
        .map_err(|_| upstream(PROVIDER, "adresse du webmail inutilisable"))?;
    let mut body = json!({
        "Header": { "context": { "_jsns": "urn:zimbra" } },
        "Body": {},
    });
    body["Body"][name] = request;
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    // Comme le client web de Zimbra : une enveloppe JSON sous ce type de contenu.
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/soap+xml; charset=utf-8"),
    );
    if let Ok(referer) = HeaderValue::from_str(base.as_str()) {
        headers.insert(header::REFERER, referer);
    }
    let (status, _, text) = net
        .request(Method::POST, url, headers, Some(body.to_string()))
        .await?;
    parse_envelope(status.as_u16(), &text)
}

/// Lit la boîte avec le cookie de la session. `GetFolderRequest` ne sert qu'au nombre de non lus :
/// son échec ne fait pas échouer la lecture.
async fn read_inbox(net: &mut Net, base: &Url, tz: Tz) -> Result<MailInbox, Failure> {
    let search = soap(
        net,
        base,
        "SearchRequest",
        json!({
            "_jsns": "urn:zimbraMail",
            "types": "message",
            "query": "in:inbox",
            "sortBy": "dateDesc",
            "limit": MAX_MESSAGES,
            "offset": 0,
        }),
    )
    .await?;
    let search = response_body(&search, "SearchResponse")?;
    let folder = soap(
        net,
        base,
        "GetFolderRequest",
        // Dossier 2 : la boîte de réception.
        json!({ "_jsns": "urn:zimbraMail", "folder": { "l": "2" }, "depth": 0 }),
    )
    .await
    .ok();
    let folder = folder
        .as_ref()
        .and_then(|envelope| response_body(envelope, "GetFolderResponse").ok());
    Ok(build_inbox(search, folder, tz, base)?)
}

/// Ouvre la page d'entrée du webmail derrière le CAS et vérifie que Zimbra a posé son cookie.
async fn sign_in(net: &mut Net, base: &Url, entry: &Url) -> Result<(), Problem> {
    // `session_expired` si le CAS redemande le mot de passe ou un second facteur.
    let page = net.open_protected(entry.clone()).await?;
    if net
        .browser
        .jar
        .contains(base.host_str().unwrap_or_default(), TOKEN_COOKIE)
    {
        return Ok(());
    }
    if forms::forms(&page.body)
        .iter()
        .any(|f| f.has("SAMLResponse") || f.has("SAMLRequest"))
    {
        return Err(Problem::new(ErrorCode::NotImplemented).detail(
            "zimbra-rest : la connexion passe par une fédération SAML, pas encore prise en charge",
        ));
    }
    Err(upstream(
        PROVIDER,
        "la connexion n'a pas posé le cookie de session Zimbra",
    ))
}

/// Connexion si besoin, lecture, et un seul nouvel essai si Zimbra refuse le cookie.
async fn run(
    net: &mut Net,
    base: &Url,
    entry: &Url,
    tz: Tz,
    now: i64,
) -> Result<MailInbox, Problem> {
    let host = base.host_str().unwrap_or_default().to_owned();
    let mut signed_in_now = false;
    if !net.browser.jar.contains(&host, TOKEN_COOKIE) {
        sign_in(net, base, entry).await?;
        signed_in_now = true;
    }
    loop {
        match read_inbox(net, base, tz).await {
            Ok(inbox) => return Ok(inbox),
            Err(Failure::Problem(problem)) => return Err(problem),
            Err(Failure::Expired) if signed_in_now => {
                return Err(upstream(
                    PROVIDER,
                    "Zimbra refuse le cookie de session obtenu à l'instant",
                ));
            }
            Err(Failure::Expired) => {
                // Cookie périmé : on l'oublie (lui seul, pas le TGC éventuellement posé pour tout
                // le domaine), on rejoue la connexion, puis on réessaie une fois.
                net.browser
                    .jar
                    .store(base, &format!("{TOKEN_COOKIE}=; Path=/; Max-Age=0"), now);
                sign_in(net, base, entry).await?;
                signed_in_now = true;
            }
        }
    }
}

/// Boîte de réception de l'étudiant. `base_url` est la racine du webmail, `entry_url` la page qui
/// lance la connexion (défaut : `base_url`).
pub(crate) async fn fetch_inbox(
    ctx: &mut Context<'_>,
    base_url: &Url,
    entry_url: Option<&Url>,
) -> Result<MailInbox, Problem> {
    let tz = university_tz(ctx)?;
    let mut net = Net::open(ctx, PROVIDER)?;
    let now = ctx.now.timestamp();
    let result = run(&mut net, base_url, entry_url.unwrap_or(base_url), tz, now).await;
    // Les cookies renouvelés repartent dans la session même si la lecture a échoué.
    net.save(ctx);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Europe::Paris;

    const SEARCH: &str = include_str!("../tests/fixtures/zimbra/search_response.json");
    const SEARCH_VIDE: &str = include_str!("../tests/fixtures/zimbra/search_vide.json");
    const FOLDER: &str = include_str!("../tests/fixtures/zimbra/folder_response.json");
    const FOLDER_ZERO: &str = include_str!("../tests/fixtures/zimbra/folder_sans_non_lus.json");
    const FAULT_AUTH: &str = include_str!("../tests/fixtures/zimbra/fault_session_expiree.json");
    const FAULT_AUTRE: &str = include_str!("../tests/fixtures/zimbra/fault_autre.json");

    fn json(source: &str) -> Value {
        serde_json::from_str(source).unwrap()
    }

    fn webmail() -> Url {
        Url::parse("https://mail.example.org/").unwrap()
    }

    fn inbox(search: &str, folder: Option<&str>) -> MailInbox {
        let search = json(search);
        let folder = folder.map(json);
        build_inbox(
            response_body(&search, "SearchResponse").unwrap(),
            folder
                .as_ref()
                .map(|f| response_body(f, "GetFolderResponse").unwrap()),
            Paris,
            &webmail(),
        )
        .unwrap()
    }

    #[test]
    fn boite_normale() {
        let inbox = inbox(SEARCH, Some(FOLDER));
        assert_eq!(inbox.unread, 2);
        assert_eq!(
            inbox.webmail_url.as_deref(),
            Some("https://mail.example.org/")
        );
        assert_eq!(inbox.messages.len(), 4);

        let first = &inbox.messages[0];
        assert_eq!(first.id, "4211");
        assert_eq!(first.subject, "Réunion de rentrée");
        assert!(first.unread);
        assert_eq!(first.from.address, "secretariat.demo@example.org");
        assert_eq!(first.from.name.as_deref(), Some("Secrétariat Demo"));
        assert_eq!(
            first.preview.as_deref(),
            Some("Bonjour Camille, la réunion de rentrée aura lieu mardi en salle B12.")
        );

        // Triés du plus récent au plus ancien, quel que soit l'ordre reçu.
        let ids: Vec<&str> = inbox.messages.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["4211", "4150", "4198", "4101"]);

        let second = inbox.messages.iter().find(|m| m.id == "4198").unwrap();
        assert!(!second.unread);
        // Pas de `p` : le nom vient de `d`.
        assert_eq!(second.from.name.as_deref(), Some("Scolarité Demo"));
    }

    #[test]
    fn dates_dans_le_fuseau_de_l_universite() {
        let inbox = inbox(SEARCH, Some(FOLDER));
        let dates: Vec<String> = inbox.messages.iter().map(|m| m.date.to_rfc3339()).collect();
        assert_eq!(
            dates,
            [
                // Lendemain du passage à l'heure d'hiver (25 octobre 2026) : +01:00.
                "2026-10-26T08:15:00+01:00",
                // Une demi-heure avant le changement (00:30 UTC) : encore +02:00.
                "2026-10-25T02:30:00+02:00",
                // Veille : encore l'heure d'été.
                "2026-10-24T20:30:00+02:00",
                // Nuit du passage à l'heure d'été (29 mars 2026) : 23:30 UTC → 00:30 à +01:00.
                "2026-03-29T00:30:00+01:00",
            ]
        );
        // Un autre fuseau change le décalage, pas l'instant.
        let search = json(SEARCH);
        let reunion = parse_messages(
            response_body(&search, "SearchResponse").unwrap(),
            chrono_tz::America::Martinique,
        )
        .unwrap();
        assert_eq!(reunion[0].date.to_rfc3339(), "2026-10-26T03:15:00-04:00");
    }

    #[test]
    fn champs_facultatifs_absents() {
        let inbox = inbox(SEARCH, Some(FOLDER));
        // `4150` n'a que `id` et `d` : ni objet, ni expéditeur, ni drapeau, ni aperçu.
        let bare = inbox.messages.iter().find(|m| m.id == "4150").unwrap();
        assert_eq!(bare.subject, "");
        assert_eq!(bare.from.address, "");
        assert_eq!(bare.from.name, None);
        assert!(!bare.unread);
        assert_eq!(bare.preview, None);
        // `4101` : nom `p` vide et pas de `d` → pas de nom, mais l'adresse reste.
        let welcome = inbox.messages.iter().find(|m| m.id == "4101").unwrap();
        assert_eq!(welcome.from.name, None);
        assert_eq!(welcome.from.address, "accueil.demo@example.org");
        // Drapeaux `ur` : non lu même si répondu.
        assert!(welcome.unread);
    }

    #[test]
    fn boite_vide() {
        let inbox = inbox(SEARCH_VIDE, Some(FOLDER_ZERO));
        assert!(inbox.messages.is_empty());
        assert_eq!(inbox.unread, 0);
    }

    #[test]
    fn non_lus_du_dossier() {
        let folder = json(FOLDER);
        assert_eq!(
            parse_unread(response_body(&folder, "GetFolderResponse").unwrap()),
            Some(2)
        );
        // `u` omis : zéro non lu.
        let folder = json(FOLDER_ZERO);
        assert_eq!(
            parse_unread(response_body(&folder, "GetFolderResponse").unwrap()),
            Some(0)
        );
        // Sans dossier : pas de réponse exploitable.
        assert_eq!(parse_unread(&json(r#"{"_jsns":"urn:zimbraMail"}"#)), None);
    }

    #[test]
    fn non_lus_comptes_sur_les_messages_sans_dossier() {
        let inbox = inbox(SEARCH, None);
        assert_eq!(inbox.unread, 2);
    }

    #[test]
    fn apercu_borne_et_compacte() {
        let long = "mot ".repeat(200);
        let m = json(&format!(r#"{{"id":"1","d":1792998900000,"fr":{long:?}}}"#));
        let message = parse_message(0, &m, Paris).unwrap();
        let preview = message.preview.unwrap();
        assert_eq!(preview.chars().count(), PREVIEW_CHARS);
        assert!(!preview.contains("  "));
        // Un aperçu blanc n'est pas un aperçu.
        let m = json(r#"{"id":"1","d":1792998900000,"fr":" \n "}"#);
        assert_eq!(parse_message(0, &m, Paris).unwrap().preview, None);
    }

    #[test]
    fn id_ou_date_absents_sont_une_erreur_de_format() {
        for source in [
            r#"{"d":1792998900000}"#,
            r#"{"id":"","d":1792998900000}"#,
            r#"{"id":"1"}"#,
            r#"{"id":"1","d":"hier"}"#,
            r#"{"id":"1","d":9223372036854775807}"#,
        ] {
            let error = parse_message(3, &json(source), Paris).unwrap_err();
            assert_eq!(error.code, ErrorCode::UpstreamChanged, "{source}");
            // Le message cite la position et le champ, jamais une valeur.
            let detail = error.detail.unwrap_or_default();
            assert!(detail.contains("message 3"), "{detail}");
        }
    }

    #[test]
    fn id_numerique_accepte() {
        let m = json(r#"{"id":4211,"d":"1792998900000"}"#);
        assert_eq!(parse_message(0, &m, Paris).unwrap().id, "4211");
    }

    #[test]
    fn liste_de_messages_illisible() {
        let search = json(r#"{"m":"oups"}"#);
        let error = parse_messages(&search, Paris).unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamChanged);
        let search = json(r#"{"m":[{"id":"1"}]}"#);
        assert!(parse_messages(&search, Paris).is_err());
    }

    #[test]
    fn corps_de_reponse_manquant() {
        let error = response_body(&json(r#"{"Body":{}}"#), "SearchResponse").unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamChanged);
        assert!(response_body(&json("{}"), "SearchResponse").is_err());
    }

    #[test]
    fn faute_d_authentification_signale_une_session_perdue() {
        // Zimbra répond HTTP 500 avec la faute : elle prime sur le statut.
        for status in [200, 401, 500] {
            assert!(
                matches!(parse_envelope(status, FAULT_AUTH), Err(Failure::Expired)),
                "{status}"
            );
        }
        let expired = FAULT_AUTH.replace("AUTH_REQUIRED", "AUTH_EXPIRED");
        assert!(matches!(
            parse_envelope(500, &expired),
            Err(Failure::Expired)
        ));
    }

    #[test]
    fn autre_faute_ne_donne_que_son_code() {
        let Err(Failure::Problem(problem)) = parse_envelope(500, FAULT_AUTRE) else {
            panic!("une autre faute n'est pas une session perdue");
        };
        assert_eq!(problem.code, ErrorCode::UpstreamChanged);
        let detail = problem.detail.unwrap_or_default();
        assert!(detail.contains("service.FAILURE"));
        // Le texte libre de la faute n'est pas repris.
        assert!(!detail.contains("system failure"));
    }

    #[test]
    fn code_de_faute_suspect_masque() {
        let envelope =
            json(r#"{"Body":{"Fault":{"Detail":{"Error":{"Code":"x@example.org <script>"}}}}}"#);
        assert_eq!(fault_code(&envelope).as_deref(), Some("inconnu"));
    }

    #[test]
    fn statuts_sans_faute() {
        assert!(matches!(parse_envelope(401, "{}"), Err(Failure::Expired)));
        // Redirection vers la page de connexion, corps HTML.
        assert!(matches!(
            parse_envelope(302, "<html>…</html>"),
            Err(Failure::Expired)
        ));
        let Err(Failure::Problem(problem)) = parse_envelope(503, "<html>indisponible</html>")
        else {
            panic!("un 503 n'est pas une session perdue");
        };
        assert_eq!(problem.code, ErrorCode::ServiceUnavailable);
        let Err(Failure::Problem(problem)) = parse_envelope(200, "<html>page de connexion</html>")
        else {
            panic!("une page HTML n'est pas une réponse SOAP");
        };
        assert_eq!(problem.code, ErrorCode::UpstreamChanged);
        // Le corps reçu n'est jamais repris dans l'erreur.
        assert!(
            !problem
                .detail
                .unwrap_or_default()
                .contains("page de connexion")
        );
        assert!(parse_envelope(200, SEARCH).is_ok());
    }
}
