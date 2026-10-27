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

use crate::net::NetSettings;
use crate::{demo, pick_establishment};

/// Cookies qui portent à eux seuls une session CAS (ticket-granting) : jamais réutilisés pour une
/// nouvelle connexion par mot de passe.
const SSO_TICKET_COOKIES: [&str; 3] = ["TGC", "CASTGC", "CASTGT"];

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
    pub net: NetSettings,
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
///
/// On part du CAS avec le service `<portail>/Login`, comme le bouton « Connexion » : la racine d'un
/// uPortal peut afficher une page invitée au lieu de rediriger vers le CAS (Rennes, octobre 2026).
fn entry_url(university: &ResolvedUniversity, cas: &CasServer) -> Url {
    let service = match &university.university.portal {
        PortalConfig::Uportal4Prolongation { base_url } | PortalConfig::Uportal5 { base_url } => {
            base_url.join("Login").ok()
        }
        PortalConfig::None => None,
    };
    cas.login_url(service.as_ref())
}

/// Une connexion qui n'a laissé aucun cookie au CAS n'est jamais passée par lui.
fn reached_cas(browser: &Browser, cas: &CasServer) -> Result<(), Problem> {
    let host = cas.base.host_str().unwrap_or_default();
    if browser.jar.has_cookies_for(host) {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::UpstreamChanged)
            .detail("la connexion n'est pas passée par le CAS : vérifier l'adresse du portail"))
    }
}

fn browser(
    university: &ResolvedUniversity,
    cookies: Vec<StoredCookie>,
    net: NetSettings,
) -> Result<Browser, Problem> {
    let options = BrowserOptions {
        extra_ca: university.extra_ca(None),
        https_only: net.https_only,
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
    let credentials = Credentials {
        username: input.username,
        password: input.password,
    };
    let entry = entry_url(university, &cas);

    // Les cookies d'une session précédente servent à faire reconnaître l'appareil (pas de second
    // facteur), jamais à se passer du mot de passe : on retire le TGC, et si le CAS laisse quand
    // même passer sans le demander, on recommence avec un pot vide.
    let previous: Vec<StoredCookie> = input
        .previous_cookies
        .into_iter()
        .filter(|c| !SSO_TICKET_COOKIES.contains(&c.name.as_str()))
        .collect();
    for cookies in [previous, Vec::new()] {
        let mut browser = browser(university, cookies, input.net)?;
        let outcome = cas::open(&mut browser, &cas, entry.clone(), Some(credentials))
            .await
            .map_err(cas_problem)?;
        match outcome {
            Outcome::SignedIn {
                password_posted: true,
                ..
            } => {
                reached_cas(&browser, &cas)?;
                return Ok(LoginOutcome::SignedIn(signed_in(
                    session,
                    &browser,
                    input.now,
                    input.ttl_secs,
                )));
            }
            Outcome::SecondFactor(challenge) if challenge.password_posted => {
                return prompt(session, &mut browser, &challenge, None, input.now).await;
            }
            // Identifiants non vérifiés : essai suivant, avec un pot vide.
            Outcome::SignedIn { .. } | Outcome::SecondFactor(_) => {}
        }
    }
    Err(Problem::new(ErrorCode::UpstreamChanged)
        .detail("le CAS n'a pas demandé le mot de passe : connexion refusée"))
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
    if !challenge.password_posted {
        return Err(Problem::new(ErrorCode::InvalidToken)
            .detail("second facteur sans mot de passe vérifié : recommencer la connexion"));
    }
    let mut browser = browser(university, pending.cookies.clone(), NetSettings::default())?;

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
        Outcome::SignedIn { .. } => Ok(LoginOutcome::SignedIn(signed_in(
            pending, &browser, now, ttl_secs,
        ))),
        Outcome::SecondFactor(mut next) => {
            // Même connexion, mot de passe déjà vérifié : la page suivante en hérite.
            next.password_posted = challenge.password_posted;
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
    let mut browser = browser(university, session.cookies.clone(), NetSettings::default())?;
    match cas::open(&mut browser, &cas, entry_url(university, &cas), None)
        .await
        .map_err(cas_problem)?
    {
        Outcome::SignedIn { .. } => Ok(signed_in(session, &browser, now, ttl_secs)),
        Outcome::SecondFactor(_) => Err(Problem::new(ErrorCode::SessionExpired)
            .detail("l'université redemande le second facteur : reconnectez-vous")),
    }
}
