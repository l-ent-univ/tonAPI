//! Connexion au webmail Zimbra (`zimbra::webmail`) contre de faux serveurs locaux : chaîne SAML
//! avec un formulaire soumis seul, liste des domaines à chaque saut et sur l'origine finale,
//! limite de sauts, page de connexion du CAS ; puis SOAP, pièce jointe et envoi de fichier avec la
//! session obtenue. Aucun test ne contacte un vrai service.

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, Response, StatusCode, Uri, header};
use serde_json::json;
use tonapi_providers::NetSettings;
use tonapi_providers::zimbra::webmail::{SignIn, SignInDomains, SignInError, sign_in};
use tonapi_session::StoredCookie;
use url::Url;

use common::{has_cookie, redirect, stored_cookie};

#[derive(Clone, Copy, PartialEq)]
enum Path {
    /// Fédération SAML, puis préauthentification avec redirection en http.
    Saml,
    /// La page du webmail se charge, avec un jeton CSRF.
    AppPage,
    /// Le CAS redemande le mot de passe.
    CasLogin,
    /// Une redirection vers un autre hôte (`localhost`).
    Elsewhere,
    /// Une redirection sans fin.
    Loop,
}

struct Mock {
    path: Path,
    base: String,
    hops: u32,
    soap_headers: Vec<HeaderMap>,
    soap_bodies: Vec<serde_json::Value>,
    uploaded: Vec<u8>,
    tgc_seen_on_post: bool,
}

type Shared = Arc<Mutex<Mock>>;

fn ok_html(body: String) -> Response<Body> {
    Response::builder()
        .header(header::CONTENT_TYPE, "text/html")
        .body(Body::from(body))
        .unwrap()
}

async fn handler(
    State(state): State<Shared>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response<Body> {
    let mut mock = state.lock().unwrap();
    mock.hops += 1;
    let base = mock.base.clone();
    match (method.as_str(), uri.path()) {
        ("GET", "/entry") => match mock.path {
            Path::Elsewhere => redirect(&base.replace("127.0.0.1", "localhost")),
            Path::Loop => redirect("/entry"),
            Path::CasLogin => ok_html(
                r#"<form method="post"><input name="username"><input type="hidden" name="execution" value="e1s1"></form>"#.to_owned(),
            ),
            Path::AppPage => Response::builder()
                .header(header::SET_COOKIE, "ZM_AUTH_TOKEN=jeton-page; Path=/")
                .header(header::CONTENT_TYPE, "text/html")
                .body(Body::from("<script>var csrfToken = \"0_csrf_de_test\";</script>"))
                .unwrap(),
            Path::Saml => redirect("/idp/sso"),
        },
        ("GET", "/idp/sso") => ok_html(format!(
            r#"<body onload="document.forms[0].submit()"><form method="post" action="{base}/sp/post"><input type="hidden" name="SAMLResponse" value="PHNhbWw+"/></form></body>"#
        )),
        ("POST", "/sp/post") => {
            mock.tgc_seen_on_post = has_cookie(&headers, "TGC=tgc-de-test");
            let form: Vec<(String, String)> = url::form_urlencoded::parse(&body).into_owned().collect();
            assert!(form.iter().any(|(n, v)| n == "SAMLResponse" && v == "PHNhbWw+"));
            assert_eq!(headers.get(header::REFERER).unwrap(), &format!("{base}/idp/sso"));
            Response::builder()
                .status(StatusCode::FOUND)
                .header(header::SET_COOKIE, "ZM_AUTH_TOKEN=jeton-saml; Path=/")
                .header(header::LOCATION, format!("{base}/service/preauth?x=1"))
                .body(Body::empty())
                .unwrap()
        }
        ("POST", "/service/soap/SearchRequest") => {
            mock.soap_headers.push(headers.clone());
            mock.soap_bodies.push(serde_json::from_slice(&body).unwrap());
            Response::builder()
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"Body": {"SearchResponse": {"c": [], "more": false}}}).to_string()))
                .unwrap()
        }
        ("GET", "/service/home/~/") => {
            if has_cookie(&headers, "ZM_AUTH_TOKEN=jeton-saml") {
                Response::builder()
                    .header(header::CONTENT_TYPE, "application/pdf")
                    .body(Body::from(vec![7_u8; 100_000]))
                    .unwrap()
            } else {
                Response::builder().status(401).body(Body::empty()).unwrap()
            }
        }
        ("POST", "/service/upload") => {
            mock.uploaded = body.to_vec();
            ok_html("200,'null',[{\"aid\":\"a1b2c3d4:e5\",\"ct\":\"application/pdf\",\"filename\":\"x.pdf\",\"s\":5}]".to_owned())
        }
        _ => Response::builder().status(404).body(Body::empty()).unwrap(),
    }
}

async fn start(path: Path) -> (Shared, String) {
    let (listener, base) = common::listen().await;
    let state = Arc::new(Mutex::new(Mock {
        path,
        base: base.clone(),
        hops: 0,
        soap_headers: Vec::new(),
        soap_bodies: Vec::new(),
        uploaded: Vec::new(),
        tgc_seen_on_post: false,
    }));
    let app = Router::new().fallback(handler).with_state(state.clone());
    common::spawn(listener, app);
    (state, base)
}

fn tgc() -> StoredCookie {
    stored_cookie("TGC", "tgc-de-test", "/")
}

async fn signed_in(
    base: &str,
    cookies: Vec<StoredCookie>,
) -> Result<tonapi_providers::zimbra::webmail::Webmail, SignInError> {
    let entry = Url::parse(&format!("{base}/entry")).unwrap();
    let domains = SignInDomains::new(["127.0.0.1"], false);
    sign_in(SignIn {
        entry: &entry,
        domains: &domains,
        cas_host: "127.0.0.1",
        cookies,
        net: NetSettings {
            https_only: false,
            user_agent: Some("node"),
        },
        extra_ca: Vec::new(),
    })
    .await
}

#[tokio::test]
async fn chaine_saml_puis_soap() {
    let (state, base) = start(Path::Saml).await;
    let webmail = signed_in(&base, vec![tgc()]).await.unwrap();
    assert_eq!(webmail.origin().as_str(), format!("{base}/"));
    assert!(!webmail.has_csrf());
    assert!(state.lock().unwrap().tgc_seen_on_post);

    let answer = webmail
        .soap(
            "SearchRequest",
            "urn:zimbraMail",
            json!({"query": "inid:2"}),
            std::time::Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert_eq!(answer["more"], false);
    let mock = state.lock().unwrap();
    let headers = &mock.soap_headers[0];
    assert!(has_cookie(headers, "ZM_AUTH_TOKEN=jeton-saml"));
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json; charset=utf-8"
    );
    assert!(headers.get("x-zimbra-csrf-token").is_none());
    assert_eq!(
        mock.soap_bodies[0]["Body"]["SearchRequest"]["_jsns"],
        "urn:zimbraMail"
    );
    drop(mock);

    // L'adresse de préauthentification porte le jeton.
    let preauth = webmail.preauth_url().unwrap();
    assert!(preauth.as_str().contains("authtoken=jeton-saml"));
    assert!(preauth.as_str().contains("isredirect=1"));
}

#[tokio::test]
async fn page_du_webmail_avec_jeton_csrf() {
    let (state, base) = start(Path::AppPage).await;
    let webmail = signed_in(&base, vec![tgc()]).await.unwrap();
    assert!(webmail.has_csrf());
    webmail
        .soap(
            "SearchRequest",
            "urn:zimbraMail",
            json!({}),
            std::time::Duration::from_secs(5),
        )
        .await
        .unwrap();
    let mock = state.lock().unwrap();
    assert_eq!(
        mock.soap_headers[0].get("x-zimbra-csrf-token").unwrap(),
        "0_csrf_de_test"
    );
    assert_eq!(
        mock.soap_bodies[0]["Header"]["context"]["csrfToken"],
        "0_csrf_de_test"
    );
}

#[tokio::test]
async fn page_de_connexion_du_cas() {
    let (_, base) = start(Path::CasLogin).await;
    assert_eq!(
        signed_in(&base, vec![tgc()]).await.unwrap_err(),
        SignInError::CasOver
    );
}

#[tokio::test]
async fn un_saut_hors_de_la_liste_n_est_pas_demande() {
    let (state, base) = start(Path::Elsewhere).await;
    let error = signed_in(&base, vec![tgc()]).await.unwrap_err();
    assert_eq!(error, SignInError::LeftDomains("localhost".to_owned()));
    // Seule l'entrée a été demandée.
    assert_eq!(state.lock().unwrap().hops, 1);
}

#[tokio::test]
async fn l_origine_finale_doit_etre_de_la_liste() {
    // Un cookie du webmail déjà là pour `localhost` : la redirection y mène, le raccourci
    // s'y arrêterait ; l'origine finale est refusée (W4).
    let (_, base) = start(Path::Elsewhere).await;
    let mut elsewhere = stored_cookie("ZM_AUTH_TOKEN", "x", "/");
    elsewhere.domain = "localhost".to_owned();
    let error = signed_in(&base, vec![tgc(), elsewhere]).await.unwrap_err();
    assert_eq!(error, SignInError::LeftDomains("localhost".to_owned()));
}

#[tokio::test]
async fn vingt_sauts_au_plus() {
    let (state, base) = start(Path::Loop).await;
    assert_eq!(
        signed_in(&base, vec![tgc()]).await.unwrap_err(),
        SignInError::TooManyHops
    );
    assert_eq!(state.lock().unwrap().hops, 20);
}

#[tokio::test]
async fn piece_jointe_et_envoi_au_fil_de_l_eau() {
    let (state, base) = start(Path::Saml).await;
    let webmail = signed_in(&base, vec![tgc()]).await.unwrap();
    let mut part = webmail.part("300", "2").await.unwrap();
    assert_eq!(part.status, 200);
    let mut read = 0;
    while let Some(chunk) = part.chunk().await.unwrap() {
        read += chunk.len();
    }
    assert_eq!(read, 100_000);

    let (status, text) = webmail
        .upload(
            tonapi_http::RequestBody::from(b"%PDF-".to_vec()),
            None,
            5,
            "application/pdf",
            "attachment; filename=\"x.pdf\"",
        )
        .await
        .unwrap();
    assert_eq!(status, 200);
    assert!(text.starts_with("200,"));
    assert_eq!(state.lock().unwrap().uploaded, b"%PDF-");
}
