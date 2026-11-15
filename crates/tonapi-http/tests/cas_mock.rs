//! Connexion CAS contre un faux CAS local qui reproduit la chaîne de Montpellier :
//! passerelle esup-otp (`/cas`), CAS mot de passe (`/pre-otp`), API OTP et socket Engine.IO.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Response, StatusCode, Uri, header};
use tonapi_config::CasFlavor;
use tonapi_http::cas::{self, CasError, CasServer, Credentials, Outcome};
use tonapi_http::{Browser, BrowserOptions, CookieJar, otp};
use url::Url;

#[derive(Default)]
struct Mock {
    base: String,
    otp_required: bool,
    password_posts: u32,
    pushed: bool,
    remembered: Option<String>,
    first_poll_done: bool,
    /// Réponse du CAS au POST du mot de passe : page de refus que le code ne connaît pas.
    refuse_password: Option<StatusCode>,
    /// Réponse du CAS au POST du code : page de refus que le code ne connaît pas.
    refuse_code: Option<StatusCode>,
}

const LOCKED_PAGE: &str = "<html><body><h1>Compte temporairement verrouillé</h1><p>Réessayez plus tard.</p></body></html>";

fn status_html(status: StatusCode, body: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

type Shared = Arc<Mutex<Mock>>;

fn redirect(location: &str, cookie: Option<&str>) -> Response<Body> {
    let mut r = Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location);
    if let Some(cookie) = cookie {
        r = r.header(header::SET_COOKIE, cookie);
    }
    r.body(Body::empty()).unwrap()
}

fn html(body: String, cookie: Option<&str>) -> Response<Body> {
    let mut r = Response::builder().header(header::CONTENT_TYPE, "text/html");
    if let Some(cookie) = cookie {
        r = r.header(header::SET_COOKIE, cookie);
    }
    r.body(Body::from(body)).unwrap()
}

fn text(body: &str) -> Response<Body> {
    Response::new(Body::from(body.to_owned()))
}

fn has_cookie(headers: &HeaderMap, pair: &str) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .is_some_and(|c| c.split("; ").any(|p| p == pair))
}

fn query(uri: &Uri, key: &str) -> Option<String> {
    url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

fn form(body: &str, key: &str) -> Option<String> {
    url::form_urlencoded::parse(body.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

fn login_page(error: bool) -> String {
    let error = if error {
        r#"<div class="banner banner-danger">Mauvais identifiant / mot de passe.</div>"#
    } else {
        ""
    };
    format!(
        r#"<html><body><div class="banner banner-warning">Confidentiel</div>{error}
        <form method="post" id="fm1" action="login">
        <input id="username" name="username"><input type="password" name="password">
        <input type="hidden" name="execution" value="E1"><input type="hidden" name="_eventId" value="submit">
        <input type="hidden" name="geolocation"><input type="hidden" name="deviceFingerprint">
        <button name="submitBtn" type="submit">OK</button></form></body></html>"#
    )
}

fn otp_page(base: &str, error: bool) -> String {
    let error = if error { "Le code est invalide." } else { "" };
    format!(
        r#"<html><head><title>UM | Authentification Renforcée</title></head><body>
        <form method="post" id="fm1"><input id="ticket" name="ticket" value="TK" type="hidden">
        <input type="hidden" name="uid_hint" value=""><input name="token"><div id="errors">{error}</div></form>
        <script type="module">start({{"apiUrl":"{base}/otp/","uid":"e1","userHash":"H","validated_otp_too_old":false}})</script>
        </body></html>"#
    )
}

const REMEMBER_PAGE: &str = r#"<html><body><form id="registerform" method="post">
    <input id="ticket" name="ticket" value="T2" type="hidden">
    <button type="submit" name="rememberMe" value="submit">Oui</button>
    <button type="submit" name="rememberMe" value="skip">Non</button>
    <input type="checkbox" id="rememberMeNoAsk" name="rememberMeNoAsk" value="1"></form></body></html>"#;

#[allow(clippy::too_many_lines)] // un faux serveur entier, route par route
async fn handle(
    State(state): State<Shared>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response<Body> {
    let mut s = state.lock().unwrap();
    let base = s.base.clone();
    let app = format!("{base}/app/");
    let callback = format!("{base}/cas/login-h?service={}&auth_checked", enc(&app));
    let path = uri.path().to_owned();

    match (method.clone(), path.as_str()) {
        (Method::GET, "/app/") => {
            if has_cookie(&headers, "app=ok") {
                html("<html><body>bienvenue</body></html>".into(), None)
            } else if query(&uri, "ticket").as_deref() == Some("ST-final") {
                redirect(&app, Some("app=ok; Path=/app"))
            } else {
                redirect(&format!("{base}/cas/login?service={}", enc(&app)), None)
            }
        }
        (Method::GET, "/cas/login") if query(&uri, "ask").is_some() => {
            html(REMEMBER_PAGE.into(), None)
        }
        (Method::POST, "/cas/login") => {
            s.remembered = form(&body, "rememberMe");
            redirect(&format!("{app}?ticket=ST-final"), None)
        }
        (Method::GET, "/cas/login") => {
            if has_cookie(&headers, "connect.sid=otp-ok") {
                redirect(&format!("{app}?ticket=ST-final"), None)
            } else {
                redirect(
                    &format!("{base}/pre-otp/login?service={}", enc(&callback)),
                    None,
                )
            }
        }
        (Method::GET, "/pre-otp/login") => {
            if has_cookie(&headers, "TGC=tgt") {
                redirect(&format!("{callback}&ticket=ST-1"), None)
            } else {
                html(login_page(false), None)
            }
        }
        (Method::POST, "/pre-otp/login") if s.refuse_password.is_some() => {
            s.password_posts += 1;
            status_html(s.refuse_password.unwrap_or(StatusCode::OK), LOCKED_PAGE)
        }
        (Method::POST, "/pre-otp/login") => {
            s.password_posts += 1;
            let ok = query(&uri, "service").is_some()
                && form(&body, "username").as_deref() == Some("etu")
                && form(&body, "password").as_deref() == Some("s3cret+&=")
                && form(&body, "execution").as_deref() == Some("E1")
                && form(&body, "deviceFingerprint").is_some_and(|f| f.len() >= 9);
            if ok {
                redirect(
                    &format!("{callback}&ticket=ST-1"),
                    Some("TGC=tgt; Path=/pre-otp/; HttpOnly"),
                )
            } else {
                html(login_page(true), None)
            }
        }
        (Method::GET, "/cas/login-h") => {
            if s.otp_required {
                html(
                    otp_page(&base, false),
                    Some("connect.sid=pending; Path=/cas"),
                )
            } else {
                redirect(
                    &format!("{app}?ticket=ST-final"),
                    Some("connect.sid=otp-ok; Path=/cas"),
                )
            }
        }
        (Method::POST, "/cas/login-h") if s.refuse_code.is_some() => {
            status_html(s.refuse_code.unwrap_or(StatusCode::OK), LOCKED_PAGE)
        }
        (Method::POST, "/cas/login-h") => {
            if form(&body, "ticket").as_deref() == Some("TK")
                && form(&body, "token").as_deref() == Some("123456")
            {
                redirect(
                    &format!("{base}/cas/login?service={}&ask=rememberMe", enc(&app)),
                    Some("connect.sid=otp-ok; Path=/cas; Max-Age=2592000"),
                )
            } else {
                html(otp_page(&base, true), None)
            }
        }
        (Method::GET, "/otp/users/e1/H") => text(
            r#"{"code":"Ok","user":{"methods":{"codeRequired":true,"waitingFor":true,"totp":{"active":true},"push":{"active":true}},"transports":{"push":"Google Pixel 8"}}}"#,
        ),
        (Method::POST, "/otp/users/e1/methods/push/transports/push/H") => {
            s.pushed = true;
            text(r#"{"code":"Ok","message":"notification sent successfully"}"#)
        }
        (Method::GET, "/otp/sockets/") if query(&uri, "sid").is_none() => text(
            r#"0{"sid":"S1","upgrades":[],"pingInterval":300,"pingTimeout":300,"maxPayload":1000000}"#,
        ),
        (Method::POST, "/otp/sockets/") => text("ok"),
        (Method::GET, "/otp/sockets/") => {
            if !s.first_poll_done {
                s.first_poll_done = true;
                text(r#"40{"sid":"N1"}"#)
            } else if s.pushed {
                text("2\u{1e}42[\"userAuth\",{\"code\":\"Ok\",\"otp\":\"123456\"}]")
            } else {
                text("6")
            }
        }
        _ => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(format!("{method} {path}")))
            .unwrap(),
    }
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

async fn start(otp_required: bool) -> (Shared, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let state: Shared = Arc::new(Mutex::new(Mock {
        base: base.clone(),
        otp_required,
        ..Mock::default()
    }));
    let app = Router::new().fallback(handle).with_state(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (state, base)
}

fn browser(jar: CookieJar) -> Browser {
    Browser::new(
        &BrowserOptions {
            https_only: false,
            ..BrowserOptions::default()
        },
        jar,
    )
    .unwrap()
}

fn server(base: &str) -> CasServer {
    CasServer::new(
        Url::parse(&format!("{base}/cas/")).unwrap(),
        CasFlavor::EsupOtpGateway,
    )
}

const ETU: Credentials<'static> = Credentials {
    username: "etu",
    password: "s3cret+&=",
};

#[tokio::test]
async fn connexion_sans_second_facteur_puis_reprise_silencieuse() {
    let (state, base) = start(false).await;
    let app = Url::parse(&format!("{base}/app/")).unwrap();
    let mut b = browser(CookieJar::default());

    let Outcome::SignedIn { page, .. } = cas::open(&mut b, &server(&base), app.clone(), Some(ETU))
        .await
        .unwrap()
    else {
        panic!("second facteur inattendu")
    };
    assert!(page.body.contains("bienvenue"));
    assert_eq!(state.lock().unwrap().password_posts, 1);

    // Même pot de cookies, sans mot de passe : le TGC suffit.
    let host = Url::parse(&base).unwrap().host_str().unwrap().to_owned();
    let mut jar = b.jar.clone();
    jar.remove_host("nulle-part");
    assert!(jar.contains(&host, "TGC"));
    let mut b2 = browser(jar);
    let outcome = cas::open(&mut b2, &server(&base), app, None).await.unwrap();
    assert!(matches!(outcome, Outcome::SignedIn { .. }));
    assert_eq!(state.lock().unwrap().password_posts, 1);
}

#[tokio::test]
async fn mauvais_mot_de_passe_poste_une_seule_fois() {
    let (state, base) = start(false).await;
    let app = Url::parse(&format!("{base}/app/")).unwrap();
    let mut b = browser(CookieJar::default());
    let wrong = Credentials {
        username: "etu",
        password: "faux",
    };
    match cas::open(&mut b, &server(&base), app.clone(), Some(wrong)).await {
        Err(CasError::InvalidCredentials(message)) => {
            assert_eq!(
                message.as_deref(),
                Some("Mauvais identifiant / mot de passe.")
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(state.lock().unwrap().password_posts, 1);

    let mut b = browser(CookieJar::default());
    assert!(matches!(
        cas::open(&mut b, &server(&base), app, None).await,
        Err(CasError::NotAuthenticated)
    ));
}

#[tokio::test]
async fn second_facteur_par_code() {
    let (state, base) = start(true).await;
    let app = Url::parse(&format!("{base}/app/")).unwrap();
    let cas_server = server(&base);
    let mut b = browser(CookieJar::default());

    let Outcome::SecondFactor(challenge) = cas::open(&mut b, &cas_server, app, Some(ETU))
        .await
        .unwrap()
    else {
        panic!("second facteur attendu")
    };
    assert_eq!(challenge.uid, "e1");

    let methods = otp::methods(&mut b, &challenge).await.unwrap();
    assert_eq!(methods.push.as_deref(), Some("Google Pixel 8"));
    assert!(methods.code);

    let Outcome::SecondFactor(again) =
        cas::submit_code(&mut b, &cas_server, &challenge, "000 000", true)
            .await
            .unwrap()
    else {
        panic!("le mauvais code devait être refusé")
    };
    assert_eq!(again.error.as_deref(), Some("Le code est invalide."));

    let Outcome::SignedIn { page, .. } =
        cas::submit_code(&mut b, &cas_server, &again, "123 456", true)
            .await
            .unwrap()
    else {
        panic!("connexion attendue")
    };
    assert!(page.body.contains("bienvenue"));
    assert_eq!(state.lock().unwrap().remembered.as_deref(), Some("submit"));
    assert_eq!(state.lock().unwrap().password_posts, 1);
}

#[tokio::test]
async fn second_facteur_par_notification() {
    let (state, base) = start(true).await;
    let app = Url::parse(&format!("{base}/app/")).unwrap();
    let cas_server = server(&base);
    let mut b = browser(CookieJar::default());

    let Outcome::SecondFactor(challenge) = cas::open(&mut b, &cas_server, app, Some(ETU))
        .await
        .unwrap()
    else {
        panic!("second facteur attendu")
    };
    let code = otp::push(&mut b, &challenge, Duration::from_secs(5))
        .await
        .unwrap();
    assert!(state.lock().unwrap().pushed);
    assert_eq!(code.as_deref(), Some("123456"));

    let outcome = cas::submit_code(&mut b, &cas_server, &challenge, &code.unwrap(), false)
        .await
        .unwrap();
    assert!(matches!(outcome, Outcome::SignedIn { .. }));
    assert_eq!(state.lock().unwrap().remembered.as_deref(), Some("skip"));
}

/// Avec les cookies d'une session précédente encore vivante, le CAS ne montre
/// pas de formulaire, et la marche « réussit » sans que le mot de passe fourni ait été vérifié.
#[tokio::test]
async fn reprise_de_cookies_ne_verifie_pas_le_mot_de_passe() {
    let (state, base) = start(false).await;
    let app = Url::parse(&format!("{base}/app/")).unwrap();
    let mut b = browser(CookieJar::default());
    cas::open(&mut b, &server(&base), app.clone(), Some(ETU))
        .await
        .unwrap();

    let wrong = Credentials {
        username: "quelquun",
        password: "faux",
    };
    let mut b2 = browser(b.jar.clone());
    let outcome = cas::open(&mut b2, &server(&base), app, Some(wrong))
        .await
        .unwrap();
    // La marche le signale : à l'appelant de ne pas prendre cela pour une connexion vérifiée.
    assert!(matches!(
        outcome,
        Outcome::SignedIn {
            password_posted: false,
            ..
        }
    ));
    assert_eq!(
        state.lock().unwrap().password_posts,
        1,
        "aucun POST du mauvais mot de passe"
    );
}

/// Une page de refus que le code ne connaît pas, après le POST du mot de passe,
/// passait pour une connexion réussie. Elle doit donner une erreur, quel que soit le statut.
#[tokio::test]
async fn page_de_refus_inconnue_apres_le_mot_de_passe() {
    for status in [
        StatusCode::OK,
        StatusCode::LOCKED,
        StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        let (state, base) = start(false).await;
        state.lock().unwrap().refuse_password = Some(status);
        let app = Url::parse(&format!("{base}/app/")).unwrap();
        let mut b = browser(CookieJar::default());
        let outcome = cas::open(&mut b, &server(&base), app, Some(ETU)).await;
        assert!(
            matches!(outcome, Err(CasError::Unexpected(_))),
            "HTTP {status} : {outcome:?}"
        );
        assert_eq!(state.lock().unwrap().password_posts, 1);
    }
}

/// Même règle après le code du second facteur : pas de ticket, pas de session.
#[tokio::test]
async fn page_de_refus_inconnue_apres_le_code() {
    let (state, base) = start(true).await;
    let app = Url::parse(&format!("{base}/app/")).unwrap();
    let cas_server = server(&base);
    let mut b = browser(CookieJar::default());
    let Outcome::SecondFactor(challenge) = cas::open(&mut b, &cas_server, app, Some(ETU))
        .await
        .unwrap()
    else {
        panic!("second facteur attendu")
    };
    state.lock().unwrap().refuse_code = Some(StatusCode::OK);
    let outcome = cas::submit_code(&mut b, &cas_server, &challenge, "123456", true).await;
    assert!(
        matches!(outcome, Err(CasError::Unexpected(_))),
        "{outcome:?}"
    );
}
