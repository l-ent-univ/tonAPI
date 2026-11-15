//! Parcours complet sur l'université de démonstration, sans réseau.

use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tonapi_config::Registry;
use tonapi_server::{AppState, Settings, app, metrics_app};
use tonapi_session::SessionKeys;
use tower::ServiceExt;

fn server() -> Router {
    app(state())
}

fn state() -> AppState {
    state_with(true)
}

fn state_with(cache_catalog: bool) -> AppState {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../universities");
    let (registry, report) = Registry::load(&dir);
    assert!(report.is_ok());
    AppState::new(
        registry,
        SessionKeys::ephemeral().unwrap(),
        Settings {
            universities_dir: dir,
            session_ttl_secs: 3600,
            trusted_proxies: Vec::new(),
            docs: true,
            auth_per_ip: 120,
            cache_catalog,
        },
    )
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Value,
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Reply {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let request = match body {
        Some(body) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Reply {
        status,
        headers,
        body,
    }
}

async fn login(app: &Router, university: &str, extra: Value) -> Reply {
    let mut body = json!({ "university": university, "username": "demo", "password": "demo" });
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        body.extend(extra.clone());
    }
    call(app, "POST", "/v1/auth/login", None, Some(body)).await
}

#[tokio::test]
async fn parcours_demo_complet() {
    let app = server();

    let r = login(&app, "demo", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.headers[header::CACHE_CONTROL], "no-store");
    let token = r.body["token"].as_str().unwrap().to_owned();
    assert!(!token.contains("demo"));

    let r = call(&app, "GET", "/v1/me", Some(&token), None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body["data"]["profile"]["username"], "demo");
    assert_eq!(r.body["meta"]["provider"], "demo");
    assert_eq!(r.body["data"]["services"].as_array().unwrap().len(), 9);

    let r = call(
        &app,
        "GET",
        "/v1/timetable?from=2026-10-12&to=2026-10-18",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body["data"].as_array().unwrap().len(), 9);
    assert_eq!(r.body["data"][0]["start"], "2026-10-12T08:30:00+02:00");
    assert_eq!(r.body["meta"]["university"], "demo");

    for path in [
        "/v1/timetable",
        "/v1/timetable/groups",
        "/v1/grades",
        "/v1/absences",
        "/v1/exams",
        "/v1/homework",
        "/v1/mail/inbox",
        "/v1/apps",
        "/v1/documents",
    ] {
        let r = call(&app, "GET", path, Some(&token), None).await;
        assert_eq!(r.status, StatusCode::OK, "{path}: {}", r.body);
        assert!(r.body["meta"]["fetchedAt"].is_string(), "{path}");
    }

    // Fichier d'un document : servi en pièce jointe, jamais mis en cache.
    let r = call(
        &app,
        "GET",
        "/v1/documents/sem.2025_S1_1",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.headers[header::CONTENT_TYPE], "application/pdf");
    assert_eq!(r.headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        r.headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"sem-2025_S1_1.pdf\""
    );
    let r = call(&app, "GET", "/v1/documents/inconnu", Some(&token), None).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.body["code"], "not_found");

    let r = call(&app, "POST", "/v1/auth/refresh", Some(&token), None).await;
    assert_eq!(r.status, StatusCode::OK);
    let refreshed = r.body["token"].as_str().unwrap();
    assert_ne!(refreshed, token);

    let r = call(&app, "POST", "/v1/auth/logout", Some(refreshed), None).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn jeton_absent_ou_modifie_donne_401() {
    let app = server();
    let token = login(&app, "demo", json!({})).await.body["token"]
        .as_str()
        .unwrap()
        .to_owned();

    let r = call(&app, "GET", "/v1/grades", None, None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.body["code"], "invalid_token");
    assert_eq!(r.headers[header::CONTENT_TYPE], "application/problem+json");
    assert_eq!(r.headers[header::WWW_AUTHENTICATE], "Bearer");

    let mut tampered = token.into_bytes();
    let i = tampered.len() - 3;
    tampered[i] = if tampered[i] == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(tampered).unwrap();
    let r = call(&app, "GET", "/v1/grades", Some(&tampered), None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.body["code"], "invalid_token");
}

#[tokio::test]
async fn erreurs_de_connexion() {
    let app = server();

    let r = call(
        &app,
        "POST",
        "/v1/auth/login",
        None,
        Some(json!({ "university": "demo", "username": "demo", "password": "faux" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.body["code"], "invalid_credentials");
    assert_eq!(r.body["type"], "urn:tonapi:problem:invalid_credentials");

    let r = login(&app, "inconnue", json!({})).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.body["code"], "unknown_university");

    let r = call(
        &app,
        "POST",
        "/v1/auth/login",
        None,
        Some(json!({ "university": "demo" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.body["code"], "invalid_request");
}

#[tokio::test]
async fn universites_et_composantes() {
    let app = server();

    let r = call(&app, "GET", "/v1/universities", None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    let ids: Vec<&str> = r
        .body
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u["id"].as_str())
        .collect();
    assert_eq!(ids, ["demo", "umontpellier", "univ-montp3", "univ-rennes"]);

    let r = call(&app, "GET", "/v1/universities/umontpellier", None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    let beziers = r.body["establishments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "iut-beziers")
        .unwrap();
    let grades = beziers["services"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["service"] == "grades")
        .unwrap();
    assert_eq!(grades["availability"], "api");
    assert_eq!(grades["provider"], "scodoc-gateway");

    let r = call(&app, "GET", "/v1/universities/inconnue", None, None).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);

    let r = call(&app, "GET", "/nulle-part", None, None).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.body["code"], "not_found");
}

#[tokio::test]
async fn changer_de_composante_renvoie_un_nouveau_jeton() {
    let app = server();
    let token = login(&app, "demo", json!({})).await.body["token"]
        .as_str()
        .unwrap()
        .to_owned();

    // La démo n'a pas de composante.
    let r = call(
        &app,
        "PUT",
        "/v1/me/establishment",
        Some(&token),
        Some(json!({ "establishment": "iut-beziers" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.body["code"], "unknown_establishment");

    let r = call(
        &app,
        "PUT",
        "/v1/me/establishment",
        Some(&token),
        Some(json!({ "establishment": null })),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let new_token = r.headers["tonapi-session"].to_str().unwrap();
    let r = call(&app, "GET", "/v1/me", Some(new_token), None).await;
    assert_eq!(r.status, StatusCode::OK);
}

#[tokio::test]
async fn plage_de_dates_controlee() {
    let app = server();
    let token = login(&app, "demo", json!({})).await.body["token"]
        .as_str()
        .unwrap()
        .to_owned();
    for query in [
        "from=2026-10-12&to=2026-10-01",
        "from=2026-01-01&to=2026-12-31",
        "from=hier",
    ] {
        let r = call(
            &app,
            "GET",
            &format!("/v1/timetable?{query}"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(r.body["code"], "invalid_request");
    }
}

#[tokio::test]
async fn specification_et_documentation() {
    let app = server();
    let r = call(&app, "GET", "/openapi.json", None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body["openapi"].as_str().unwrap().starts_with("3."));
    for path in [
        "/v1/auth/login",
        "/v1/auth/service-login",
        "/v1/me",
        "/v1/timetable",
        "/v1/grades",
        "/v1/me/establishment",
    ] {
        assert!(
            r.body["paths"][path].is_object(),
            "{path} absent de la spec"
        );
    }
    assert!(r.body["components"]["securitySchemes"]["session"].is_object());

    let response = app
        .clone()
        .oneshot(Request::get("/docs").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn trop_d_echecs_bloquent_le_compte() {
    let app = server();
    let wrong = json!({ "university": "demo", "username": "demo", "password": "faux" });
    for _ in 0..5 {
        let r = call(&app, "POST", "/v1/auth/login", None, Some(wrong.clone())).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    let r = login(&app, "demo", json!({})).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(r.body["code"], "rate_limited");
    assert_eq!(r.headers[header::RETRY_AFTER], "900");
}

#[tokio::test]
async fn second_facteur_controle_ses_entrees() {
    let app = server();
    let token = login(&app, "demo", json!({})).await.body["token"]
        .as_str()
        .unwrap()
        .to_owned();

    // Un jeton de session complet n'attend pas de second facteur.
    let r = call(
        &app,
        "POST",
        "/v1/auth/mfa",
        None,
        Some(json!({ "pending": token, "method": "push" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    let r = call(
        &app,
        "POST",
        "/v1/auth/mfa",
        None,
        Some(json!({ "pending": "v1.dev.abc", "method": "code", "code": "123456" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.body["code"], "invalid_token");

    let r = call(
        &app,
        "POST",
        "/v1/auth/mfa",
        None,
        Some(json!({ "method": "sms" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn ancien_jeton_accepte_a_la_connexion() {
    let app = server();
    let old = login(&app, "demo", json!({})).await.body["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let r = login(&app, "demo", json!({ "previousToken": old })).await;
    assert_eq!(r.status, StatusCode::OK);
    // Un jeton illisible est ignoré, pas une erreur.
    let r = login(&app, "demo", json!({ "previousToken": "n'importe quoi" })).await;
    assert_eq!(r.status, StatusCode::OK);
}

#[tokio::test]
async fn connexion_a_un_service_controle_ses_entrees() {
    let app = server();
    let body = json!({ "service": "grades", "username": "camille.demo", "password": "x" });

    // Le jeton de session est obligatoire.
    let r = call(
        &app,
        "POST",
        "/v1/auth/service-login",
        None,
        Some(body.clone()),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    let token = login(&app, "demo", json!({})).await.body["token"]
        .as_str()
        .unwrap()
        .to_owned();

    // Service sans compte propre, inconnu, ou champ manquant : 400, sans jeton renvoyé.
    for body in [
        body.clone(),
        json!({ "service": "mail", "username": "u", "password": "p" }),
        json!({ "service": "grades", "username": "u" }),
    ] {
        let r = call(
            &app,
            "POST",
            "/v1/auth/service-login",
            Some(&token),
            Some(body),
        )
        .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.body);
        assert_eq!(r.body["code"], "invalid_request");
        assert!(r.headers.get("tonapi-session").is_none());
    }
    // Les identifiants envoyés n'apparaissent pas dans l'erreur.
    let r = call(
        &app,
        "POST",
        "/v1/auth/service-login",
        Some(&token),
        Some(body),
    )
    .await;
    assert!(!r.body.to_string().contains("camille.demo"));
}

#[tokio::test]
async fn appel_groupe_en_parallele() {
    let app = server();
    let token = login(&app, "demo", json!({})).await.body["token"]
        .as_str()
        .unwrap()
        .to_owned();

    // Par défaut : le profil et chaque service servi par l'API, dans l'ordre fixe.
    let r = call(&app, "GET", "/v1/overview", Some(&token), None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.headers[header::CACHE_CONTROL], "no-store");
    let services: Vec<&str> = r.body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["service"].as_str().unwrap())
        .collect();
    assert_eq!(
        services,
        [
            "profile",
            "timetable",
            "grades",
            "absences",
            "exams",
            "homework",
            "mail",
            "apps",
            "documents"
        ]
    );
    for item in r.body["data"].as_array().unwrap() {
        assert_eq!(item["provider"], "demo", "{item}");
        assert!(item["problem"].is_null(), "{item}");
        assert!(item["durationMs"].is_u64());
    }
    assert_eq!(r.body["meta"]["provider"], "overview");

    // Liste choisie, dates de l'emploi du temps comme `/v1/timetable`.
    let r = call(
        &app,
        "GET",
        "/v1/overview?services=timetable,timetable-groups&from=2026-10-12&to=2026-10-18",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body["data"][0]["data"].as_array().unwrap().len(), 9);
    assert_eq!(r.body["data"][1]["service"], "timetable-groups");

    let r = call(
        &app,
        "GET",
        "/v1/overview?services=notes",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.body["code"], "invalid_request");

    let r = call(&app, "GET", "/v1/overview", None, None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn disponibilite_pendant_l_arret() {
    let state = state();
    let readiness = state.readiness();
    let app = app(state);
    let r = call(&app, "GET", "/readyz", None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["status"], "ready");

    readiness.drain();
    let r = call(&app, "GET", "/readyz", None, None).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(r.body["code"], "service_unavailable");
    // L'instance sert encore les requêtes pendant que le répartiteur la retire.
    let r = call(&app, "GET", "/healthz", None, None).await;
    assert_eq!(r.status, StatusCode::OK);
}

#[tokio::test]
async fn identifiant_de_requete() {
    let app = server();
    let r = call(&app, "GET", "/healthz", None, None).await;
    let generated = r.headers["x-request-id"].to_str().unwrap().to_owned();
    assert!(!generated.is_empty());
    let r = call(&app, "GET", "/healthz", None, None).await;
    assert_ne!(r.headers["x-request-id"], generated.as_str());

    let reused = |id: &'static str| {
        let app = app.clone();
        async move {
            let response = app
                .oneshot(
                    Request::get("/healthz")
                        .header("x-request-id", id)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            response.headers()["x-request-id"]
                .to_str()
                .unwrap()
                .to_owned()
        }
    };
    assert_eq!(reused("lb-42.a_b").await, "lb-42.a_b");
    // Valeur douteuse remplacée : rien de ce qui vient du client n'entre tel quel au journal.
    assert_ne!(reused("a b").await, "a b");
}

#[tokio::test]
async fn metriques_par_route_modele_sans_valeur_de_compte() {
    let app = server();
    let r = login(&app, "demo", json!({})).await;
    let token = r.body["token"].as_str().unwrap().to_owned();
    let r = call(
        &app,
        "GET",
        "/v1/documents/doc-secret-42",
        Some(&token),
        None,
    )
    .await;
    assert!(r.status.is_client_error() || r.status.is_success());
    let _ = login(&app, "demo", json!({ "password": "faux" })).await;

    let response = metrics_app(None)
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/plain; version=0.0.4")
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        text.contains("tonapi_login_total{university=\"demo\",outcome=\"success\"}"),
        "{text}"
    );
    assert!(
        text.contains("tonapi_login_total{university=\"demo\",outcome=\"invalid_credentials\"}"),
        "{text}"
    );
    assert!(text.contains("tonapi_http_requests_in_flight "), "{text}");
    assert!(text.contains("tonapi_universities_loaded "), "{text}");
    assert!(
        text.contains("tonapi_limiter_entries{limiter=\"login\"}"),
        "{text}"
    );
    assert!(
        text.contains("tonapi_limiter_entries{limiter=\"slots\"}"),
        "{text}"
    );
    assert!(
        text.contains("tonapi_requests_total{route=\"/v1/auth/login\",status=\"200\"}"),
        "{text}"
    );
    assert!(text.contains("route=\"/v1/documents/{id}\""), "{text}");
    assert!(text.contains("tonapi_errors_total{code=\"invalid_credentials\",university=\"demo\"}"));
    assert!(text.contains("tonapi_session_token_bytes_count"));
    assert!(!text.contains("doc-secret-42"));
    assert!(!text.contains(&token));
}

async fn scrape(
    router: Router,
    authorization: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let mut request = Request::get("/metrics");
    if let Some(value) = authorization {
        request = request.header(header::AUTHORIZATION, value);
    }
    let response = router
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

#[tokio::test]
async fn le_routeur_public_ne_sert_pas_les_metriques() {
    let app = server();
    let r = call(&app, "GET", "/metrics", None, None).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.body["code"], "not_found");
    // Même avec un jeton : la route n'existe pas ici.
    let r = call(&app, "GET", "/metrics", Some("n-importe-quoi"), None).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn les_metriques_exigent_le_jeton_quand_il_est_defini() {
    let token = "t".repeat(40);
    let router = metrics_app(Some(&token));

    let (status, headers, body) = scrape(router.clone(), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(headers[header::WWW_AUTHENTICATE], "Bearer");
    assert!(body.is_empty(), "aucun détail dans le corps : {body}");

    for wrong in [
        format!("Bearer {}", "u".repeat(40)),
        format!("Bearer {}", "t".repeat(39)),
        format!("Bearer {token}x"),
        "Bearer ".to_owned(),
        format!("Basic {token}"),
        token.clone(),
    ] {
        let (status, _, body) = scrape(router.clone(), Some(&wrong)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{wrong}");
        assert!(body.is_empty());
    }

    let (status, headers, body) = scrape(router.clone(), Some(&format!("Bearer {token}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/plain; version=0.0.4")
    );
    assert!(body.contains("# HELP tonapi_requests_total"));
    // Le schéma se compare sans tenir compte de la casse.
    let (status, ..) = scrape(router, Some(&format!("bearer {token}"))).await;
    assert_eq!(status, StatusCode::OK);

    // Jeton vide : comme sans jeton (boucle locale seulement, voir `check_metrics_exposure`).
    let (status, ..) = scrape(metrics_app(Some("")), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn nouvelles_metriques_documentees_et_alimentees() {
    let app = server();
    // Six demandes sur le même compte : cinq échecs, puis le limiteur de comptes refuse.
    let wrong = json!({ "university": "demo", "username": "limite-metrique", "password": "faux" });
    for _ in 0..5 {
        let r = call(&app, "POST", "/v1/auth/login", None, Some(wrong.clone())).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    let r = call(&app, "POST", "/v1/auth/login", None, Some(wrong)).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    // Un jeton renouvelé : l'en-tête TonAPI-Session est renvoyé.
    let token = login(&app, "demo", json!({})).await.body["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let r = call(
        &app,
        "PUT",
        "/v1/me/establishment",
        Some(&token),
        Some(json!({ "establishment": null })),
    )
    .await;
    assert!(r.headers.contains_key("tonapi-session"));

    let (_, _, text) = scrape(metrics_app(None), None).await;
    for name in [
        "tonapi_rate_limited_total",
        "tonapi_login_total",
        "tonapi_config_reloads_total",
        "tonapi_session_renewed_total",
        "tonapi_http_requests_in_flight",
        "tonapi_limiter_entries",
        "tonapi_universities_loaded",
        "process_resident_memory_bytes",
        "process_open_fds",
        "process_max_fds",
    ] {
        assert!(text.contains(&format!("# HELP {name} ")), "{name}");
        assert!(text.contains(&format!("# TYPE {name} ")), "{name}");
    }
    assert!(
        text.contains("tonapi_rate_limited_total{limiter=\"login\"}"),
        "{text}"
    );
    assert!(
        text.contains("tonapi_login_total{university=\"demo\",outcome=\"rate_limited\"}"),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|l| l.starts_with("tonapi_session_renewed_total ")),
        "{text}"
    );
    // Aucune valeur de compte dans les étiquettes.
    assert!(!text.contains("limite-metrique"));
}

#[tokio::test]
async fn catalogue_des_universites_en_cache_public_sauf_en_dev() {
    for path in ["/v1/universities", "/v1/universities/demo"] {
        let r = call(&server(), "GET", path, None, None).await;
        assert_eq!(r.status, StatusCode::OK, "{path}");
        assert_eq!(r.headers[header::CACHE_CONTROL], "public, max-age=300");

        let dev = app(state_with(false));
        let r = call(&dev, "GET", path, None, None).await;
        assert_eq!(r.status, StatusCode::OK, "{path}");
        assert!(!r.headers.contains_key(header::CACHE_CONTROL), "{path}");
    }
    // Une erreur ne se met jamais en cache public.
    let r = call(&server(), "GET", "/v1/universities/inconnue", None, None).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert!(!r.headers.contains_key(header::CACHE_CONTROL));
    // Les données personnelles restent `no-store`.
    let app = server();
    let token = login(&app, "demo", json!({})).await.body["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let r = call(&app, "GET", "/v1/me", Some(&token), None).await;
    assert_eq!(r.headers[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn controle_prealable_cors_retenu_dix_minutes() {
    let request = Request::builder()
        .method("OPTIONS")
        .uri("/v1/universities")
        .header(header::ORIGIN, "https://appli.example")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .body(Body::empty())
        .unwrap();
    let response = server().oneshot(request).await.unwrap();
    assert_eq!(response.headers()[header::ACCESS_CONTROL_MAX_AGE], "600");
}
