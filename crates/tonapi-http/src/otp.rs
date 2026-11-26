//! API esup-otp (second facteur) : méthodes du compte, code envoyé par SMS ou par mail et
//! validation par notification (push).
//!
//! Le push suit le navigateur : ouvrir la socket Engine.IO 4 en long polling **avant** d'envoyer la
//! notification, puis attendre le paquet `42["userAuth",{"code":"Ok","otp":"<code>"}]`. Ce code est
//! ensuite posté au CAS comme un code saisi. Un refus sur le téléphone n'émet rien : on s'arrête au
//! délai d'attente.

use std::time::{Duration, Instant};

use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::browser::{Browser, HttpError};
use crate::cas::{CasError, OtpChallenge};

/// Méthodes proposées à l'étudiant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtpMethods {
    /// Appareil qui reçoit les notifications (« marque modèle »).
    pub push: Option<String>,
    /// Un code à saisir est accepté (application TOTP, codes de secours, code envoyé).
    pub code: bool,
    /// Une application d'authentification (TOTP) est active.
    pub app: bool,
    /// Numéro, masqué par l'API, où un code peut être envoyé par SMS.
    pub sms: Option<String>,
    /// Adresse, masquée par l'API, où un code peut être envoyé par mail.
    pub mail: Option<String>,
}

/// Moyen par lequel l'API envoie un code à l'étudiant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OtpTransport {
    Sms,
    Mail,
}

impl OtpTransport {
    /// Nom du transport dans l'API.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            OtpTransport::Sms => "sms",
            OtpTransport::Mail => "mail",
        }
    }
}

impl OtpMethods {
    /// Seul moyen qu'a l'étudiant d'obtenir un code : ni application, ni notification, et un
    /// seul de SMS et mail. La page de l'université envoie alors le code sans qu'il le demande.
    #[must_use]
    pub fn only_transport(&self) -> Option<OtpTransport> {
        if self.app || self.push.is_some() {
            return None;
        }
        match (&self.sms, &self.mail) {
            (Some(_), None) => Some(OtpTransport::Sms),
            (None, Some(_)) => Some(OtpTransport::Mail),
            _ => None,
        }
    }
}

/// Méthodes qui envoient un code. `random_code` prend l'un ou l'autre transport ;
/// `random_code_mail` est la méthode propre au mail de certaines universités.
const CODE_METHODS: [&str; 2] = ["random_code", "random_code_mail"];

fn api_headers(challenge: &OtpChallenge) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("application/json, text/plain, */*"),
    );
    let origin = challenge.page_url.origin().ascii_serialization();
    if let Ok(value) = HeaderValue::from_str(&origin) {
        headers.insert(header::ORIGIN, value);
    }
    if let Ok(value) = HeaderValue::from_str(&format!("{origin}/")) {
        headers.insert(header::REFERER, value);
    }
    headers
}

fn api_url(challenge: &OtpChallenge, path: &str) -> Result<Url, CasError> {
    challenge
        .api_url
        .join(path)
        .map_err(|_| CasError::Unexpected("URL de l'API OTP".into()))
}

/// Compte de l'étudiant dans l'API (`GET /users/<uid>/<hash>`) : l'objet `user`. Aucun cookie
/// n'est nécessaire, le hash de la page tient lieu d'identité.
async fn user(
    browser: &mut Browser,
    challenge: &OtpChallenge,
) -> Result<serde_json::Value, CasError> {
    let url = api_url(
        challenge,
        &format!("users/{}/{}", challenge.uid, challenge.user_hash),
    )?;
    let (status, _, body) = browser
        .send(Method::GET, url, api_headers(challenge), None, None)
        .await?;
    if !status.is_success() {
        return Err(CasError::Unexpected(format!("API OTP : HTTP {status}")));
    }
    let mut json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|_| CasError::Unexpected("API OTP : réponse illisible".into()))?;
    Ok(json["user"].take())
}

/// Méthode active qui envoie un code par ce transport, et sa destination masquée.
fn channel(user: &serde_json::Value, transport: OtpTransport) -> Option<(&'static str, String)> {
    let to = user["transports"][transport.as_str()].as_str()?.trim();
    if to.is_empty() {
        return None;
    }
    let method = CODE_METHODS.into_iter().find(|name| {
        let method = &user["methods"][name];
        method["active"] == true
            && method["transports"]
                .as_array()
                .is_some_and(|list| list.iter().any(|t| t == transport.as_str()))
    })?;
    Some((method, to.to_owned()))
}

fn read_methods(user: &serde_json::Value) -> OtpMethods {
    let methods = &user["methods"];
    let push = (methods["push"]["active"] == true).then(|| {
        user["transports"]["push"]
            .as_str()
            .unwrap_or("téléphone")
            .to_owned()
    });
    let app = methods["totp"]["active"] == true;
    let code = methods["codeRequired"] == true || app || methods["bypass"]["active"] == true;
    OtpMethods {
        push,
        code,
        app,
        sms: channel(user, OtpTransport::Sms).map(|(_, to)| to),
        mail: channel(user, OtpTransport::Mail).map(|(_, to)| to),
    }
}

/// Méthodes actives du compte.
pub async fn methods(
    browser: &mut Browser,
    challenge: &OtpChallenge,
) -> Result<OtpMethods, CasError> {
    Ok(read_methods(&user(browser, challenge).await?))
}

/// Fait envoyer un code à l'étudiant par SMS ou par mail
/// (`POST /users/<uid>/methods/<méthode>/transports/<transport>/<hash>`). `unasked` : le code
/// part sans que l'étudiant l'ait demandé, ce que l'API apprend comme la page de l'université le
/// lui dit (`?auto`).
///
/// Erreur si le compte n'a pas ce transport ou si l'API refuse l'envoi.
pub async fn send_code(
    browser: &mut Browser,
    challenge: &OtpChallenge,
    transport: OtpTransport,
    unasked: bool,
) -> Result<(), CasError> {
    let user = user(browser, challenge).await?;
    let (method, _) = channel(&user, transport)
        .ok_or_else(|| CasError::Unexpected("API OTP : transport absent du compte".into()))?;
    let mut url = api_url(
        challenge,
        &format!(
            "users/{}/methods/{method}/transports/{}/{}",
            challenge.uid,
            transport.as_str(),
            challenge.user_hash
        ),
    )?;
    if unasked {
        url.set_query(Some("auto"));
    }
    let (status, _, body) = browser
        .send(Method::POST, url, api_headers(challenge), None, None)
        .await?;
    let sent = status.is_success()
        && serde_json::from_str::<serde_json::Value>(&body).is_ok_and(|json| json["code"] == "Ok");
    if sent {
        Ok(())
    } else {
        Err(CasError::Unexpected(format!(
            "envoi du code : refusé par l'API OTP (HTTP {status})"
        )))
    }
}

/// Paquets d'une réponse de polling Engine.IO 4 (séparés par `\x1e`).
fn packets(body: &str) -> impl Iterator<Item = &str> {
    body.split('\u{1e}').filter(|p| !p.is_empty())
}

/// Code transmis par `userAuth`, s'il est dans ce paquet.
fn user_auth_code(packet: &str) -> Option<String> {
    let payload = packet.strip_prefix("42")?;
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    let event = value.get(0)?.as_str()?;
    let data = value.get(1)?;
    (event == "userAuth" && data["code"] == "Ok")
        .then(|| data["otp"].as_str().map(str::to_owned))
        .flatten()
}

/// Envoie une notification et attend sa validation sur le téléphone, au plus `wait`.
/// Renvoie le code à poster au CAS, ou `None` si rien n'est arrivé à temps.
pub async fn push(
    browser: &mut Browser,
    challenge: &OtpChallenge,
    wait: Duration,
) -> Result<Option<String>, CasError> {
    let deadline = Instant::now() + wait;
    let headers = api_headers(challenge);
    let (socket, poll_timeout) = open_socket(browser, challenge, &headers).await?;

    let mut text_headers = headers.clone();
    text_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain;charset=UTF-8"),
    );
    browser
        .send(
            Method::POST,
            socket.clone(),
            text_headers.clone(),
            Some("40".into()),
            None,
        )
        .await?;

    let mut push_sent = false;
    let mut code = None;
    'poll: while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let result = browser
            .send_long_poll(
                Method::GET,
                socket.clone(),
                headers.clone(),
                poll_timeout.min(remaining + Duration::from_secs(1)),
            )
            .await;
        let body = match result {
            Ok((status, _, body)) if status.is_success() => body,
            Ok((status, _, _)) => {
                return Err(CasError::Unexpected(format!("socket OTP : HTTP {status}")));
            }
            Err(HttpError::Network(_)) => {
                // Délai de polling écoulé, ou erreur immédiate : courte pause pour ne pas
                // relancer en boucle serrée jusqu'à l'échéance.
                tokio::time::sleep(Duration::from_millis(500).min(remaining)).await;
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        for packet in packets(&body) {
            if packet.starts_with("44") {
                return Err(CasError::Unexpected("socket OTP refusée".into()));
            }
            if packet == "2" {
                browser
                    .send(
                        Method::POST,
                        socket.clone(),
                        text_headers.clone(),
                        Some("3".into()),
                        None,
                    )
                    .await?;
            } else if packet == "1" {
                break 'poll;
            } else if let Some(otp) = user_auth_code(packet) {
                code = Some(otp);
                break 'poll;
            }
        }
        if !push_sent {
            // La socket est connectée au namespace : la notification peut partir.
            send_push(browser, challenge, &headers).await?;
            push_sent = true;
        }
    }
    // Déconnexion propre, au mieux.
    let _ = browser
        .send(
            Method::POST,
            socket,
            text_headers,
            Some("41".into()),
            Some(Duration::from_secs(5)),
        )
        .await;
    Ok(code)
}

/// Poignée de main Engine.IO : `0{"sid":…}`. Renvoie l'URL de polling et son délai.
async fn open_socket(
    browser: &mut Browser,
    challenge: &OtpChallenge,
    headers: &HeaderMap,
) -> Result<(Url, Duration), CasError> {
    let stamp = chrono::Utc::now().timestamp_millis();
    let mut socket = api_url(challenge, "sockets/")?;
    socket
        .query_pairs_mut()
        .append_pair("uid", &challenge.uid)
        .append_pair("hash", &challenge.user_hash)
        .append_pair("app", "cas")
        .append_pair("EIO", "4")
        .append_pair("transport", "polling")
        .append_pair("t", &stamp.to_string());

    // Poignée de main : 0{"sid":…}
    let (_, _, body) = browser
        .send(Method::GET, socket.clone(), headers.clone(), None, None)
        .await?;
    let open: serde_json::Value = body
        .strip_prefix('0')
        .and_then(|j| serde_json::from_str(j).ok())
        .ok_or_else(|| CasError::Unexpected("socket OTP : poignée de main".into()))?;
    let sid = open["sid"]
        .as_str()
        .ok_or_else(|| CasError::Unexpected("socket OTP : sid absent".into()))?
        .to_owned();
    // Valeurs annoncées par le serveur OTP : bornées (une somme démesurée ou nulle ferait
    // boucler ou attendre sans fin).
    let poll_timeout = Duration::from_millis(
        open["pingInterval"]
            .as_u64()
            .unwrap_or(25_000)
            .saturating_add(open["pingTimeout"].as_u64().unwrap_or(20_000))
            .clamp(1_000, 60_000),
    );
    socket.query_pairs_mut().append_pair("sid", &sid);
    Ok((socket, poll_timeout))
}

/// `POST /users/<uid>/methods/push/transports/push/<hash>?auto`.
async fn send_push(
    browser: &mut Browser,
    challenge: &OtpChallenge,
    headers: &HeaderMap,
) -> Result<(), CasError> {
    let url = api_url(
        challenge,
        &format!(
            "users/{}/methods/push/transports/push/{}?auto",
            challenge.uid, challenge.user_hash
        ),
    )?;
    let (status, _, _) = browser
        .send(Method::POST, url, headers.clone(), None, None)
        .await?;
    if !status.is_success() {
        return Err(CasError::Unexpected(format!(
            "envoi du push : HTTP {status}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methodes_et_transports_du_compte() {
        // Forme vue à Rennes (novembre 2026) : le mail seul, par `random_code_mail`.
        let user: serde_json::Value = serde_json::from_str(
            r#"{"methods":{"codeRequired":true,"waitingFor":false,
                "totp":{"active":false,"transports":[]},
                "random_code":{"active":false,"transports":["sms"]},
                "random_code_mail":{"active":true,"transports":["mail"]},
                "bypass":{"active":false,"transports":[]},
                "push":{"active":false,"transports":["push"]}},
              "transports":{"mail":"e***@exemple.test","push":"téléphone"}}"#,
        )
        .unwrap();
        let methods = read_methods(&user);
        assert_eq!(
            methods,
            OtpMethods {
                push: None,
                code: true,
                app: false,
                sms: None,
                mail: Some("e***@exemple.test".into()),
            }
        );
        assert_eq!(methods.only_transport(), Some(OtpTransport::Mail));
        assert_eq!(
            channel(&user, OtpTransport::Mail),
            Some(("random_code_mail", "e***@exemple.test".to_owned()))
        );
        // Une méthode inactive, ou sans destination, n'envoie rien.
        assert_eq!(channel(&user, OtpTransport::Sms), None);

        // `random_code` porte les deux transports ; avec une application, rien ne part d'office.
        let both: serde_json::Value = serde_json::from_str(
            r#"{"methods":{"totp":{"active":true},"random_code":{"active":true,"transports":["sms","mail"]}},
              "transports":{"sms":"06******12","mail":"e***@exemple.test"}}"#,
        )
        .unwrap();
        let methods = read_methods(&both);
        assert!(methods.app && methods.code);
        assert_eq!(methods.sms.as_deref(), Some("06******12"));
        assert_eq!(
            channel(&both, OtpTransport::Sms).map(|(method, _)| method),
            Some("random_code")
        );
        assert_eq!(methods.only_transport(), None);
        let two = OtpMethods {
            app: false,
            ..methods
        };
        assert_eq!(two.only_transport(), None);
    }

    #[test]
    fn paquets_engine_io() {
        let body = "2\u{1e}42[\"userAuth\",{\"code\":\"Ok\",\"otp\":\"123456\"}]";
        let list: Vec<&str> = packets(body).collect();
        assert_eq!(list[0], "2");
        assert_eq!(user_auth_code(list[1]).as_deref(), Some("123456"));
        assert_eq!(user_auth_code("42[\"autre\",{}]"), None);
        assert_eq!(
            user_auth_code("42[\"userAuth\",{\"code\":\"Error\"}]"),
            None
        );
    }
}
