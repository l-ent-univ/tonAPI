//! Fournisseur `ade-myplanning` de bout en bout contre un faux ADE local : CAS, page MyPlanning,
//! chargeur GWT, code de permutation, quatre services GWT-RPC et `anonymous_cal.jsp`.
//! Codes d'enseignement et numéros de ressources inventés.

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Response, StatusCode, Uri, header};
use chrono::{NaiveDate, TimeZone, Utc};
use tonapi_config::ResolvedUniversity;
use tonapi_core::{Course, ErrorCode, Problem, TimetableGroup};
use tonapi_providers::{Context, NetSettings, timetable, timetable_groups};
use tonapi_session::SessionData;

use common::{has_cookie, query, redirect};

const SEMAINE: &str = include_str!("fixtures/ade_ical_link/semaine.ics");
const PERMUTATION: &str = "1987F1EF14DA1BA12007E141147356DC";
const POLICY_DIRECT: &str = "AF8E80FD952FAE1F1E1D64FCA0763541";
const POLICY_WEB: &str = "5EB99F51B592E97DD3B5315B286394D6";
const POLICY_MY: &str = "E3E2173A11FF04C6E8A538A7E7A002AF";
const POLICY_CONF: &str = "0F1E2D3C4B5A69788796A5B4C3D2E1F0";
const LOGIN_REPLY: &str = r#"//OK[0,1,11,9,10,9,2,2,1,8,1,7,6,5,0,2,4,3,1,2,1,["com.adesoft.gwt.directplan.client.rpc.data.MyPlanningLoginReponse/1769343849","java.util.ArrayList/4159755760","com.adesoft.gwt.directplan.client.rpc.data.ProjectParameter/3599371664","com.adesoft.gwt.directplan.client.UrlParameter/970611799","web","","java.util.HashMap/1797211028","com.adesoft.gwt.directplan.client.rpc.ResourceFieldCriteria/1324434193","java.lang.String/2004016611","S1-DEMO-001","S1-DEMO-002"],0,7]"#;
const IDS_REPLY: &str = r#"//OK[4242,2,4141,2,2,1,["java.util.ArrayList/4159755760","java.lang.Integer/3438268394"],0,7]"#;
/// Chemin des ressources 4141 et 4242 : catégorie -1, dossiers 10 puis 20.
const SEARCH_TREE: &str = r#"{"-100""true""0""-1""-1""-1""0""false"[0]""[0][1]{"-1""true""0""-1""-1""-1""0""false"[0]""[0][1]{"10""true""0""-1""-1""-1""0""false"[0]""[0][1]{"20""true""0""-1""-1""-1""0""false"[0]""[2]{"4141""false""0""-1""-1""-1""0""false"[0]""[0]{"4242""false""0""-1""-1""-1""0""false"[0]""[0][0]"#;

/// Nœud de `getChildren` avec son nom (format d'ADE, valeurs inventées).
fn node(id: &str, folder: bool, name: &str) -> String {
    format!(
        r#"{{"{id}""{folder}""1""-1""0""0""0""false"[2]{{"ColorField""COLOR""LabelColor""0,0,0""false""false"{{"StringField""NAME""LabelName""{name}""false""false""{name}""trainee""1""0"[0][0]"#
    )
}

/// Enfants d'un dossier, enveloppés comme une réponse GWT.
fn children_reply(opened: &str) -> String {
    let id = opened
        .strip_prefix("{\"")
        .and_then(|r| r.split('"').next())
        .unwrap_or("");
    let kids = match id {
        "-100" => vec![node("-1", true, "Groupes d'étudiants")],
        "-1" => vec![
            node("10", true, "Licence Démo"),
            node("11", true, "Hors chemin"),
        ],
        "10" => vec![node("20", true, "L1 Démo")],
        "20" => vec![
            node("4141", false, "Groupe A"),
            node("4242", false, "Groupe B"),
            node("4343", false, "Groupe C"),
        ],
        other => panic!("dossier {other} ouvert hors du chemin"),
    };
    // ADE termine la réponse par une valeur après les enfants.
    let text = format!(r#"{{"0"{opened}[{}]{}"false""#, kids.len(), kids.concat());
    // GWT écrit l'apostrophe en \x27.
    let quoted = serde_json::to_string(&text).unwrap().replace('\'', "\\x27");
    format!("//OK[1,[{quoted}],0,7]")
}

#[derive(Default)]
struct Mock {
    base: String,
    /// Appels GWT reçus : service et méthode.
    calls: Vec<String>,
    /// `resources` des demandes de flux.
    feeds: Vec<String>,
    /// Les demandes de flux renvoient une page HTML (numéros périmés).
    stale_feed: bool,
    /// Lectures du chargeur GWT (signatures).
    loader_reads: u32,
}

type Shared = Arc<Mutex<Mock>>;

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
        "/direct/gwtdirectplanning/gwtdirectplanning.nocache.js" => {
            s.loader_reads += 1;
            reply(
                StatusCode::OK,
                "text/javascript",
                None,
                format!("var a='0000000000000000000000000000000A',b='{PERMUTATION}';"),
            )
        }
        p if p == format!("/direct/gwtdirectplanning/{PERMUTATION}.cache.html") => reply(
            StatusCode::OK,
            "text/html",
            None,
            format!(
                "function a(){{Yx.call(this,EEd(),'DirectPlanningServiceProxy','{POLICY_DIRECT}',X)}} function b(){{Yx.call(this,EEd(),'WebClientServiceProxy','{POLICY_WEB}',X)}} function c(){{Yx.call(this,EEd(),'MyPlanningClientServiceProxy','{POLICY_MY}',X)}} function d(){{Yx.call(this,EEd(),'ConfigurationServiceProxy','{POLICY_CONF}',X)}}"
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
                "method25searchDataById" => {
                    assert!(body.ends_with("|6|3|8|2|9|4141|9|4242|"), "{body}");
                    let quoted = serde_json::to_string(SEARCH_TREE).unwrap();
                    (POLICY_CONF, format!("//OK[1,[{quoted}],0,7]"))
                }
                "method4getChildren" => {
                    let opened = parts.get(10).copied().unwrap_or("");
                    (POLICY_DIRECT, children_reply(opened))
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
    let (listener, base) = common::listen().await;
    let state: Shared = Arc::new(Mutex::new(Mock {
        base: base.clone(),
        ..Mock::default()
    }));
    let app = Router::new().fallback(handle).with_state(state.clone());
    common::spawn(listener, app);
    (state, base)
}

fn university(base: &str) -> ResolvedUniversity {
    common::university(serde_json::json!({
        "id": "test-ade",
        "name": "Université de test",
        "sso": { "kind": "cas", "base-url": format!("{base}/cas/") },
        "services": { "timetable": { "provider": "ade-myplanning", "base-url": format!("{base}/") } },
    }))
}

fn session(tgc: bool) -> SessionData {
    let cookies = if tgc {
        vec![common::stored_cookie("TGC", "TGT-demo", "/")]
    } else {
        Vec::new()
    };
    common::session("test-ade", "e00000000000", cookies)
}

async fn week(
    university: &ResolvedUniversity,
    session: &mut SessionData,
) -> Result<Vec<Course>, Problem> {
    let mut ctx = Context {
        university,
        session,
        now: Utc.with_ymd_and_hms(2026, 10, 5, 8, 0, 0).unwrap(),
        net: NetSettings {
            https_only: false,
            user_agent: None,
        },
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

#[tokio::test]
async fn groupes_nommes_par_l_arbre_ade() {
    let (state, base) = start().await;
    let uni = university(&base);
    let mut session = session(true);
    let mut ctx = Context {
        university: &uni,
        session: &mut session,
        now: Utc.with_ymd_and_hms(2026, 10, 5, 8, 0, 0).unwrap(),
        net: NetSettings {
            https_only: false,
            user_agent: None,
        },
    };
    let groups = timetable_groups(&mut ctx).await.unwrap();
    assert_eq!(groups.provider, "ade-myplanning");
    assert_eq!(
        groups.data,
        [
            TimetableGroup {
                id: "4141".to_owned(),
                name: "Groupe A".to_owned(),
                path: vec!["Licence Démo".to_owned(), "L1 Démo".to_owned()],
                selected: true,
            },
            TimetableGroup {
                id: "4242".to_owned(),
                name: "Groupe B".to_owned(),
                path: vec!["Licence Démo".to_owned(), "L1 Démo".to_owned()],
                selected: true,
            },
        ]
    );
    let s = state.lock().unwrap();
    // Racine, catégorie, deux dossiers : le dossier 11 hors chemin n'est pas ouvert.
    assert_eq!(
        s.calls
            .iter()
            .filter(|c| *c == "method4getChildren")
            .count(),
        4
    );
    let key = format!("ade-myplanning.{}", base.trim_start_matches("http://"));
    assert_eq!(
        session.tokens.get(&key).map(String::as_str),
        Some("4141,4242")
    );
}

#[tokio::test]
async fn deuxieme_etudiant_sans_relire_signatures_ni_arbre() {
    let (state, base) = start().await;
    let uni = university(&base);
    for _ in 0..2 {
        let mut session = session(true);
        let mut ctx = Context {
            university: &uni,
            session: &mut session,
            now: Utc.with_ymd_and_hms(2026, 10, 5, 8, 0, 0).unwrap(),
            net: NetSettings {
                https_only: false,
                user_agent: None,
            },
        };
        let groups = timetable_groups(&mut ctx).await.unwrap().data;
        assert_eq!(groups.len(), 2);
    }
    let s = state.lock().unwrap();
    assert_eq!(s.loader_reads, 1, "signatures GWT lues une fois");
    assert_eq!(
        s.calls
            .iter()
            .filter(|c| *c == "method4getChildren")
            .count(),
        4,
        "arbre ouvert pour le premier étudiant seulement"
    );
}
