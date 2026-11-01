//! Kit des faux serveurs de `tests/*_mock.rs` : écoute sur `127.0.0.1:0`, lecture des requêtes
//! reçues, université, session et contexte de test. Aucune donnée réelle.
#![allow(dead_code)] // chaque fichier de test n'en utilise qu'une partie

use std::collections::BTreeMap;
use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Response, StatusCode, Uri, header};
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
        net: NetSettings { https_only: false },
    }
}
