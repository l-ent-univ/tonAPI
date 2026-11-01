//! Parcours complet sur l'université de démonstration, sans réseau.

use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tonapi_config::Registry;
use tonapi_server::{AppState, Settings, app};
use tonapi_session::SessionKeys;
use tower::ServiceExt;

fn server() -> Router {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../universities");
    let (registry, report) = Registry::load(&dir);
    assert!(report.is_ok());
    app(AppState::new(
        registry,
        SessionKeys::ephemeral().unwrap(),
        Settings {
            universities_dir: dir,
            session_ttl_secs: 3600,
            trusted_proxies: Vec::new(),
            docs: true,
            auth_per_ip: 120,
        },
    ))
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
