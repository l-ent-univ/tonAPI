//! Fournisseur `mon-dossier-web` : certificats de scolarité d'ESUP MonDossierWeb
//! (`fr.univlorraine.mondossierweb`, Vaadin 8), derrière le CAS de l'université.
//!
//! L'application n'a pas d'API de téléchargement : le lien d'un PDF n'existe que dans une interface
//! Vaadin ouverte. Le fournisseur tient donc le rôle du navigateur, avec la session de l'étudiant :
//! 1. `GET <base>` → page contenant `vaadin.initApplication("<appId>", {…})` ;
//! 2. `POST <base>?v-<ms>` (détails du navigateur, formulaire) → `{"v-uiId": n, "uidl": "<JSON en
//!    chaîne>"}` ; dans `uidl` : `Vaadin-Security-Key` (jeton CSRF), `syncId`, `clientId` et
//!    `state`, l'état des composants par identifiant ;
//! 3. `POST <base>UIDL/?v-uiId=n` (JSON) : un clic sur le bouton de menu « Inscriptions » ; la
//!    réponse, préfixée `for(;;);`, décrit un `FileDownloader` par fichier
//!    (`state[*].resources.dl.uRL = "app://APP/connector/…/<fichier>.pdf"`) ;
//! 4. `GET <base>APP/connector/…` : le PDF.
//!
//! Chaque lecture ouvre une interface neuve : les identifiants des composants et les chemins des
//! fichiers ne valent que pour elle. L'identifiant d'un document (`certificat-2026`) n'en reprend
//! donc rien : le téléchargement rouvre une interface et y retrouve le certificat de la même année.
//! Deux interfaces ouvertes en même temps avec la même session se gênent (elles partagent la session
//! Vaadin et son `syncId`) : au client de ne pas demander deux lectures à la fois.
//!
//! [code] Porté du serveur de tafac (`server/schoolCertificate.js`, Université de Rennes) ; formats
//! dans `docs/recherche/rennes/ent.md` §7. Jamais essayé par tonAPI avec un vrai compte. Le libellé
//! « Inscriptions » et le thème `valo-ul` sont ceux de ce déploiement ; qu'un autre les change
//! [inconnu].

use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tonapi_core::{Document, DocumentKind, ErrorCode, Problem};
use url::Url;

use crate::common::{id_string, root, str_field};
use crate::net::{Net, upstream};
use crate::{Context, Download};

const PROVIDER: &str = "mon-dossier-web";
/// Libellé du bouton de menu qui liste les inscriptions et leurs certificats.
const MENU_CAPTION: &str = "Inscriptions";
/// Thème envoyé quand la page ne dit pas le sien.
const DEFAULT_THEME: &str = "valo-ul";
/// Préfixe que Vaadin met devant ses réponses JSON.
const GUARD: &str = "for(;;);";
/// Adresse d'un fichier servi par l'interface, relative à la racine de l'application.
const APP_SCHEME: &str = "app://";
/// Nom du fichier rendu quand celui de l'application est inutilisable.
const FALLBACK_FILE_NAME: &str = "certificat-de-scolarite.pdf";

fn bad(what: impl std::fmt::Display) -> Problem {
    upstream(PROVIDER, what)
}

fn unknown() -> Problem {
    Problem::new(ErrorCode::NotFound).detail("document inconnu")
}

// Analyse pure

/// Ce que la page d'accueil dit de l'application Vaadin.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct App {
    pub id: String,
    pub theme: String,
}

/// Texte entre guillemets qui suit `marker`.
fn quoted_after<'a>(html: &'a str, marker: &str) -> Option<&'a str> {
    let rest = html.get(html.find(marker)? + marker.len()..)?;
    let value = rest.split('"').next()?;
    (!value.is_empty() && value.len() <= 200).then_some(value)
}

/// `vaadin.initApplication("<appId>", {…"theme":"<thème>"…})`. Sans cet appel, la page n'est pas
/// l'application (page d'erreur, maintenance).
pub(crate) fn parse_page(html: &str) -> Result<App, Problem> {
    let id = quoted_after(html, "vaadin.initApplication(\"")
        .ok_or_else(|| bad("page sans application Vaadin"))?;
    let theme = quoted_after(html, "\"theme\":\"").unwrap_or(DEFAULT_THEME);
    Ok(App {
        id: id.to_owned(),
        theme: theme.to_owned(),
    })
}

/// Corps de la requête qui ouvre l'interface : ce que le client Vaadin dit du navigateur. Les
/// dimensions et le fuseau ne servent qu'à la mise en page.
pub(crate) fn details_body(app: &App, page: &Url, now_ms: i64) -> String {
    let now = now_ms.to_string();
    let window = format!("{}-0.5", app.id);
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("v-browserDetails", "1"),
            ("theme", app.theme.as_str()),
            ("v-appId", app.id.as_str()),
            ("v-sh", "900"),
            ("v-sw", "1440"),
            ("v-cw", "1440"),
            ("v-ch", "900"),
            ("v-tzo", "-60"),
            ("v-dstd", "60"),
            ("v-rtzo", "-60"),
            ("v-dston", "false"),
            ("v-tzid", "Europe/Paris"),
            ("v-vw", "1440"),
            ("v-vh", "0"),
            ("v-curdate", now.as_str()),
            ("v-loc", page.as_str()),
            ("v-wn", window.as_str()),
        ])
        .finish()
}

fn without_guard(body: &str) -> &str {
    let body = body.trim_start();
    body.strip_prefix(GUARD).unwrap_or(body)
}

/// Interface ouverte : de quoi lui envoyer un clic.
#[derive(Debug)]
pub(crate) struct Ui {
    pub id: String,
    pub csrf: String,
    pub sync_id: i64,
    pub client_id: i64,
    /// État des composants, par identifiant.
    pub state: Value,
}

/// Réponse de l'ouverture : `{"v-uiId": 3, "uidl": "<JSON en chaîne>"}`.
pub(crate) fn parse_ui(body: &str) -> Result<Ui, Problem> {
    let payload: Value = serde_json::from_str(without_guard(body))
        .map_err(|_| bad("ouverture de l'interface : réponse non JSON"))?;
    let uidl: Value = payload
        .get("uidl")
        .and_then(Value::as_str)
        .and_then(|text| serde_json::from_str(text).ok())
        .ok_or_else(|| bad("ouverture de l'interface : `uidl` absent ou illisible"))?;
    let id = id_string(payload.get("v-uiId"))
        .ok_or_else(|| bad("ouverture de l'interface : `v-uiId` absent"))?;
    let csrf = str_field(&uidl, "Vaadin-Security-Key")
        .ok_or_else(|| bad("ouverture de l'interface : jeton CSRF absent"))?
        .to_owned();
    let counter = |key: &str| uidl.get(key).and_then(Value::as_i64).unwrap_or(0);
    Ok(Ui {
        id,
        csrf,
        sync_id: counter("syncId"),
        client_id: counter("clientId"),
        state: uidl.get("state").cloned().unwrap_or(Value::Null),
    })
}

/// Composants d'un `state`, dans l'ordre de leurs identifiants (des entiers).
fn components(state: &Value) -> Vec<(&String, &Value)> {
    let mut list: Vec<(&String, &Value)> = state
        .as_object()
        .map(|map| map.iter().collect())
        .unwrap_or_default();
    list.sort_by(|(a, _), (b, _)| {
        let number = |key: &str| key.parse::<u64>().unwrap_or(u64::MAX);
        number(a).cmp(&number(b)).then_with(|| a.cmp(b))
    });
    list
}

/// Identifiant du composant dont le libellé est `caption`, sans tenir compte de la casse.
pub(crate) fn menu_button(state: &Value, caption: &str) -> Option<String> {
    let wanted = caption.trim().to_lowercase();
    components(state)
        .into_iter()
        .find(|(_, component)| {
            component
                .get("caption")
                .and_then(Value::as_str)
                .is_some_and(|found| found.trim().to_lowercase() == wanted)
        })
        .map(|(id, _)| id.clone())
}

/// Corps de la requête UIDL : un clic gauche sur le bouton.
pub(crate) fn click_body(ui: &Ui, button: &str) -> String {
    json!({
        "csrfToken": ui.csrf,
        "rpc": [[button, "com.vaadin.shared.ui.button.ButtonServerRpc", "click", [{
            "altKey": false, "button": "LEFT", "clientX": 1, "clientY": 1, "ctrlKey": false,
            "metaKey": false, "relativeX": 1, "relativeY": 1, "shiftKey": false, "type": 1,
        }]]],
        "syncId": ui.sync_id,
        "clientId": ui.client_id,
    })
    .to_string()
}

/// Certificat proposé par l'interface ouverte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Certificate {
    /// Identifiant du document, stable d'une interface à l'autre.
    pub id: String,
    /// Chemin du fichier sous la racine de l'application, valable pour cette interface seulement.
    pub path: String,
    pub file_name: String,
    /// Année universitaire lue dans le nom du fichier : début et fin (`2026`, `2027`).
    pub years: Option<(String, String)>,
}

/// Octets d'un texte encodé en pourcents ; une séquence mal formée est gardée telle quelle.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        let escaped = (byte == b'%')
            .then(|| text.get(at + 1..at + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(value) = escaped {
            decoded.push(value);
            at += 3;
        } else {
            decoded.push(byte);
            at += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// Première année universitaire écrite `AAAA-AAAA` dans un nom de fichier.
fn school_years(file_name: &str) -> Option<(String, String)> {
    let bytes = file_name.as_bytes();
    let digits = |from: usize| {
        bytes
            .get(from..from + 4)
            .is_some_and(|part| part.iter().all(u8::is_ascii_digit))
    };
    (0..bytes.len()).find_map(|at| {
        if digits(at) && bytes.get(at + 4) == Some(&b'-') && digits(at + 5) {
            let start = file_name.get(at..at + 4)?;
            let end = file_name.get(at + 5..at + 9)?;
            Some((start.to_owned(), end.to_owned()))
        } else {
            None
        }
    })
}

/// Nom de fichier sans séparateur de chemin ni caractère de contrôle.
fn clean_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\'))
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.');
    if cleaned.is_empty() {
        FALLBACK_FILE_NAME.to_owned()
    } else {
        cleaned.to_owned()
    }
}

/// Réponse au clic sur « Inscriptions » : les fichiers téléchargeables dont le nom est celui d'un
/// certificat, l'année universitaire la plus récente d'abord. Les autres fichiers (relevés,
/// quittances) sont ignorés.
pub(crate) fn parse_certificates(body: &str) -> Result<Vec<Certificate>, Problem> {
    let value: Value = serde_json::from_str(without_guard(body))
        .map_err(|_| bad("menu des inscriptions : réponse non JSON"))?;
    let messages: Vec<&Value> = match &value {
        Value::Array(list) => list.iter().collect(),
        Value::Object(_) => vec![&value],
        _ => return Err(bad("menu des inscriptions : réponse inattendue")),
    };
    let mut found = Vec::new();
    for message in messages {
        let Some(state) = message.get("state") else {
            continue;
        };
        for (_, component) in components(state) {
            let Some(path) = component
                .pointer("/resources/dl/uRL")
                .and_then(Value::as_str)
                .and_then(|url| url.strip_prefix(APP_SCHEME))
            else {
                continue;
            };
            let file_name = percent_decoded(path.rsplit('/').next().unwrap_or_default());
            if !file_name.to_lowercase().contains("certificat") {
                continue;
            }
            found.push(Certificate {
                id: String::new(),
                path: path.to_owned(),
                years: school_years(&file_name),
                file_name: clean_file_name(&file_name),
            });
        }
    }
    // Tri stable : à année égale, l'ordre de l'interface.
    found.sort_by(|a, b| {
        let start = |c: &Certificate| c.years.as_ref().map(|(start, _)| start.clone());
        start(b).cmp(&start(a))
    });
    let mut seen: Vec<String> = Vec::new();
    for certificate in &mut found {
        let base = match &certificate.years {
            Some((start, _)) => format!("certificat-{start}"),
            None => "certificat".to_owned(),
        };
        let rank = seen.iter().filter(|other| **other == base).count() + 1;
        certificate.id = if rank == 1 {
            base.clone()
        } else {
            format!("{base}.{rank}")
        };
        seen.push(base);
    }
    Ok(found)
}

fn document_of(certificate: &Certificate) -> Document {
    let title = match &certificate.years {
        Some((start, end)) => format!("Certificat de scolarité {start}-{end}"),
        None => "Certificat de scolarité".to_owned(),
    };
    Document {
        id: certificate.id.clone(),
        title,
        kind: DocumentKind::Certificate,
        year: certificate.years.as_ref().map(|(start, _)| start.clone()),
        session: None,
        downloadable: true,
        url: None,
    }
}

/// Adresse d'un fichier de l'interface : sous la racine de l'application, et nulle part ailleurs.
pub(crate) fn file_url(base: &Url, path: &str) -> Result<Url, Problem> {
    let outside = || bad("fichier hors de l'application");
    if path.starts_with('/') || path.contains("..") || path.contains('\\') {
        return Err(outside());
    }
    let url = base.join(path).map_err(|_| outside())?;
    let same_origin = url.scheme() == base.scheme()
        && url.host_str() == base.host_str()
        && url.port_or_known_default() == base.port_or_known_default();
    if same_origin && url.path().starts_with(base.path()) {
        Ok(url)
    } else {
        Err(outside())
    }
}

// Récupération

fn status_problem(status: StatusCode, what: &str) -> Problem {
    if status.is_server_error() {
        Problem::new(ErrorCode::ServiceUnavailable)
            .detail(format!("{PROVIDER} : {what} : HTTP {status}"))
    } else {
        bad(format!("{what} : HTTP {status}"))
    }
}

fn headers(base: &Url, content_type: Option<&'static str>, accept: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static(accept));
    if let Some(content_type) = content_type {
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    }
    if let Ok(referer) = HeaderValue::from_str(base.as_str()) {
        headers.insert(header::REFERER, referer);
    }
    headers
}

/// Ouvre une interface et lit les certificats de l'onglet « Inscriptions » (étapes 1 à 3).
async fn certificates(net: &mut Net, base: &Url, now_ms: i64) -> Result<Vec<Certificate>, Problem> {
    let page = net.open_protected(base.clone()).await?;
    if page.url.host_str() != base.host_str() {
        return Err(bad("la connexion n'a pas abouti sur l'application"));
    }
    if !page.status.is_success() {
        return Err(status_problem(page.status, "page d'accueil"));
    }
    let app = parse_page(&page.body)?;

    let mut opening = base.clone();
    opening.set_query(Some(&format!("v-{now_ms}")));
    let (status, _, body) = net
        .request(
            Method::POST,
            opening,
            headers(
                base,
                Some("application/x-www-form-urlencoded"),
                "application/json",
            ),
            Some(details_body(&app, base, now_ms)),
        )
        .await?;
    if !status.is_success() {
        return Err(status_problem(status, "ouverture de l'interface"));
    }
    let ui = parse_ui(&body)?;
    let button = menu_button(&ui.state, MENU_CAPTION)
        .ok_or_else(|| bad("bouton « Inscriptions » absent de l'interface"))?;

    let mut uidl = base
        .join("UIDL/")
        .map_err(|_| bad("adresse de l'application illisible"))?;
    uidl.query_pairs_mut().append_pair("v-uiId", &ui.id);
    let (status, _, body) = net
        .request(
            Method::POST,
            uidl,
            headers(
                base,
                Some("application/json; charset=UTF-8"),
                "application/json",
            ),
            Some(click_body(&ui, &button)),
        )
        .await?;
    if !status.is_success() {
        return Err(status_problem(status, "menu des inscriptions"));
    }
    parse_certificates(&body)
}

/// Certificats de scolarité proposés à l'étudiant, le plus récent d'abord. Une liste vide : aucun
/// certificat pour ce compte.
pub(crate) async fn fetch_documents(
    ctx: &mut Context<'_>,
    base_url: &Url,
) -> Result<Vec<Document>, Problem> {
    let base = root(base_url);
    let now_ms = ctx.now.timestamp_millis();
    let mut net = Net::open(ctx, PROVIDER)?;
    let result = certificates(&mut net, &base, now_ms).await;
    net.save(ctx);
    Ok(result?.iter().map(document_of).collect())
}

/// PDF d'un certificat de [`fetch_documents`] : une interface est rouverte pour retrouver son
/// fichier.
pub(crate) async fn download(
    ctx: &mut Context<'_>,
    base_url: &Url,
    id: &str,
) -> Result<Download, Problem> {
    let well_formed = !id.is_empty()
        && id.len() <= 40
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.'));
    if !well_formed {
        return Err(unknown());
    }
    let base = root(base_url);
    let now_ms = ctx.now.timestamp_millis();
    let mut net = Net::open(ctx, PROVIDER)?;
    let result = async {
        let found = certificates(&mut net, &base, now_ms).await?;
        let certificate = found.iter().find(|c| c.id == id).ok_or_else(unknown)?;
        let url = file_url(&base, &certificate.path)?;
        let (status, _, bytes) = net
            .download(url, headers(&base, None, "application/pdf,*/*"))
            .await?;
        match status {
            s if s.is_success() && bytes.starts_with(b"%PDF") => Ok(Download {
                content_type: "application/pdf".to_owned(),
                file_name: certificate.file_name.clone(),
                bytes,
            }),
            s if s.is_success() => Err(bad("le téléchargement n'est pas un PDF")),
            s => Err(status_problem(s, "téléchargement")),
        }
    }
    .await;
    net.save(ctx);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = include_str!("../tests/fixtures/mon_dossier_web/page.html");
    const OPENING: &str = include_str!("../tests/fixtures/mon_dossier_web/ouverture.json");
    const MENU: &str = include_str!("../tests/fixtures/mon_dossier_web/inscriptions.json");

    #[test]
    fn page_de_l_application_et_details_du_navigateur() {
        let app = parse_page(PAGE).unwrap();
        assert_eq!(
            app,
            App {
                id: "ROOT-1234567".to_owned(),
                theme: "valo-ul".to_owned()
            }
        );
        // Sans thème annoncé : celui du logiciel.
        let bare = parse_page(r#"<script>vaadin.initApplication("mdw-42",{});</script>"#).unwrap();
        assert_eq!(bare.theme, "valo-ul");
        // Formulaire du CAS ou page d'erreur : pas l'application.
        for other in [
            r#"<form id="fm1"><input name="username"></form>"#,
            "",
            r#"vaadin.initApplication("")"#,
        ] {
            assert_eq!(
                parse_page(other).unwrap_err().code,
                ErrorCode::UpstreamChanged
            );
        }

        let page = Url::parse("https://mdw.exemple.test/").unwrap();
        let body = details_body(&app, &page, 42);
        let fields: Vec<(String, String)> = url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect();
        let field = |name: &str| {
            fields
                .iter()
                .find(|(found, _)| found == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(field("v-browserDetails"), Some("1"));
        assert_eq!(field("theme"), Some("valo-ul"));
        assert_eq!(field("v-appId"), Some("ROOT-1234567"));
        assert_eq!(field("v-curdate"), Some("42"));
        assert_eq!(field("v-loc"), Some("https://mdw.exemple.test/"));
        assert_eq!(field("v-wn"), Some("ROOT-1234567-0.5"));
    }

    #[test]
    fn interface_ouverte_bouton_du_menu_et_clic() {
        let ui = parse_ui(OPENING).unwrap();
        assert_eq!(
            (ui.id.as_str(), ui.csrf.as_str(), ui.sync_id, ui.client_id),
            ("3", "jeton-csrf-invente", 0, 0)
        );
        assert_eq!(
            menu_button(&ui.state, "inscriptions").as_deref(),
            Some("547")
        );
        assert_eq!(
            menu_button(&ui.state, " INSCRIPTIONS ").as_deref(),
            Some("547")
        );
        assert_eq!(menu_button(&ui.state, "Absences"), None);
        assert_eq!(menu_button(&Value::Null, "Inscriptions"), None);

        let click: Value = serde_json::from_str(&click_body(&ui, "547")).unwrap();
        assert_eq!(click["csrfToken"], "jeton-csrf-invente");
        assert_eq!(click["rpc"][0][0], "547");
        assert_eq!(
            click["rpc"][0][1],
            "com.vaadin.shared.ui.button.ButtonServerRpc"
        );
        assert_eq!(click["rpc"][0][2], "click");
        assert_eq!(click["rpc"][0][3][0]["button"], "LEFT");
        assert_eq!(
            (&click["syncId"], &click["clientId"]),
            (&json!(0), &json!(0))
        );
    }

    #[test]
    fn ouverture_prefixee_ou_incomplete() {
        // Vaadin préfixe parfois aussi cette réponse.
        assert!(parse_ui(&format!("for(;;);{OPENING}")).is_ok());
        for broken in [
            "{}",
            "<html>Session expirée</html>",
            r#"{"v-uiId":3,"uidl":"pas du JSON"}"#,
            r#"{"v-uiId":3,"uidl":"{\"syncId\":0}"}"#,
            r#"{"uidl":"{\"Vaadin-Security-Key\":\"k\"}"}"#,
        ] {
            assert_eq!(
                parse_ui(broken).unwrap_err().code,
                ErrorCode::UpstreamChanged,
                "{broken}"
            );
        }
    }

    #[test]
    fn certificats_seuls_le_plus_recent_d_abord() {
        let found = parse_certificates(MENU).unwrap();
        let ids: Vec<&str> = found.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["certificat-2026", "certificat-2025"]);
        assert_eq!(
            found[0].path,
            "APP/connector/3/646/dl/Certificat_de_Scolarit%C3%A9_X0001_2026-2027_CAMILLE_DEMO.pdf"
        );
        assert_eq!(
            found[0].file_name,
            "Certificat_de_Scolarité_X0001_2026-2027_CAMILLE_DEMO.pdf"
        );
        let documents: Vec<Document> = found.iter().map(document_of).collect();
        assert_eq!(documents[0].title, "Certificat de scolarité 2026-2027");
        assert_eq!(documents[0].year.as_deref(), Some("2026"));
        assert_eq!(documents[0].kind, DocumentKind::Certificate);
        assert!(documents[0].downloadable && documents[0].url.is_none());
        // Ni le nom ni le numéro de l'étudiant dans l'identifiant.
        assert!(!documents[0].id.contains("DEMO") && !documents[0].id.contains("X0001"));
    }

    #[test]
    fn certificats_de_la_meme_annee_sans_annee_ou_absents() {
        let state = |files: &[&str]| {
            let components: serde_json::Map<String, Value> = files
                .iter()
                .enumerate()
                .map(|(index, file)| {
                    let url = format!("app://APP/connector/0/{index}/dl/{file}");
                    (
                        (700 + index).to_string(),
                        json!({ "resources": { "dl": { "uRL": url } } }),
                    )
                })
                .collect();
            format!("for(;;);[{}]", json!({ "state": components }))
        };
        let found = parse_certificates(&state(&[
            "Certificat_A_2026-2027.pdf",
            "certificat.pdf",
            "CERTIFICAT_B_2026-2027.pdf",
            "Quittance_2026-2027.pdf",
        ]))
        .unwrap();
        let ids: Vec<&str> = found.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["certificat-2026", "certificat-2026.2", "certificat"]);
        assert_eq!(document_of(&found[2]).title, "Certificat de scolarité");
        assert_eq!(document_of(&found[2]).year, None);

        // Aucun fichier, un objet seul, des composants sans lien : aucune erreur.
        for empty in [
            "for(;;);[{\"state\":{}}]",
            "[]",
            r#"{"state":{"1":{"caption":"x"}}}"#,
            r#"[{"state":{"1":{"resources":{"dl":{"uRL":"https://ailleurs.test/certificat.pdf"}}}}}]"#,
        ] {
            assert!(parse_certificates(empty).unwrap().is_empty(), "{empty}");
        }
        for broken in ["<html>", "42", ""] {
            assert_eq!(
                parse_certificates(broken).unwrap_err().code,
                ErrorCode::UpstreamChanged,
                "{broken}"
            );
        }
    }

    #[test]
    fn nom_de_fichier_et_adresse_restent_dans_l_application() {
        assert_eq!(percent_decoded("Scolarit%C3%A9%20%zz%4"), "Scolarité %zz%4");
        assert_eq!(clean_file_name("../a\\b\u{7}.pdf"), "ab.pdf");
        assert_eq!(clean_file_name(" "), FALLBACK_FILE_NAME);
        assert_eq!(
            school_years("Certificat_2026-2027_x"),
            Some(("2026".to_owned(), "2027".to_owned()))
        );
        assert_eq!(school_years("Certificat_2026_2027"), None);
        assert_eq!(school_years("é2026-20"), None);

        let base = Url::parse("https://mdw.exemple.test/mdw/").unwrap();
        assert_eq!(
            file_url(&base, "APP/connector/3/646/dl/c.pdf")
                .unwrap()
                .as_str(),
            "https://mdw.exemple.test/mdw/APP/connector/3/646/dl/c.pdf"
        );
        for outside in [
            "/APP/c.pdf",
            "//ailleurs.test/c.pdf",
            "../autre/c.pdf",
            "APP/../../c.pdf",
            "https://ailleurs.test/c.pdf",
            "APP\\c.pdf",
        ] {
            assert_eq!(
                file_url(&base, outside).unwrap_err().code,
                ErrorCode::UpstreamChanged,
                "{outside}"
            );
        }
    }
}
