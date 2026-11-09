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
        // Le portail a son propre client CAS : on entre par lui, il renvoie au CAS avec son service.
        PortalConfig::TilesPortal { base_url } => {
            if let Ok(entry) = base_url.join("cas/login?next=%2F") {
                return entry;
            }
            None
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
    Browser::new(&options, CookieJar::from_stored(cookies)).map_err(|e| crate::client_problem(&e))
}

/// Ferme la session CAS de l'étudiant (le TGC n'est plus reconnu). Au mieux : un CAS qui ne répond
/// pas en 5 s ne fait pas échouer la déconnexion de l'API.
pub async fn logout(university: &ResolvedUniversity, session: &SessionData) {
    let Some(cas) = cas_server(university) else {
        return;
    };
    let Ok(url) = cas.base.join("logout") else {
        return;
    };
    let Ok(mut browser) = browser(university, session.cookies.clone(), NetSettings::default())
    else {
        return;
    };
    let _ = browser
        .send(
            reqwest::Method::GET,
            url,
            reqwest::header::HeaderMap::new(),
            None,
            Some(Duration::from_secs(5)),
        )
        .await;
}

/// Cookies d'une session précédente (du même compte, vérifié par le serveur) gardés pour la
/// reconnaissance de l'appareil : ceux que le CAS reçoit, c'est-à-dire posés pour son hôte ou un
/// domaine parent (`Domain=univ.fr`), sans le TGC : jamais réutilisé pour une nouvelle connexion
/// par mot de passe. Les cookies propres aux services (ScoDoc,
/// Moodle, mail…) ne passent pas d'une session à l'autre.
fn device_cookies(previous: Vec<StoredCookie>, cas: &CasServer) -> Vec<StoredCookie> {
    let host = cas.base.host_str().unwrap_or_default().to_ascii_lowercase();
    previous
        .into_iter()
        .filter(|c| !cas::TICKET_GRANTING.contains(&c.name.as_str()))
        .filter(|c| {
            let domain = c.domain.trim_start_matches('.').to_ascii_lowercase();
            host == domain || (!c.host_only && host.ends_with(&format!(".{domain}")))
        })
        .collect()
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
    // Le délai court depuis la première demande : un code refusé ne le prolonge pas.
    if !session.pending {
        session.expires_at = now.timestamp() + PENDING_TTL_SECS;
    }
    session.pending = true;
    Ok(LoginOutcome::SecondFactor(
        session,
        MfaPrompt {
            methods,
            message: message.or_else(|| challenge.error.clone()),
        },
    ))
}

/// Ouvre la page `[sso.mfa] step-up-url` juste après le mot de passe. Certains CAS exigent le
/// second facteur pour un service seulement (le mail à Montpellier) : sans ce passage, la
/// connexion réussit sans lui et ce service renvoie ensuite `session_expired` à chaque essai,
/// reconnexion comprise. Une page inaccessible ne fait pas échouer la connexion.
async fn step_up(
    university: &ResolvedUniversity,
    cas: &CasServer,
    browser: &mut Browser,
) -> Option<OtpChallenge> {
    let Sso::Cas(config) = &university.university.sso else {
        return None;
    };
    let url = config.mfa.as_ref()?.step_up_url.clone()?;
    match cas::open(browser, cas, url, None).await {
        Ok(Outcome::SecondFactor(mut challenge)) => {
            // Même navigateur, quelques instants après le POST du mot de passe vérifié.
            challenge.password_posted = true;
            Some(challenge)
        }
        Ok(Outcome::SignedIn { .. }) => None,
        Err(error) => {
            tracing::debug!("page du second facteur renforcé inaccessible : {error}");
            None
        }
    }
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
    let previous = device_cookies(input.previous_cookies, &cas);
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
                if let Some(challenge) = step_up(university, &cas, &mut browser).await {
                    return prompt(session, &mut browser, &challenge, None, input.now).await;
                }
                let session = signed_in(session, &browser, input.now, input.ttl_secs);
                return Ok(LoginOutcome::SignedIn(
                    crate::with_detected_establishment(university, session, input.now, input.net)
                        .await,
                ));
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
        Outcome::SignedIn { .. } => {
            let session = signed_in(pending, &browser, now, ttl_secs);
            Ok(LoginOutcome::SignedIn(
                crate::with_detected_establishment(
                    university,
                    session,
                    now,
                    NetSettings::default(),
                )
                .await,
            ))
        }
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
    let entry = entry_url(university, &cas);
    // Avec un portail, le renouvellement réussi revient sur lui : une page du CAS non reconnue
    // (maintenance, refus) ne prolonge pas la session.
    let expects_portal = !cas.owns(&entry) || entry.query_pairs().any(|(k, _)| k == "service");
    match cas::open(&mut browser, &cas, entry, None)
        .await
        .map_err(cas_problem)?
    {
        Outcome::SignedIn { page, .. } if expects_portal && cas.owns(&page.url) => {
            Err(Problem::new(ErrorCode::UpstreamChanged)
                .detail("le CAS n'a pas renvoyé vers le portail : session non prolongée"))
        }
        Outcome::SignedIn { .. } => Ok(signed_in(session, &browser, now, ttl_secs)),
        Outcome::SecondFactor(_) => Err(Problem::new(ErrorCode::SessionExpired)
            .detail("l'université redemande le second facteur : reconnectez-vous")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(domain: &str, name: &str) -> StoredCookie {
        StoredCookie {
            domain: domain.to_owned(),
            path: "/".to_owned(),
            name: name.to_owned(),
            value: "v".to_owned(),
            host_only: true,
            expires: None,
        }
    }

    #[test]
    fn seuls_les_cookies_du_cas_reconnaissent_l_appareil() {
        let cas = CasServer::new(
            Url::parse("https://cas.exemple.test/cas/").unwrap(),
            tonapi_config::CasFlavor::default(),
        );
        let kept = device_cookies(
            vec![
                cookie("cas.exemple.test", "connect.sid"),
                cookie("cas.exemple.test", "TGC"),
                cookie("scodoc.exemple.test", "PHPSESSID"),
                cookie("exemple.test", "MoodleSession"),
                StoredCookie {
                    host_only: false,
                    ..cookie("exemple.test", "appareil")
                },
            ],
            &cas,
        );
        let names: Vec<&str> = kept.iter().map(|c| c.name.as_str()).collect();
        // `Domain=exemple.test` est envoyé au CAS ; un cookie réservé à l'hôte `exemple.test`
        // ne l'est pas.
        assert_eq!(names, ["connect.sid", "appareil"]);
    }
}
