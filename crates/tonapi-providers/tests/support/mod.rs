//! Faux serveur ScoDoc (passerelle) et faux CAS phpCAS, sur 127.0.0.1:0.
//!
//! Partagé par `tests/scodoc_mock.rs` et par les tests du module `scodoc`. Aucune donnée réelle.
//!
//! - `/scodoc/services/doAuth.php` : pose `PHPSESSID`, renvoie vers le CAS, valide le ticket ;
//! - `/scodoc/services/data.php?q=dataPremièreConnexion` : le contenu fourni, ou `{"redirect"}` ;
//! - `/cas/login` et `POST /cas/` : formulaire à jeton `lt`, TGC `tgc-ok`.
#![allow(dead_code)] // chaque consommateur n'utilise qu'une partie

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::Router;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, Method, Response, StatusCode, header};
use axum::routing::{any, get, post};

pub const USERNAME: &str = "camille.demo";
pub const PASSWORD: &str = "mot-de-passe-de-test";
/// Valeur du TGC que le faux CAS reconnaît.
pub const TGC: &str = "tgc-ok";

#[derive(Default)]
pub struct Mock {
    /// Corps de `dataPremièreConnexion`.
    pub payload: String,
    /// Nombre de prochaines réponses `{"redirect"}`, même avec une session valable.
    pub expire_next: u32,
    /// Toutes les réponses sont `{"redirect"}`.
    pub always_expired: bool,
    pub data_calls: u32,
    pub data_methods: Vec<String>,
    /// Tickets validés par `doAuth.php`.
    pub ticket_validations: u32,
    /// Passages par le CAS sans TGC (formulaire affiché).
    pub cas_forms: u32,
    pub password_posts: u32,
    /// En-têtes `Cookie` reçus par le CAS.
    pub cas_cookies: Vec<String>,
    sessions: HashMap<String, bool>,
    counter: u32,
}

type Shared = Arc<Mutex<Mock>>;

pub struct Server {
    /// `http://127.0.0.1:<port>`
    pub base: String,
    state: Shared,
}

impl Server {
    pub fn state(&self) -> MutexGuard<'_, Mock> {
        self.state.lock().unwrap()
    }

    /// Racine de la passerelle (avec barre finale).
    pub fn gateway(&self) -> String {
        format!("{}/scodoc/", self.base)
    }

    /// Racine du CAS (avec barre finale).
    pub fn cas(&self) -> String {
        format!("{}/cas/", self.base)
    }

    /// `host:port`, tel que la clé du jeton le reprend.
    pub fn authority(&self) -> &str {
        self.base.trim_start_matches("http://")
    }

    /// `university.toml` d'une université de test dont le CAS et la passerelle sont ce serveur ;
    /// `sign_in` est le `sign-in` des notes et des absences.
    pub fn university(&self, sign_in: &serde_json::Value) -> serde_json::Value {
        let gateway = serde_json::json!({
            "provider": "scodoc-gateway",
            "base-url": self.gateway(),
            "sign-in": sign_in,
        });
        serde_json::json!({
            "id": "test",
            "name": "Université de test",
            "sso": { "kind": "cas", "base-url": self.cas(), "flavor": "phpcas" },
            "services": { "grades": gateway, "absences": gateway },
        })
    }

    /// Les sessions PHP existantes cessent d'être authentifiées (gc_maxlifetime dépassé).
    pub fn expire_sessions(&self) {
        for authenticated in self.state().sessions.values_mut() {
            *authenticated = false;
        }
    }
}

pub async fn start(payload: &str) -> Server {
    let state = Arc::new(Mutex::new(Mock {
        payload: payload.to_owned(),
        ..Mock::default()
    }));
    let app = Router::new()
        .route("/scodoc/services/doAuth.php", get(do_auth))
        .route("/scodoc/services/data.php", any(data))
        .route("/scodoc/", get(|| async { "passerelle" }))
        .route("/cas/login", get(cas_login))
        .route("/cas/", post(cas_post))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Server { base, state }
}

fn origin(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    format!("http://{host}")
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split("; ")
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")).map(str::to_owned))
}

fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn redirect(status: StatusCode, location: &str, set_cookie: Option<&str>) -> Response<Body> {
    let mut response = Response::builder()
        .status(status)
        .header(header::LOCATION, location);
    if let Some(value) = set_cookie {
        response = response.header(header::SET_COOKIE, value);
    }
    response.body(Body::empty()).unwrap()
}

fn json(body: &str) -> Response<Body> {
    Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

fn service_url(base: &str, href: &str) -> String {
    format!("{base}/scodoc/services/doAuth.php?href={}", encode(href))
}

async fn do_auth(
    State(shared): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response<Body> {
    let base = origin(&headers);
    let href = query.get("href").cloned().unwrap_or_default();
    let mut state = shared.lock().unwrap();
    if let Some(ticket) = query.get("ticket") {
        if !ticket.starts_with("ST-") {
            return Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Body::empty())
                .unwrap();
        }
        state.ticket_validations += 1;
        state.counter += 1;
        let id = format!("sess{}", state.counter);
        state.sessions.insert(id.clone(), true);
        return redirect(
            StatusCode::FOUND,
            &service_url(&base, &href),
            Some(&format!("PHPSESSID={id}; Path=/")),
        );
    }
    let known = cookie(&headers, "PHPSESSID");
    if known
        .as_ref()
        .is_some_and(|id| state.sessions.get(id) == Some(&true))
    {
        return redirect(StatusCode::FOUND, &format!("{base}/scodoc/"), None);
    }
    let id = known.unwrap_or_else(|| {
        state.counter += 1;
        format!("sess{}", state.counter)
    });
    state.sessions.entry(id.clone()).or_insert(false);
    let login = format!(
        "{base}/cas/login?service={}",
        encode(&service_url(&base, &href))
    );
    redirect(
        StatusCode::FOUND,
        &login,
        Some(&format!("PHPSESSID={id}; Path=/")),
    )
}

async fn data(
    State(shared): State<Shared>,
    method: Method,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response<Body> {
    let mut state = shared.lock().unwrap();
    state.data_calls += 1;
    state.data_methods.push(method.to_string());
    if query.get("q").map(String::as_str) != Some("dataPremièreConnexion") {
        return json(r#"{"erreur":"Mauvaise requête."}"#);
    }
    let lost = r#"{"redirect":"/services/doAuth.php"}"#;
    if state.always_expired {
        return json(lost);
    }
    if state.expire_next > 0 {
        state.expire_next -= 1;
        return json(lost);
    }
    let authenticated =
        cookie(&headers, "PHPSESSID").is_some_and(|id| state.sessions.get(&id) == Some(&true));
    if authenticated {
        json(&state.payload)
    } else {
        json(lost)
    }
}

fn login_form(service: &str, error: bool) -> Response<Body> {
    let banner = if error {
        r#"<div class="errors">Mauvais identifiant ou mot de passe.</div>"#
    } else {
        ""
    };
    let page = format!(
        r#"<html><body>{banner}<form action='/cas/' method='post' name='cas_auth_form' id='cas_auth_form'>
<input type="hidden" name="lt" value="LT-test-1">
<input type="hidden" name="service" value="{}"/>
<input type='text' name="username"><input type='password' name="password"></form></body></html>"#,
        service.replace('&', "&amp;")
    );
    Response::builder()
        .header(header::CONTENT_TYPE, "text/html")
        .body(Body::from(page))
        .unwrap()
}

fn with_ticket(service: &str) -> String {
    let separator = if service.contains('?') { '&' } else { '?' };
    format!("{service}{separator}ticket=ST-1-test")
}

fn has_tgc(headers: &HeaderMap) -> bool {
    ["TGC", "CASTGC"]
        .iter()
        .any(|name| cookie(headers, name).as_deref() == Some(TGC))
}

async fn cas_login(
    State(shared): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response<Body> {
    let mut state = shared.lock().unwrap();
    state.cas_cookies.push(
        headers
            .get(header::COOKIE)
            .and_then(|c| c.to_str().ok())
            .unwrap_or_default()
            .to_owned(),
    );
    let service = query.get("service").cloned().unwrap_or_default();
    if has_tgc(&headers) {
        return redirect(StatusCode::FOUND, &with_ticket(&service), None);
    }
    state.cas_forms += 1;
    login_form(&service, false)
}

async fn cas_post(
    State(shared): State<Shared>,
    headers: HeaderMap,
    body: String,
) -> Response<Body> {
    let mut state = shared.lock().unwrap();
    state.cas_cookies.push(
        headers
            .get(header::COOKIE)
            .and_then(|c| c.to_str().ok())
            .unwrap_or_default()
            .to_owned(),
    );
    state.password_posts += 1;
    let form: HashMap<String, String> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    let service = form.get("service").cloned().unwrap_or_default();
    let valid = form.get("lt").is_some_and(|lt| lt == "LT-test-1")
        && form.get("username").map(String::as_str) == Some(USERNAME)
        && form.get("password").map(String::as_str) == Some(PASSWORD);
    if valid {
        redirect(
            StatusCode::SEE_OTHER,
            &with_ticket(&service),
            Some(&format!("CASTGC={TGC}; Path=/cas")),
        )
    } else {
        login_form(&service, true)
    }
}
