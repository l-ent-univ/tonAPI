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

mod shape;

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

/// Formulaire où le code du second facteur est posté, avec le champ `token`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OtpForm {
    /// Passerelle esup-otp-cas-server (Montpellier) : `ticket`, `uid_hint` et `token`.
    Gateway { ticket: String },
    /// Étape du webflow d'Apereo CAS (Rennes) : les champs cachés de la page (`execution`,
    /// `_eventId_submit`) et `token`. `execution` change à chaque réponse du CAS.
    Webflow { fields: Vec<(String, String)> },
}

// Le ticket et `execution` valent pour la connexion en cours : jamais dans une trace.
impl std::fmt::Debug for OtpForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            OtpForm::Gateway { .. } => "Gateway",
            OtpForm::Webflow { .. } => "Webflow",
        })
    }
}

/// Second facteur esup-otp demandé après le mot de passe.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct OtpChallenge {
    /// Page « Authentification renforcée », où le code est posté.
    pub page_url: Url,
    pub form: OtpForm,
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

// Le ticket, l'identifiant et le hash donnent accès à l'API OTP au nom de l'étudiant : jamais
// dans une trace.
impl std::fmt::Debug for OtpChallenge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OtpChallenge")
            .field("page", &self.page_url.host_str())
            .field("api", &self.api_url.host_str())
            .field("error", &self.error)
            .field("password_posted", &self.password_posted)
            .finish_non_exhaustive()
    }
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
    walk(browser, cas, page, credentials, true, false).await
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
    let mut fields = match &challenge.form {
        OtpForm::Gateway { ticket } => vec![
            ("ticket".to_owned(), ticket.clone()),
            ("uid_hint".to_owned(), challenge.uid.clone()),
        ],
        OtpForm::Webflow { fields } => fields.clone(),
    };
    fields.push(("token".to_owned(), code));
    let page = browser
        .post_form(challenge.page_url.clone(), &fields, &challenge.page_url)
        .await?;
    walk(browser, cas, page, None, remember_device, true).await
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

/// Cookies qui portent à eux seuls une session CAS (ticket-granting).
pub const TICKET_GRANTING: [&str; 3] = ["TGC", "CASTGC", "CASTGT"];

/// `code_posted` : la page de départ répond déjà à l'envoi d'un code du second facteur.
async fn walk(
    browser: &mut Browser,
    cas: &CasServer,
    mut page: Page,
    credentials: Option<Credentials<'_>>,
    remember_device: bool,
    code_posted: bool,
) -> Result<Outcome, CasError> {
    let mut posted = false;
    let mut remembered = false;
    // Dernier secret posté, et ticket de service vu depuis.
    let mut secret = if code_posted {
        Secret::Code
    } else {
        Secret::Nothing
    };
    let mut ticket = false;
    loop {
        ticket |= carries_ticket(&page);
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
                secret = Secret::Password;
                ticket = false;
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
                let host = cas.base.host_str().unwrap_or_default();
                let granted = TICKET_GRANTING
                    .iter()
                    .any(|name| browser.jar.contains(host, name));
                let evidence = Evidence {
                    secret,
                    ticket,
                    granted,
                };
                accept(cas, &page, evidence)?;
                return Ok(Outcome::SignedIn {
                    page,
                    password_posted: posted,
                });
            }
        }
    }
}

/// Secret posté pendant la marche.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Secret {
    Nothing,
    Password,
    /// Code du second facteur.
    Code,
}

/// Ce que la marche a vu avant d'arriver sur une page non reconnue.
#[derive(Clone, Copy)]
struct Evidence {
    secret: Secret,
    /// Ticket de service (`?ticket=…`) vu depuis le dernier secret posté.
    ticket: bool,
    /// Le pot contient un cookie ticket-granting du CAS.
    granted: bool,
}

/// Une page non reconnue n'ouvre la session que sur une preuve positive. Sans cette règle, une
/// page de refus que le code ne connaît pas (compte verrouillé, limitation, erreur 500) passait
/// pour une connexion réussie.
///
/// - Le CAS qui répond autre chose qu'un 2xx : erreur.
/// - Après un mot de passe ou un code : un ticket de service, ou un TGC émis par ce POST de mot
///   de passe (connexion sans service). Le pot de départ d'une connexion n'a jamais de TGC.
/// - Sans secret posté (page protégée, renouvellement), la page revient à l'appelant : c'est
///   lui qui sait où la marche devait arriver (voir `refresh` dans `tonapi-providers`).
fn accept(cas: &CasServer, page: &Page, evidence: Evidence) -> Result<(), CasError> {
    if page.stopped_at.is_some() {
        return Ok(());
    }
    let on_cas = cas.owns(&page.url);
    if on_cas && !page.status.is_success() {
        return Err(CasError::Unexpected(format!(
            "le CAS répond HTTP {} pendant la connexion",
            page.status.as_u16()
        )));
    }
    let proven = match evidence.secret {
        Secret::Nothing => true,
        Secret::Code => evidence.ticket,
        Secret::Password => evidence.ticket || evidence.granted,
    };
    if proven {
        Ok(())
    } else {
        Err(CasError::Unexpected(
            "page du CAS non reconnue, sans ticket de service".into(),
        ))
    }
}

/// Une redirection de la page mène à un ticket de service non vide. L'adresse demandée ne compte
/// pas : la page esup-otp, où le code est posté, porte déjà le ticket du mot de passe.
fn carries_ticket(page: &Page) -> bool {
    page.hops.iter().skip(1).any(|url| {
        url.query_pairs()
            .any(|(key, value)| key == "ticket" && !value.is_empty())
    })
}

fn classify(cas: &CasServer, page: &Page) -> Kind {
    if page.stopped_at.is_some() {
        return Kind::Other;
    }
    // Toutes les pages reconnues sont servies par le CAS. Une page d'un autre hôte de la chaîne
    // n'est même pas analysée : elle ne peut ni désigner l'API OTP appelée, ni recevoir le code,
    // ni coûter une analyse HTML.
    if !cas.owns(&page.url) {
        return Kind::Other;
    }
    let body = &page.body;
    let forms = forms::forms(body);

    if let Some(form) = forms
        .iter()
        .find(|f| f.has_password && (f.has("execution") || f.has("lt")))
    {
        return Kind::Login(form.clone());
    }
    if let Some(challenge) = otp_challenge(cas, page, &forms) {
        return Kind::Otp(challenge);
    }
    if let Some(form) = forms
        .iter()
        .find(|f| f.has("rememberMe") && f.has("ticket"))
    {
        return Kind::Remember(form.clone());
    }
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
        tracing::debug!(
            "page du CAS non reconnue : {}",
            shape::page_shape(body, &forms)
        );
    }
    Kind::Other
}

/// Page du second facteur esup-otp, sous l'une de ses deux formes. L'API désignée par la page
/// doit être en `https`, comme l'exige le script de la page (ou sur le schéma du CAS, pour les
/// faux serveurs des tests).
fn otp_challenge(cas: &CasServer, page: &Page, forms: &[Form]) -> Option<OtpChallenge> {
    let body = &page.body;
    let (form, api_url, uid, user_hash) =
        gateway_otp_page(body, forms).or_else(|| webflow_otp_page(body, forms))?;
    let mut api_url = Url::parse(&api_url).ok()?;
    if api_url.scheme() != "https" && api_url.scheme() != cas.base.scheme() {
        return None;
    }
    // Les chemins de l'API s'ajoutent à sa racine : elle finit par une barre.
    if !api_url.path().ends_with('/') {
        let path = format!("{}/", api_url.path());
        api_url.set_path(&path);
    }
    Some(OtpChallenge {
        page_url: page.url.clone(),
        form,
        uid,
        user_hash,
        api_url,
        error: forms::error_message(body),
        password_posted: false,
    })
}

/// Formulaire, adresse de l'API, identifiant et hash lus sur une page de second facteur.
type OtpPage = (OtpForm, String, String, String);

/// Passerelle esup-otp-cas-server : `start({"apiUrl":…,"uid":…,"userHash":…})` et un formulaire
/// avec `ticket`. La page peut contenir d'autres `start(` (déclaration de la fonction,
/// `restart(`…) : on retient le premier appel dont l'argument est un objet avec `apiUrl`.
fn gateway_otp_page(body: &str, forms: &[Form]) -> Option<OtpPage> {
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
    Some((
        OtpForm::Gateway { ticket },
        params["apiUrl"].as_str()?.to_owned(),
        params["uid"].as_str()?.to_owned(),
        params["userHash"].as_str()?.to_owned(),
    ))
}

/// Étape esup-otp du webflow d'Apereo CAS (Rennes, novembre 2026) : un formulaire sans mot de
/// passe qui porte `execution`, et un script qui déclare `apiUrl`, `uid` et `userHash` en
/// constantes. Le champ `token` n'est pas dans la page : le script de l'API l'ajoute.
fn webflow_otp_page(body: &str, forms: &[Form]) -> Option<OtpPage> {
    let form = forms
        .iter()
        .find(|f| !f.has_password && f.value("execution").is_some_and(|e| !e.is_empty()))?;
    let api_url = script_constant(body, "apiUrl")?;
    let uid = script_constant(body, "uid")?;
    let user_hash = script_constant(body, "userHash")?;
    if uid.is_empty() || user_hash.is_empty() {
        return None;
    }
    let fields = form
        .fields
        .iter()
        .filter(|(name, _)| name != "token")
        .cloned()
        .collect();
    Some((OtpForm::Webflow { fields }, api_url, uid, user_hash))
}

/// Valeur d'une constante de script déclarée avec une chaîne : `const nom = "valeur";` (ou `let`,
/// `var`, guillemets simples). La chaîne est lue comme Thymeleaf l'écrit : `\/`, `\xHH`,
/// `\uHHHH`.
fn script_constant(body: &str, name: &str) -> Option<String> {
    body.match_indices(name).find_map(|(index, _)| {
        let before = body[..index].trim_end();
        let declared = ["const", "let", "var"].iter().any(|keyword| {
            before.strip_suffix(keyword).is_some_and(|rest| {
                !rest
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$')
            })
        });
        // Un mot entier, suivi de `=` : `uid`, pas `uidHint`.
        let after = body[index + name.len()..].trim_start().strip_prefix('=')?;
        if !declared || body[..index].len() == before.len() {
            return None;
        }
        script_string(after.trim_start())
    })
}

/// Chaîne JavaScript entre guillemets au début de `source`, échappements décodés.
fn script_string(source: &str) -> Option<String> {
    let mut chars = source.chars();
    let quote = chars.next().filter(|c| *c == '"' || *c == '\'')?;
    let mut value = String::new();
    loop {
        match chars.next()? {
            c if c == quote => return Some(value),
            '\n' | '\r' => return None,
            '\\' => match chars.next()? {
                'x' => value.push(hex_char(&mut chars, 2)?),
                'u' => value.push(hex_char(&mut chars, 4)?),
                'n' => value.push('\n'),
                'r' => value.push('\r'),
                't' => value.push('\t'),
                other => value.push(other),
            },
            c => value.push(c),
        }
    }
}

fn hex_char(chars: &mut std::str::Chars<'_>, digits: usize) -> Option<char> {
    let hex: String = chars.take(digits).collect();
    if hex.len() != digits {
        return None;
    }
    char::from_u32(u32::from_str_radix(&hex, 16).ok()?)
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
        assert_eq!(
            challenge.form,
            OtpForm::Gateway {
                ticket: "TK".into()
            }
        );
    }

    fn rennes() -> CasServer {
        CasServer::new(
            Url::parse("https://sso-cas.exemple.test/").unwrap(),
            CasFlavor::Apereo,
        )
    }

    /// Forme de la page « Verification » du CAS de Rennes (novembre 2026), valeurs inventées.
    fn webflow_page(error: &str) -> String {
        format!(
            r#"<html><head><title>Verification</title></head><body><main>
            <form method="post" id="fm1">{error}
              <input type="hidden" name="_eventId_submit" value="Login" /><input type="hidden" name="execution" value="E2" /></form>
            <script type="module">
              // (ugly) syntax explained here
              const apiUrl = "https:\/\/mfa-api.exemple.test";
              const uid = "e\x31";
              const userHash = "h\u0041sh";
              const otpManagerUrl = "https:\/\/mfa.exemple.test";
              (async () => {{ const url = new URL(apiUrl); }})();
            </script></main></body></html>"#
        )
    }

    #[test]
    fn page_otp_du_webflow_apereo() {
        let url = "https://sso-cas.exemple.test/login?service=https%3A%2F%2Fx";
        let Kind::Otp(challenge) = classify(&rennes(), &page(url, &webflow_page(""))) else {
            panic!("page de code non reconnue")
        };
        assert_eq!(challenge.page_url.as_str(), url);
        assert_eq!(
            challenge.form,
            OtpForm::Webflow {
                fields: vec![
                    ("_eventId_submit".to_owned(), "Login".to_owned()),
                    ("execution".to_owned(), "E2".to_owned()),
                ]
            }
        );
        assert_eq!(challenge.uid, "e1");
        assert_eq!(challenge.user_hash, "hAsh");
        assert_eq!(challenge.api_url.as_str(), "https://mfa-api.exemple.test/");
        assert_eq!(challenge.error, None);

        // Code refusé : la même page, avec le message du CAS.
        let refused = webflow_page(
            r#"<div id="errors" class="notification text-center" data-status="error" role="alert"><p>Verification failed!</p><p>Please try again...</p></div>"#,
        );
        let Kind::Otp(challenge) = classify(&rennes(), &page(url, &refused)) else {
            panic!("page de code refusé non reconnue")
        };
        assert_eq!(
            challenge.error.as_deref(),
            Some("Verification failed! Please try again...")
        );
    }

    #[test]
    fn page_du_webflow_qui_n_est_pas_celle_du_code() {
        let url = "https://sso-cas.exemple.test/login";
        // Sans les constantes du script : une autre étape du CAS.
        let other = r#"<form method="post" id="fm1"><input type="hidden" name="execution" value="E2"></form>"#;
        assert!(matches!(
            classify(&rennes(), &page(url, other)),
            Kind::Other
        ));
        // Une API qui n'est pas en https n'est pas appelée.
        let insecure = webflow_page("").replace("https:\\/\\/mfa-api", "http:\\/\\/mfa-api");
        assert!(matches!(
            classify(&rennes(), &page(url, &insecure)),
            Kind::UnknownMfa
        ));
        // Le formulaire du mot de passe reste celui du mot de passe.
        let login =
            webflow_page(r#"<input name="username"><input type="password" name="password">"#);
        assert!(matches!(
            classify(&rennes(), &page(url, &login)),
            Kind::Login(_)
        ));
    }

    #[test]
    fn constantes_de_script() {
        let script = r#"const apiUrl = "a\/b"; let uidHint = "non"; var uid='o\'k';
            constuid = "non"; const  userHash  =  "\x3Dé";  const vide = "";"#;
        assert_eq!(script_constant(script, "apiUrl").as_deref(), Some("a/b"));
        assert_eq!(script_constant(script, "uid").as_deref(), Some("o'k"));
        assert_eq!(script_constant(script, "userHash").as_deref(), Some("=é"));
        assert_eq!(script_constant(script, "vide").as_deref(), Some(""));
        assert_eq!(script_constant(script, "absent"), None);
        // Pas une chaîne, chaîne non fermée, mot plus long.
        assert_eq!(script_constant("const uid = 12;", "uid"), None);
        assert_eq!(script_constant("const uid = \"a\nb\";", "uid"), None);
        assert_eq!(script_constant("const myuid = \"a\";", "uid"), None);
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
        assert_eq!(
            challenge.form,
            OtpForm::Gateway {
                ticket: "TK".into()
            }
        );
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
