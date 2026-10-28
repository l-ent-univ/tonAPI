//! Connexion CAS : suit une chaîne de redirections et répond aux pages du CAS comme un navigateur.
//!
//! Règles de sécurité :
//! - le mot de passe n'est posté qu'une fois par tentative (risque de verrouillage du compte) ;
//! - seulement vers l'hôte exact du CAS, en `https` ;
//! - si le formulaire revient après le POST, les identifiants sont refusés : pas de nouvel essai.

use serde::{Deserialize, Serialize};
use tonapi_config::CasFlavor;
use url::Url;

use crate::browser::{Browser, HttpError, Page};
use crate::forms::{self, Form};

/// Serveur CAS.
#[derive(Debug, Clone)]
pub struct CasServer {
    /// Racine, avec une barre finale (ex. `https://cas.exemple.fr/cas/`).
    pub base: Url,
    pub flavor: CasFlavor,
}

impl CasServer {
    #[must_use]
    pub fn new(mut base: Url, flavor: CasFlavor) -> Self {
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        CasServer { base, flavor }
    }

    /// Même hôte et même port que le CAS.
    #[must_use]
    pub fn owns(&self, url: &Url) -> bool {
        url.host_str() == self.base.host_str()
            && url.port_or_known_default() == self.base.port_or_known_default()
    }

    /// `<base>login`, avec un service facultatif.
    #[must_use]
    pub fn login_url(&self, service: Option<&Url>) -> Url {
        let mut url = self
            .base
            .join("login")
            .unwrap_or_else(|_| self.base.clone());
        if let Some(service) = service {
            url.query_pairs_mut()
                .append_pair("service", service.as_str());
        }
        url
    }
}

/// Second facteur esup-otp demandé après le mot de passe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OtpChallenge {
    /// Page « Authentification renforcée », où le code est posté.
    pub page_url: Url,
    pub ticket: String,
    pub uid: String,
    pub user_hash: String,
    /// Racine de l'API esup-otp.
    pub api_url: Url,
    /// Message affiché par la page (code refusé…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Le mot de passe fourni a été posté avant cette page. Sinon, une session existante a mené
    /// au second facteur sans vérifier les identifiants.
    #[serde(default)]
    pub password_posted: bool,
}

/// Fin de la marche.
#[derive(Debug)]
pub enum Outcome {
    /// Page finale du service, session ouverte. `password_posted` dit si le mot de passe fourni a
    /// été envoyé au CAS pendant cette marche : sinon, une session existante a suffi et les
    /// identifiants fournis n'ont **pas** été vérifiés.
    SignedIn { page: Page, password_posted: bool },
    /// Le CAS attend un second facteur.
    SecondFactor(OtpChallenge),
}

#[derive(Debug, thiserror::Error)]
pub enum CasError {
    #[error("identifiant ou mot de passe refusé")]
    InvalidCredentials(Option<String>),
    /// Formulaire de connexion sans identifiants à poster : la session CAS a expiré.
    #[error("session CAS expirée")]
    NotAuthenticated,
    #[error("étape de connexion non prise en charge : {0}")]
    Unsupported(&'static str),
    #[error("le formulaire de connexion vise un hôte non sûr")]
    InsecureForm,
    #[error("réponse inattendue : {0}")]
    Unexpected(String),
    #[error(transparent)]
    Http(#[from] HttpError),
}

/// Identifiants à poster si le CAS les demande.
#[derive(Clone, Copy)]
pub struct Credentials<'a> {
    pub username: &'a str,
    pub password: &'a str,
}

/// Ouvre `url` (un service protégé ou le CAS lui-même) et traverse le CAS.
pub async fn open(
    browser: &mut Browser,
    cas: &CasServer,
    url: Url,
    credentials: Option<Credentials<'_>>,
) -> Result<Outcome, CasError> {
    let page = browser.get(url).await?;
    walk(browser, cas, page, credentials, true).await
}

/// Envoie le code du second facteur, puis termine la connexion.
///
/// Un code refusé renvoie [`Outcome::SecondFactor`] avec `error` renseigné.
pub async fn submit_code(
    browser: &mut Browser,
    cas: &CasServer,
    challenge: &OtpChallenge,
    code: &str,
    remember_device: bool,
) -> Result<Outcome, CasError> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let fields = vec![
        ("ticket".to_owned(), challenge.ticket.clone()),
        ("uid_hint".to_owned(), challenge.uid.clone()),
        ("token".to_owned(), code),
    ];
    let page = browser
        .post_form(challenge.page_url.clone(), &fields, &challenge.page_url)
        .await?;
    walk(browser, cas, page, None, remember_device).await
}

/// Ticket de service (`ST-…`) pour `service`, obtenu avec la session CAS du navigateur (TGC) sans
/// être consommé : la chaîne de redirections est suivie tant qu'elle reste sur le CAS et s'arrête
/// à la redirection vers `service?ticket=…`. Pour un service qui valide lui-même un ticket reçu
/// dans une requête d'API (ardemat). Sans session ou avec un second facteur : `NotAuthenticated`.
pub async fn service_ticket(
    browser: &mut Browser,
    cas: &CasServer,
    service: &Url,
) -> Result<String, CasError> {
    const MAX_HOPS: usize = 10;
    let mut url = cas.login_url(Some(service));
    for _ in 0..MAX_HOPS {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("text/html"),
        );
        let (status, response, body) = browser
            .send(reqwest::Method::GET, url.clone(), headers, None, None)
            .await?;
        let location = response
            .get(reqwest::header::LOCATION)
            .and_then(|l| l.to_str().ok())
            .filter(|_| status.is_redirection());
        let Some(location) = location else {
            // Page du CAS : formulaire de connexion ou second facteur, il faut se reconnecter.
            let page = Page {
                url: url.clone(),
                status,
                content_type: None,
                body,
                hops: Vec::new(),
                stopped_at: None,
            };
            return match classify(cas, &page) {
                Kind::Login(_) | Kind::Otp(_) | Kind::UnknownMfa | Kind::Remember(_) => {
                    Err(CasError::NotAuthenticated)
                }
                _ => Err(CasError::Unexpected(format!(
                    "pas de ticket pour le service (HTTP {status})"
                ))),
            };
        };
        let next = url
            .join(location)
            .map_err(|_| CasError::Unexpected("Location illisible".into()))?;
        if same_endpoint(&next, service) {
            return next
                .query_pairs()
                .find(|(k, _)| k == "ticket")
                .map(|(_, v)| v.into_owned())
                .filter(|t| !t.is_empty())
                .ok_or_else(|| CasError::Unexpected("retour au service sans ticket".into()));
        }
        if !cas.owns(&next) {
            return Err(CasError::Unexpected(
                "le CAS renvoie vers un autre hôte que le service".into(),
            ));
        }
        url = next;
    }
    Err(CasError::Unexpected(
        "trop de redirections sur le CAS".into(),
    ))
}

/// Même schéma, hôte, port et chemin (la requête est ignorée).
fn same_endpoint(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
        && a.path() == b.path()
}

enum Kind {
    Login(Form),
    Otp(OtpChallenge),
    Remember(Form),
    PasswordChange,
    UnknownMfa,
    Other,
}

async fn walk(
    browser: &mut Browser,
    cas: &CasServer,
    mut page: Page,
    credentials: Option<Credentials<'_>>,
    remember_device: bool,
) -> Result<Outcome, CasError> {
    let mut posted = false;
    let mut remembered = false;
    loop {
        match classify(cas, &page) {
            Kind::Login(form) => {
                let Some(credentials) = credentials else {
                    return Err(CasError::NotAuthenticated);
                };
                if posted {
                    return Err(CasError::InvalidCredentials(forms::error_message(
                        &page.body,
                    )));
                }
                let target = login_target(cas, &page.url, &form)?;
                let fields = login_fields(&form, credentials);
                posted = true;
                page = browser.post_form(target, &fields, &page.url).await?;
            }
            Kind::Otp(mut challenge) => {
                challenge.password_posted = posted;
                return Ok(Outcome::SecondFactor(challenge));
            }
            Kind::Remember(form) if !remembered => {
                remembered = true;
                let fields = remember_fields(&form, remember_device);
                let url = page.url.clone();
                page = browser.post_form(url.clone(), &fields, &url).await?;
            }
            Kind::Remember(_) => {
                return Err(CasError::Unexpected("page « appareil » en boucle".into()));
            }
            Kind::PasswordChange => return Err(CasError::Unsupported("password_change")),
            Kind::UnknownMfa => return Err(CasError::Unsupported("mfa")),
            Kind::Other => {
                return Ok(Outcome::SignedIn {
                    page,
                    password_posted: posted,
                });
            }
        }
    }
}

fn classify(cas: &CasServer, page: &Page) -> Kind {
    if page.stopped_at.is_some() {
        return Kind::Other;
    }
    let on_cas = cas.owns(&page.url);
    let body = &page.body;
    let forms = forms::forms(body);

    if on_cas
        && let Some(form) = forms
            .iter()
            .find(|f| f.has_password && (f.has("execution") || f.has("lt")))
    {
        return Kind::Login(form.clone());
    }
    if let Some(challenge) = otp_challenge(page, &forms) {
        return Kind::Otp(challenge);
    }
    if let Some(form) = forms
        .iter()
        .find(|f| f.has("rememberMe") && f.has("ticket"))
    {
        return Kind::Remember(form.clone());
    }
    if on_cas {
        let lower = body.to_lowercase();
        if lower.contains("casmustchangepass")
            || lower.contains("mot de passe a expiré")
            || lower.contains("mot de passe expiré")
            || lower.contains("password has expired")
        {
            return Kind::PasswordChange;
        }
        if forms.iter().any(|f| f.has("token") || f.has("otp")) || lower.contains("mfa-api") {
            return Kind::UnknownMfa;
        }
        if tracing::enabled!(tracing::Level::DEBUG) {
            tracing::debug!("page du CAS non reconnue : {}", page_shape(body, &forms));
        }
    }
    Kind::Other
}

/// Structure d'une page pour le diagnostic : scripts, appels `f({…})`, clés JSON et champs de
/// formulaire. Aucune valeur n'est reprise.
fn page_shape(body: &str, forms: &[Form]) -> String {
    let html = scraper::Html::parse_document(body);
    let mut sources = Vec::new();
    let mut calls = std::collections::BTreeSet::new();
    let mut keys = std::collections::BTreeSet::new();
    if let Ok(selector) = scraper::Selector::parse("script") {
        for script in html.select(&selector) {
            if let Some(src) = script.value().attr("src") {
                sources.push(src.split('?').next().unwrap_or_default().to_owned());
                continue;
            }
            let text: String = script.text().collect();
            collect_calls(&text, &mut calls);
            collect_keys(&text, &mut keys);
        }
    }
    let forms: Vec<String> = forms
        .iter()
        .map(|f| {
            let action = f
                .action
                .as_deref()
                .unwrap_or("")
                .split('?')
                .next()
                .unwrap_or("");
            format!("{action:?}:{:?}", f.names)
        })
        .collect();
    // Pour la page esup-otp : chaque `start(` et le type des paramètres attendus.
    let starts: Vec<String> = body
        .match_indices("start(")
        .map(|(index, call)| {
            let rest = body[index + call.len()..].trim_start();
            let value = serde_json::Deserializer::from_str(rest)
                .into_iter::<serde_json::Value>()
                .next()
                .and_then(Result::ok);
            match value {
                Some(serde_json::Value::Object(map)) => {
                    let types: Vec<String> = ["apiUrl", "uid", "userHash"]
                        .iter()
                        .map(|k| format!("{k}:{}", json_type(map.get(*k))))
                        .collect();
                    format!("objet({})", types.join(","))
                }
                Some(_) => "autre".to_owned(),
                None => rest
                    .chars()
                    .next()
                    .map_or("fin".to_owned(), |c| format!("{c:?}")),
            }
        })
        .collect();
    let ticket = forms_ticket(body);
    format!(
        "scripts={sources:?} appels={calls:?} clés={keys:?} formulaires={forms:?} start={starts:?} ticket={ticket}"
    )
}

fn json_type(value: Option<&serde_json::Value>) -> &'static str {
    match value {
        None => "absent",
        Some(serde_json::Value::String(s)) if s.is_empty() => "chaîne vide",
        Some(serde_json::Value::String(_)) => "chaîne",
        Some(serde_json::Value::Number(_)) => "nombre",
        Some(serde_json::Value::Null) => "null",
        Some(_) => "autre",
    }
}

/// État du champ `ticket` : absent, vide ou rempli (jamais sa valeur).
fn forms_ticket(body: &str) -> &'static str {
    match forms::forms(body)
        .iter()
        .find_map(|f| f.value("ticket").map(str::is_empty))
    {
        None => "absent",
        Some(true) => "vide",
        Some(false) => "rempli",
    }
}

/// Noms `f` des appels `f({` ou `f(\"` dans un script.
fn collect_calls(text: &str, out: &mut std::collections::BTreeSet<String>) {
    let bytes = text.as_bytes();
    for (i, _) in text.match_indices('(') {
        let next = text[i + 1..].trim_start().chars().next();
        if !matches!(next, Some('{' | '"' | '\'')) {
            continue;
        }
        let mut start = i;
        while start > 0
            && (bytes[start - 1].is_ascii_alphanumeric()
                || matches!(bytes[start - 1], b'_' | b'$' | b'.'))
        {
            start -= 1;
        }
        if start < i && out.len() < 30 {
            out.insert(text[start..i].to_owned());
        }
    }
}

/// Clés `"clé":` d'objets JSON ou JavaScript dans un script.
fn collect_keys(text: &str, out: &mut std::collections::BTreeSet<String>) {
    for (i, _) in text.match_indices('"') {
        let rest = &text[i + 1..];
        let Some(end) = rest.find('"') else { continue };
        let key = &rest[..end];
        let after = rest[end + 1..].trim_start();
        if after.starts_with(':')
            && !key.is_empty()
            && key.len() <= 40
            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && out.len() < 60
        {
            out.insert(key.to_owned());
        }
    }
}

/// Page esup-otp : `start({"apiUrl":…,"uid":…,"userHash":…})` et un formulaire avec `ticket`.
/// La page peut contenir d'autres `start(` (déclaration de la fonction, `restart(`…) : on retient
/// le premier appel dont l'argument est un objet avec `apiUrl`.
fn otp_challenge(page: &Page, forms: &[Form]) -> Option<OtpChallenge> {
    let body = &page.body;
    let params = body.match_indices("start(").find_map(|(index, call)| {
        let rest = body[index + call.len()..].trim_start();
        if !rest.starts_with('{') {
            return None;
        }
        serde_json::Deserializer::from_str(rest)
            .into_iter::<serde_json::Value>()
            .next()?
            .ok()
            .filter(|value| value.get("apiUrl").is_some())
    })?;
    let ticket = forms.iter().find_map(|f| f.value("ticket"))?.to_owned();
    Some(OtpChallenge {
        page_url: page.url.clone(),
        ticket,
        uid: params["uid"].as_str()?.to_owned(),
        user_hash: params["userHash"].as_str()?.to_owned(),
        api_url: Url::parse(params["apiUrl"].as_str()?).ok()?,
        error: forms::error_message(body),
        password_posted: false,
    })
}

/// Où poster le mot de passe. Apereo CAS 7 poste sur l'URL de la page, query comprise, alors que
/// l'action relative `login` la perdrait.
fn login_target(cas: &CasServer, page_url: &Url, form: &Form) -> Result<Url, CasError> {
    let action = match form.action.as_deref() {
        Some(action) if !action.is_empty() => page_url
            .join(action)
            .map_err(|_| CasError::Unexpected("action de formulaire illisible".into()))?,
        _ => page_url.clone(),
    };
    let target = match cas.flavor {
        CasFlavor::Apereo | CasFlavor::EsupOtpGateway
            if action.host_str() == page_url.host_str() && action.path() == page_url.path() =>
        {
            page_url.clone()
        }
        _ => action,
    };
    let secure = target.scheme() == "https" || cas.base.scheme() == target.scheme();
    if !secure || !cas.owns(&target) {
        return Err(CasError::InsecureForm);
    }
    Ok(target)
}

fn login_fields(form: &Form, credentials: Credentials<'_>) -> Vec<(String, String)> {
    let mut fields = vec![
        ("username".to_owned(), credentials.username.to_owned()),
        ("password".to_owned(), credentials.password.to_owned()),
    ];
    for (name, value) in &form.fields {
        match name.as_str() {
            "username" | "password" => {}
            "deviceFingerprint" if value.is_empty() => {
                fields.push((name.clone(), fingerprint(credentials.username)));
            }
            _ => fields.push((name.clone(), value.clone())),
        }
    }
    if form.has("execution") && !form.has("_eventId") {
        fields.push(("_eventId".to_owned(), "submit".to_owned()));
    }
    fields
}

fn remember_fields(form: &Form, remember: bool) -> Vec<(String, String)> {
    let mut fields: Vec<(String, String)> = form
        .fields
        .iter()
        .filter(|(name, _)| name == "ticket")
        .cloned()
        .collect();
    fields.push((
        "rememberMe".to_owned(),
        if remember { "submit" } else { "skip" }.to_owned(),
    ));
    if remember && form.has("rememberMeNoAsk") {
        fields.push(("rememberMeNoAsk".to_owned(), "1".to_owned()));
    }
    fields
}

/// Empreinte ClientJS stable pour un même compte (entier de 9 à 10 chiffres), comme un navigateur
/// qui revient toujours du même appareil.
fn fingerprint(username: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in username.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    (100_000_000 + hash % (u64::from(u32::MAX) - 100_000_000)).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(url: &str, body: &str) -> Page {
        Page {
            url: Url::parse(url).unwrap(),
            status: reqwest::StatusCode::OK,
            content_type: None,
            body: body.to_owned(),
            hops: Vec::new(),
            stopped_at: None,
        }
    }

    fn um() -> CasServer {
        CasServer::new(
            Url::parse("https://cas.umontpellier.fr/cas").unwrap(),
            CasFlavor::EsupOtpGateway,
        )
    }

    #[test]
    fn le_mot_de_passe_part_vers_l_url_de_la_page() {
        let url = "https://cas.umontpellier.fr/pre-otp/login?service=https%3A%2F%2Fx";
        let p = page(
            url,
            r#"<form id="fm1" action="login"><input name="username"><input type="password" name="password"><input type="hidden" name="execution" value="e"><input type="hidden" name="deviceFingerprint"></form>"#,
        );
        let Kind::Login(form) = classify(&um(), &p) else {
            panic!("formulaire non reconnu")
        };
        assert_eq!(login_target(&um(), &p.url, &form).unwrap().as_str(), url);
        let fields = login_fields(
            &form,
            Credentials {
                username: "u",
                password: "p",
            },
        );
        let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "username",
                "password",
                "execution",
                "deviceFingerprint",
                "_eventId"
            ]
        );
        assert_eq!(fields[3].1, fingerprint("u"));
        assert!((9..=10).contains(&fields[3].1.len()));
    }

    #[test]
    fn phpcas_poste_sur_l_action_et_refuse_un_autre_hote() {
        let cas = CasServer::new(
            Url::parse("https://cas.iutbeziers.fr:8443/").unwrap(),
            CasFlavor::Phpcas,
        );
        let p = page(
            "https://cas.iutbeziers.fr:8443/login?service=x",
            r#"<form action='/' id='cas_auth_form'><input type="hidden" name="lt" value="LT-1"><input type="hidden" name="service" value="https://wscodoc.iutbeziers.fr/"><input name="username"><input type="password" name="password"></form>"#,
        );
        let Kind::Login(form) = classify(&cas, &p) else {
            panic!("formulaire phpCAS non reconnu")
        };
        assert_eq!(
            login_target(&cas, &p.url, &form).unwrap().as_str(),
            "https://cas.iutbeziers.fr:8443/"
        );

        let mut evil = form.clone();
        evil.action = Some("https://ailleurs.fr/".into());
        assert!(matches!(
            login_target(&cas, &p.url, &evil),
            Err(CasError::InsecureForm)
        ));
    }

    #[test]
    fn page_otp_avec_la_fonction_start_declaree_avant_l_appel() {
        let otp = page(
            "https://cas.umontpellier.fr/cas/login-abc?service=x&auth_checked&ticket=ST-1",
            r#"<script>function start(params) { restart(params); } function restart(p) {}</script>
            <form method="post"><input name="ticket" value="TK" type="hidden"><input type="hidden" name="uid_hint" value=""></form>
            <script type="module">start({"apiUrl":"https://otp-api.umontpellier.fr/","uid":"e1","userHash":"h","validated_otp_too_old":false})</script>"#,
        );
        let Kind::Otp(challenge) = classify(&um(), &otp) else {
            panic!("page OTP non reconnue")
        };
        assert_eq!(challenge.uid, "e1");
        assert_eq!(challenge.ticket, "TK");
    }

    #[test]
    fn page_otp_et_page_appareil() {
        let otp = page(
            "https://cas.umontpellier.fr/cas/login-abc?service=x&auth_checked&ticket=ST-1",
            r#"<form method="post" id="fm1"><input id="ticket" name="ticket" value="TK" type="hidden"><input type="hidden" name="uid_hint" value=""><div id="errors"></div></form>
            <script type="module">start({"apiUrl":"https://otp-api.umontpellier.fr/","uid":"e1","userHash":"h","validated_otp_too_old":false})</script>"#,
        );
        let Kind::Otp(challenge) = classify(&um(), &otp) else {
            panic!("page OTP non reconnue")
        };
        assert_eq!(challenge.ticket, "TK");
        assert_eq!(challenge.uid, "e1");
        assert_eq!(
            challenge.api_url.as_str(),
            "https://otp-api.umontpellier.fr/"
        );
        assert_eq!(challenge.error, None);

        let remember = page(
            "https://cas.umontpellier.fr/cas/login?service=x",
            r#"<form id="registerform" method="post"><input id="ticket" name="ticket" value="T2" type="hidden"><button type="submit" name="rememberMe" value="submit">Oui</button><button type="submit" name="rememberMe" value="skip">Non</button><input type="checkbox" name="rememberMeNoAsk" value="1"></form>"#,
        );
        let Kind::Remember(form) = classify(&um(), &remember) else {
            panic!("page appareil non reconnue")
        };
        assert_eq!(
            remember_fields(&form, true),
            [
                ("ticket".to_owned(), "T2".to_owned()),
                ("rememberMe".to_owned(), "submit".to_owned()),
                ("rememberMeNoAsk".to_owned(), "1".to_owned())
            ]
        );
    }
}
