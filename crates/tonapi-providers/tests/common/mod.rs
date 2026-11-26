//! Kit des faux serveurs de `tests/*_mock.rs` : écoute sur `127.0.0.1:0`, lecture des requêtes
//! reçues, université, session et contexte de test. Aucune donnée réelle.
#![allow(dead_code)] // chaque fichier de test n'en utilise qu'une partie

use std::collections::BTreeMap;
use std::path::PathBuf;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Response, StatusCode, Uri, header};
use chrono::{DateTime, Utc};
use tokio::net::TcpListener;
use tonapi_config::{ResolvedUniversity, UniversityFile};
use tonapi_providers::{Context, NetSettings};
use tonapi_session::{SessionData, StoredCookie};

/// Port local libre : l'écouteur et `http://<adresse>`, à connaître avant de construire l'état.
pub async fn listen() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    (listener, base)
}

/// Sert `app` en tâche de fond.
pub fn spawn(listener: TcpListener, app: Router) {
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
}

/// Sert `app` sur un port local libre et rend `http://<adresse>`.
pub async fn serve(app: Router) -> String {
    let (listener, base) = listen().await;
    spawn(listener, app);
    base
}

pub fn query(uri: &Uri, key: &str) -> Option<String> {
    url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

/// `302 Found` vers `location`, sans cookie.
pub fn redirect(location: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location)
        .body(Body::empty())
        .unwrap()
}

/// L'en-tête `Cookie` contient exactement `nom=valeur`.
pub fn has_cookie(headers: &HeaderMap, pair: &str) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .is_some_and(|c| c.split("; ").any(|p| p == pair))
}

/// Université lue depuis un `university.toml` écrit en JSON, sans composante.
pub fn university(file: serde_json::Value) -> ResolvedUniversity {
    ResolvedUniversity {
        dir: PathBuf::new(),
        university: serde_json::from_value::<UniversityFile>(file).unwrap(),
        establishments: BTreeMap::new(),
    }
}

/// Cookie du faux serveur (`127.0.0.1`, hôte seul, sans expiration).
pub fn stored_cookie(name: &str, value: &str, path: &str) -> StoredCookie {
    StoredCookie {
        domain: "127.0.0.1".to_owned(),
        path: path.to_owned(),
        name: name.to_owned(),
        value: value.to_owned(),
        host_only: true,
        expires: None,
    }
}

/// Session sans jeton ni composante, qui n'expire pas.
pub fn session(university: &str, username: &str, cookies: Vec<StoredCookie>) -> SessionData {
    SessionData {
        university: university.to_owned(),
        username: username.to_owned(),
        establishment: None,
        cookies,
        tokens: BTreeMap::new(),
        pending: false,
        issued_at: 0,
        expires_at: i64::MAX,
    }
}

/// Contexte d'un fournisseur ; `http` est permis vers le faux serveur.
pub fn context<'a>(
    university: &'a ResolvedUniversity,
    session: &'a mut SessionData,
    now: DateTime<Utc>,
) -> Context<'a> {
    Context {
        university,
        session,
        now,
        net: NetSettings {
            https_only: false,
            user_agent: None,
        },
    }
}

// Faux CAS Apereo

/// Identifiants que le faux CAS accepte.
pub const CAS_USER: &str = "camille.demo";
pub const CAS_PASSWORD: &str = "mot-de-passe-de-test";
/// Ticket de service que le faux CAS rend.
pub const CAS_TICKET: &str = "ST-1-test";

/// Ce que le faux CAS a reçu.
#[derive(Default)]
pub struct CasLog {
    /// Services demandés à `login`, dans l'ordre.
    pub services: Vec<String>,
    /// Formulaires de connexion affichés.
    pub forms: u32,
    /// Mots de passe postés.
    pub password_posts: u32,
}

/// Faux CAS Apereo sous `/cas/` : avec le cookie `TGC=ok`, `login` renvoie vers le service avec
/// [`CAS_TICKET`] ; sinon il affiche son formulaire, qui accepte [`CAS_USER`] et
/// [`CAS_PASSWORD`] et pose le TGC.
#[derive(Clone, Default)]
pub struct Cas {
    pub log: Arc<Mutex<CasLog>>,
}

const CAS_FORM: &str = r#"<html><body><form method="post" id="fm1" action="login">
    <input name="username"><input type="password" name="password">
    <input type="hidden" name="execution" value="e1s1"></form></body></html>"#;
const CAS_REFUSED: &str = r#"<html><body><div class="banner banner-danger"><span>Identifiants refusés.</span></div>
    <form method="post" id="fm1" action="login">
    <input name="username"><input type="password" name="password">
    <input type="hidden" name="execution" value="e1s2"></form></body></html>"#;

fn page(status: StatusCode, body: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

async fn cas_handler(
    State(cas): State<Cas>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response<Body> {
    if uri.path() != "/cas/login" {
        return page(StatusCode::OK, "<html><body>Déconnexion</body></html>");
    }
    let service = query(&uri, "service");
    let mut log = cas.log.lock().unwrap();
    if let Some(service) = &service {
        log.services.push(service.clone());
    }
    let back = |tgc: bool| match &service {
        Some(service) => {
            let separator = if service.contains('?') { '&' } else { '?' };
            let mut response = redirect(&format!("{service}{separator}ticket={CAS_TICKET}"));
            if tgc {
                response
                    .headers_mut()
                    .insert(header::SET_COOKIE, "TGC=ok; Path=/cas".parse().unwrap());
            }
            response
        }
        None => page(
            StatusCode::OK,
            "<html><body>Connexion réussie</body></html>",
        ),
    };
    if method == Method::POST {
        log.password_posts += 1;
        let field = |name: &str| {
            url::form_urlencoded::parse(body.as_bytes())
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default()
        };
        return if field("username") == CAS_USER && field("password") == CAS_PASSWORD {
            back(true)
        } else {
            page(StatusCode::UNAUTHORIZED, CAS_REFUSED)
        };
    }
    if has_cookie(&headers, "TGC=ok") {
        return back(false);
    }
    log.forms += 1;
    page(StatusCode::OK, CAS_FORM)
}

/// Sert le faux CAS sur l'écouteur donné. Son adresse, sous le nom `localhost`, est
/// [`cas_base`] : un autre hôte que les faux services, comme un vrai CAS.
pub fn spawn_cas(listener: TcpListener, cas: &Cas) {
    spawn(
        listener,
        Router::new().fallback(cas_handler).with_state(cas.clone()),
    );
}

/// `http://localhost:<port>/cas/` pour un écouteur de `http://127.0.0.1:<port>`.
pub fn cas_base(listener_base: &str) -> String {
    format!("{}/cas/", listener_base.replace("127.0.0.1", "localhost"))
}

/// TGC que le faux CAS reconnaît, tel qu'une connexion le laisse dans la session.
pub fn tgc() -> StoredCookie {
    StoredCookie {
        domain: "localhost".to_owned(),
        path: "/cas".to_owned(),
        name: "TGC".to_owned(),
        value: "ok".to_owned(),
        host_only: true,
        expires: None,
    }
}
