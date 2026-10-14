//! Connexion à l'université : démo ou CAS, second facteur, renouvellement silencieux.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tonapi_config::{PortalConfig, ResolvedUniversity, Sso};
use tonapi_core::{ErrorCode, Problem};
use tonapi_http::cas::{self, CasError, CasServer, Credentials, OtpChallenge, Outcome};
use tonapi_http::otp::{self, OtpMethods};
use tonapi_http::{Browser, BrowserOptions, CookieJar};
use tonapi_session::{SessionData, StoredCookie};
use url::Url;

use crate::{demo, pick_establishment};

/// Clé de [`SessionData::tokens`] qui garde le défi du second facteur.
const OTP_TOKEN: &str = "otp";
/// Durée de vie d'un jeton « en attente du second facteur ».
const PENDING_TTL_SECS: i64 = 300;

/// Second facteur demandé par l'université.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MfaPrompt {
    pub methods: OtpMethods,
    /// Message de l'université ou de tonAPI (code refusé, notification non validée).
    pub message: Option<String>,
}

/// Résultat d'une étape de connexion.
#[derive(Debug)]
pub enum LoginOutcome {
    SignedIn(SessionData),
    /// Session en attente (`pending`) à renvoyer avec la validation.
    SecondFactor(SessionData, MfaPrompt),
}

/// Ce que l'étudiant fournit pour se connecter.
pub struct LoginInput<'a> {
    pub username: &'a str,
    pub password: &'a str,
    pub establishment: Option<&'a str>,
    /// Cookies d'une session précédente : le CAS reconnaît l'appareil et ne redemande pas le
    /// second facteur.
    pub previous_cookies: Vec<StoredCookie>,
    pub now: DateTime<Utc>,
    pub ttl_secs: i64,
}

/// Validation du second facteur.
pub enum MfaAction {
    Code(String),
    /// Notification sur le téléphone, attendue au plus `wait`.
    Push {
        wait: Duration,
    },
}

fn cas_problem(error: CasError) -> Problem {
    match error {
        CasError::InvalidCredentials(message) => {
            let problem = Problem::new(ErrorCode::InvalidCredentials);
            match message {
                Some(m) => problem.detail(m),
                None => problem,
            }
        }
        CasError::NotAuthenticated => Problem::new(ErrorCode::SessionExpired),
        CasError::Unsupported("password_change") => Problem::new(ErrorCode::AccountActionRequired)
            .detail("mot de passe à changer sur le site de l'université"),
        CasError::Unsupported(step) => Problem::new(ErrorCode::NotImplemented).detail(format!(
            "étape de connexion `{step}` pas encore prise en charge pour ce CAS"
        )),
        CasError::InsecureForm | CasError::Unexpected(_) => {
            Problem::new(ErrorCode::UpstreamChanged).detail(error.to_string())
        }
        CasError::Http(e) => Problem::new(ErrorCode::ServiceUnavailable).detail(e.to_string()),
    }
}

fn cas_server(university: &ResolvedUniversity) -> Option<CasServer> {
    match &university.university.sso {
        Sso::Cas(cas) => Some(CasServer::new(cas.base_url.clone(), cas.flavor)),
        Sso::Demo => None,
    }
}

/// Page protégée qui lance la connexion : le portail s'il y en a un, sinon le CAS seul.
fn entry_url(university: &ResolvedUniversity, cas: &CasServer) -> Url {
    match &university.university.portal {
        PortalConfig::Uportal4Prolongation { base_url } | PortalConfig::Uportal5 { base_url } => {
            base_url.clone()
        }
        PortalConfig::None => cas.login_url(None),
    }
}

fn browser(
    university: &ResolvedUniversity,
    cookies: Vec<StoredCookie>,
) -> Result<Browser, Problem> {
    let options = BrowserOptions {
        extra_ca: university.extra_ca(None),
        ..BrowserOptions::default()
    };
    Browser::new(&options, CookieJar::from_stored(cookies))
        .map_err(|e| Problem::new(ErrorCode::Internal).detail(e.to_string()))
}

fn signed_in(
    mut session: SessionData,
    browser: &Browser,
    now: DateTime<Utc>,
    ttl_secs: i64,
) -> SessionData {
    session.cookies = browser.jar.to_stored(now.timestamp());
    session.tokens.remove(OTP_TOKEN);
    session.pending = false;
    session.issued_at = now.timestamp();
    session.expires_at = now.timestamp() + ttl_secs;
    session
}

async fn prompt(
    mut session: SessionData,
    browser: &mut Browser,
    challenge: &OtpChallenge,
    message: Option<String>,
    now: DateTime<Utc>,
) -> Result<LoginOutcome, Problem> {
    // Sans la liste des méthodes, le code saisi reste toujours possible.
    let methods = otp::methods(browser, challenge)
        .await
        .unwrap_or(OtpMethods {
            push: None,
            code: true,
        });
    session.cookies = browser.jar.to_stored(now.timestamp());
    session.tokens.insert(
        OTP_TOKEN.to_owned(),
        serde_json::to_string(challenge).map_err(|_| ErrorCode::Internal)?,
    );
    session.pending = true;
    session.expires_at = now.timestamp() + PENDING_TTL_SECS;
    Ok(LoginOutcome::SecondFactor(
        session,
        MfaPrompt {
            methods,
            message: message.or_else(|| challenge.error.clone()),
        },
    ))
}

/// Connexion avec identifiant et mot de passe. Le mot de passe n'est posté qu'une fois.
pub async fn login(
    university: &ResolvedUniversity,
    input: LoginInput<'_>,
) -> Result<LoginOutcome, Problem> {
    let session = SessionData {
        university: university.university.id.clone(),
        username: input.username.to_owned(),
        establishment: pick_establishment(university, input.establishment)?,
        cookies: Vec::new(),
        tokens: BTreeMap::new(),
        pending: false,
        issued_at: input.now.timestamp(),
        expires_at: input.now.timestamp() + input.ttl_secs,
    };
    let Some(cas) = cas_server(university) else {
        demo::check_credentials(input.username, input.password)?;
        return Ok(LoginOutcome::SignedIn(session));
    };
    let mut browser = browser(university, input.previous_cookies)?;
    let credentials = Credentials {
        username: input.username,
        password: input.password,
    };
    let entry = entry_url(university, &cas);
    match cas::open(&mut browser, &cas, entry, Some(credentials))
        .await
        .map_err(cas_problem)?
    {
        Outcome::SignedIn(_) => Ok(LoginOutcome::SignedIn(signed_in(
            session,
            &browser,
            input.now,
            input.ttl_secs,
        ))),
        Outcome::SecondFactor(challenge) => {
            prompt(session, &mut browser, &challenge, None, input.now).await
        }
    }
}

/// Valide le second facteur d'une session en attente.
pub async fn complete_mfa(
    university: &ResolvedUniversity,
    pending: SessionData,
    action: MfaAction,
    remember_device: bool,
    now: DateTime<Utc>,
    ttl_secs: i64,
) -> Result<LoginOutcome, Problem> {
    let cas = cas_server(university).ok_or(ErrorCode::InvalidRequest)?;
    let challenge: OtpChallenge = pending
        .tokens
        .get(OTP_TOKEN)
        .and_then(|json| serde_json::from_str(json).ok())
        .ok_or_else(|| {
            Problem::new(ErrorCode::InvalidToken).detail("aucun second facteur en attente")
        })?;
    let mut browser = browser(university, pending.cookies.clone())?;

    let code = match action {
        MfaAction::Code(code) => code,
        MfaAction::Push { wait } => {
            let received = otp::push(&mut browser, &challenge, wait)
                .await
                .map_err(cas_problem)?;
            let Some(code) = received else {
                let message = Some("Notification non validée à temps : réessayez.".to_owned());
                return prompt(pending, &mut browser, &challenge, message, now).await;
            };
            code
        }
    };
    match cas::submit_code(&mut browser, &cas, &challenge, &code, remember_device)
        .await
        .map_err(cas_problem)?
    {
        Outcome::SignedIn(_) => Ok(LoginOutcome::SignedIn(signed_in(
            pending, &browser, now, ttl_secs,
        ))),
        Outcome::SecondFactor(next) => {
            let message = next
                .error
                .clone()
                .or_else(|| Some("Code refusé.".to_owned()));
            prompt(pending, &mut browser, &next, message, now).await
        }
    }
}

/// Prolonge une session en repassant par le SSO sans mot de passe.
pub async fn refresh(
    university: &ResolvedUniversity,
    session: SessionData,
    now: DateTime<Utc>,
    ttl_secs: i64,
) -> Result<SessionData, Problem> {
    let Some(cas) = cas_server(university) else {
        let mut session = session;
        session.expires_at = now.timestamp() + ttl_secs;
        return Ok(session);
    };
    let mut browser = browser(university, session.cookies.clone())?;
    match cas::open(&mut browser, &cas, entry_url(university, &cas), None)
        .await
        .map_err(cas_problem)?
    {
        Outcome::SignedIn(_) => Ok(signed_in(session, &browser, now, ttl_secs)),
        Outcome::SecondFactor(_) => Err(Problem::new(ErrorCode::SessionExpired)
            .detail("l'université redemande le second facteur : reconnectez-vous")),
    }
}
