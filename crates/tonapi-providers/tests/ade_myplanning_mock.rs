//! Fournisseur `ade-myplanning` de bout en bout contre un faux ADE local : CAS, page MyPlanning,
//! chargeur GWT, code de permutation, trois services GWT-RPC et `anonymous_cal.jsp`.
//! Codes d'enseignement et numéros de ressources inventés.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Response, StatusCode, Uri, header};
use chrono::{NaiveDate, TimeZone, Utc};
use tonapi_config::{ResolvedUniversity, UniversityFile};
use tonapi_core::{Course, ErrorCode, Problem};
use tonapi_providers::{Context, NetSettings, timetable};
use tonapi_session::{SessionData, StoredCookie};

const SEMAINE: &str = include_str!("fixtures/ade_ical/semaine.ics");
const PERMUTATION: &str = "1987F1EF14DA1BA12007E141147356DC";
const POLICY_DIRECT: &str = "AF8E80FD952FAE1F1E1D64FCA0763541";
const POLICY_WEB: &str = "5EB99F51B592E97DD3B5315B286394D6";
const POLICY_MY: &str = "E3E2173A11FF04C6E8A538A7E7A002AF";
const LOGIN_REPLY: &str = r#"//OK[0,1,11,9,10,9,2,2,1,8,1,7,6,5,0,2,4,3,1,2,1,["com.adesoft.gwt.directplan.client.rpc.data.MyPlanningLoginReponse/1769343849","java.util.ArrayList/4159755760","com.adesoft.gwt.directplan.client.rpc.data.ProjectParameter/3599371664","com.adesoft.gwt.directplan.client.UrlParameter/970611799","web","","java.util.HashMap/1797211028","com.adesoft.gwt.directplan.client.rpc.ResourceFieldCriteria/1324434193","java.lang.String/2004016611","S1-DEMO-001","S1-DEMO-002"],0,7]"#;
const IDS_REPLY: &str = r#"//OK[4242,2,4141,2,2,1,["java.util.ArrayList/4159755760","java.lang.Integer/3438268394"],0,7]"#;

#[derive(Default)]
struct Mock {
    base: String,
    /// Appels GWT reçus : service et méthode.
    calls: Vec<String>,
    /// `resources` des demandes de flux.
    feeds: Vec<String>,
    /// Les demandes de flux renvoient une page HTML (numéros périmés).
    stale_feed: bool,
}

type Shared = Arc<Mutex<Mock>>;

fn query(uri: &Uri, key: &str) -> Option<String> {
    url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

fn reply(
    status: StatusCode,
    content_type: &str,
    cookie: Option<&str>,
    body: String,
) -> Response<Body> {
    let mut r = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type);
    if let Some(cookie) = cookie {
        r = r.header(header::SET_COOKIE, cookie);
    }
    r.body(Body::from(body)).unwrap()
}

fn redirect(location: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location)
        .body(Body::empty())
        .unwrap()
}

fn has_cookie(headers: &HeaderMap, pair: &str) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .is_some_and(|c| c.split("; ").any(|p| p == pair))
}

async fn handle(
    State(state): State<Shared>,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response<Body> {
    let mut s = state.lock().unwrap();
    let base = s.base.clone();
    match uri.path() {
        "/cas/login" => match query(&uri, "service") {
            Some(service) if has_cookie(&headers, "TGC=TGT-demo") => {
                redirect(&format!("{service}?ticket=ST-1"))
            }
            _ => reply(
                StatusCode::OK,
                "text/html",
                None,
                r#"<form method="post" id="fm1" action="login"><input name="username"><input type="password" name="password"><input type="hidden" name="execution" value="e1"></form>"#.to_owned(),
            ),
        },
        "/direct/myplanning.jsp" => {
            if query(&uri, "ticket").is_none() && !has_cookie(&headers, "JSESSIONID=ade") {
                let service = url::form_urlencoded::byte_serialize(
                    format!("{base}/direct/myplanning.jsp").as_bytes(),
                )
                .collect::<String>();
                return redirect(&format!("{base}/cas/login?service={service}"));
            }
            reply(
                StatusCode::OK,
                "text/html",
                Some("JSESSIONID=ade; Path=/direct"),
                "<html><title>Ade Web MyPlanning</title></html>".to_owned(),
            )
        }
        "/direct/gwtdirectplanning/gwtdirectplanning.nocache.js" => reply(
            StatusCode::OK,
            "text/javascript",
            None,
            format!("var a='0000000000000000000000000000000A',b='{PERMUTATION}';"),
        ),
        p if p == format!("/direct/gwtdirectplanning/{PERMUTATION}.cache.html") => reply(
            StatusCode::OK,
            "text/html",
            None,
            format!(
                "function a(){{Yx.call(this,EEd(),'DirectPlanningServiceProxy','{POLICY_DIRECT}',X)}} function b(){{Yx.call(this,EEd(),'WebClientServiceProxy','{POLICY_WEB}',X)}} function c(){{Yx.call(this,EEd(),'MyPlanningClientServiceProxy','{POLICY_MY}',X)}}"
            ),
        ),
        p if p.starts_with("/direct/gwtdirectplanning/") && p.ends_with("ServiceProxy") => {
            if !has_cookie(&headers, "JSESSIONID=ade")
                || headers.get("X-GWT-Permutation").is_none()
            {
                return reply(StatusCode::FORBIDDEN, "text/plain", None, String::new());
            }
            let parts: Vec<&str> = body.split('|').collect();
            let (policy, method) = (parts.get(4).copied(), parts.get(6).copied().unwrap_or(""));
            s.calls.push(method.to_owned());
            let (expected, answer) = match method {
                "method1login" => (POLICY_MY, LOGIN_REPLY.to_owned()),
                "method6loadProject" => (POLICY_WEB, "//OK[[],0,7]".to_owned()),
                "method7getResourceIds" => {
                    // Les critères de la connexion doivent revenir tels quels.
                    assert!(body.contains("|S1-DEMO-001|S1-DEMO-002|"), "{body}");
                    assert!(body.contains("|10|1|11|1|9|2|12|"), "{body}");
                    (POLICY_DIRECT, IDS_REPLY.to_owned())
                }
                _ => return reply(StatusCode::OK, "text/plain", None, "//EX[0,[],0,7]".to_owned()),
            };
            if policy != Some(expected) {
                return reply(StatusCode::OK, "text/plain", None, "//EX[0,[],0,7]".to_owned());
            }
            reply(StatusCode::OK, "text/plain; charset=utf-8", None, answer)
        }
        "/jsp/custom/modules/plannings/anonymous_cal.jsp" => {
            // Le flux est public : aucun cookie de session ne doit l'accompagner.
            assert!(headers.get(header::COOKIE).is_none());
            s.feeds.push(query(&uri, "resources").unwrap_or_default());
            if s.stale_feed {
                return reply(StatusCode::OK, "text/html", None, "<html></html>".to_owned());
            }
            reply(StatusCode::OK, "text/calendar;charset=UTF-8", None, SEMAINE.to_owned())
        }
        _ => reply(StatusCode::NOT_FOUND, "text/plain", None, String::new()),
    }
}

async fn start() -> (Shared, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let state: Shared = Arc::new(Mutex::new(Mock {
        base: base.clone(),
        ..Mock::default()
    }));
    let app = Router::new().fallback(handle).with_state(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (state, base)
}

fn university(base: &str) -> ResolvedUniversity {
    let file: UniversityFile = serde_json::from_value(serde_json::json!({
        "id": "test-ade",
        "name": "Université de test",
        "sso": { "kind": "cas", "base-url": format!("{base}/cas/") },
        "services": { "timetable": { "provider": "ade-myplanning", "base-url": format!("{base}/") } },
    }))
    .unwrap();
    ResolvedUniversity {
        dir: std::path::PathBuf::new(),
        university: file,
        establishments: BTreeMap::new(),
    }
}

fn session(tgc: bool) -> SessionData {
    SessionData {
        university: "test-ade".to_owned(),
        username: "e00000000000".to_owned(),
        establishment: None,
        cookies: if tgc {
            vec![StoredCookie {
                domain: "127.0.0.1".to_owned(),
                path: "/".to_owned(),
                name: "TGC".to_owned(),
                value: "TGT-demo".to_owned(),
                host_only: true,
                expires: None,
            }]
        } else {
            Vec::new()
        },
        tokens: BTreeMap::new(),
        pending: false,
        issued_at: 0,
        expires_at: i64::MAX,
    }
}

async fn week(
    university: &ResolvedUniversity,
    session: &mut SessionData,
) -> Result<Vec<Course>, Problem> {
    let mut ctx = Context {
        university,
        session,
        now: Utc.with_ymd_and_hms(2026, 10, 5, 8, 0, 0).unwrap(),
        net: NetSettings { https_only: false },
    };
    let from = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 10, 11).unwrap();
    timetable(&mut ctx, from, to).await.map(|f| f.data)
}

#[tokio::test]
async fn ressources_lues_par_gwt_puis_flux_anonyme() {
    let (state, base) = start().await;
    let uni = university(&base);
    let mut session = session(true);

    let courses = week(&uni, &mut session).await.unwrap();
    assert_eq!(courses.len(), 4);
    {
        let s = state.lock().unwrap();
        assert_eq!(
            s.calls,
            [
                "method1login",
                "method6loadProject",
                "method7getResourceIds"
            ]
        );
        assert_eq!(s.feeds, ["4141,4242"]);
    }
    let key = format!("ade-myplanning.{}", base.trim_start_matches("http://"));
    assert_eq!(
        session.tokens.get(&key).map(String::as_str),
        Some("4141,4242")
    );

    // Deuxième appel : les numéros gardés suffisent, aucun appel GWT.
    week(&uni, &mut session).await.unwrap();
    let s = state.lock().unwrap();
    assert_eq!(s.calls.len(), 3);
    assert_eq!(s.feeds.len(), 2);
}

#[tokio::test]
async fn numeros_perimes_relus_une_seule_fois() {
    let (state, base) = start().await;
    state.lock().unwrap().stale_feed = true;
    let uni = university(&base);
    let mut session = session(true);
    let key = format!("ade-myplanning.{}", base.trim_start_matches("http://"));
    session.tokens.insert(key, "1,2".to_owned());

    let problem = week(&uni, &mut session).await.unwrap_err();
    assert_eq!(problem.code, ErrorCode::UpstreamChanged);
    let s = state.lock().unwrap();
    // Flux des anciens numéros, relecture GWT, flux des nouveaux : pas plus.
    assert_eq!(s.feeds, ["1,2", "4141,4242"]);
    assert_eq!(s.calls.len(), 3);
}

#[tokio::test]
async fn sans_session_cas_la_session_est_expiree() {
    let (_state, base) = start().await;
    let uni = university(&base);
    let problem = week(&uni, &mut session(false)).await.unwrap_err();
    assert_eq!(problem.code, ErrorCode::SessionExpired);
}
