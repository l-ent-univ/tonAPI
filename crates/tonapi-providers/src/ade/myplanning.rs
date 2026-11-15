//! Fournisseur `ade-myplanning` : emploi du temps ADE pour une interface « MyPlanning » (GWT).
//!
//! L'interface `direct/myplanning.jsp` ne communique qu'en GWT-RPC. On rejoue les appels qu'elle
//! fait au chargement, derrière le CAS de l'université :
//! 1. `MyPlanningClientServiceProxy.login` renvoie les critères de l'étudiant (codes de ses
//!    enseignements, par champ ADE) ;
//! 2. `WebClientServiceProxy.loadProject` ouvre le projet ;
//! 3. `DirectPlanningServiceProxy.getResourceIds` traduit ces critères en numéros de ressources.
//!
//! Le flux iCal vient ensuite de `jsp/custom/modules/plannings/anonymous_cal.jsp`, sans cookie.
//! Les numéros sont gardés dans la session ; ils sont relus si le flux devient vide ou illisible.
//!
//! Les « policies » GWT (une signature par service) changent à chaque version d'ADE : elles sont
//! lues dans le JavaScript public de la permutation (`<permutation>.cache.html`), jamais figées.
//! [vu] Paul-Valéry, ADE 6.15, 27/10/2026.
//!
//! Groupes : `ConfigurationServiceProxy.searchDataById` donne l'arbre des dossiers qui mènent aux
//! ressources de l'étudiant (numéros seulement) ; `DirectPlanningServiceProxy.getChildren` ouvre
//! ces dossiers un par un et donne les noms. Comme MyPlanning, on n'ouvre que ce chemin.

use std::collections::{BTreeMap, HashSet, VecDeque};

use chrono::NaiveDate;
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode};
use tonapi_core::{Course, ErrorCode, Problem, TimetableGroup};
use url::Url;

use std::time::Duration;

use crate::Context;
use crate::ade::ical_link::{is_calendar, parse_timetable};
use crate::cache::Shared;
use crate::common::{root, token_key, university_tz};
use crate::net::{Net, upstream};

mod gwt;
mod tree;

use gwt::{GwtResponse, gwt_long, integer_list, login_criteria, permutations, service_policies};
use tree::{TREE_ROOT, TreeNode, children, path_folders};

/// Signatures GWT par module : les mêmes pour tous les étudiants, jusqu'à la prochaine version.
static POLICIES: Shared<Vec<(String, String)>> = Shared::new(Duration::from_secs(3600));
/// Enfants d'un dossier de l'arbre des ressources (noms des groupes), par module et nœud.
static CHILDREN: Shared<Vec<TreeNode>> = Shared::new(Duration::from_secs(3600));

const PROVIDER: &str = "ade-myplanning";
const MODULE: &str = "direct/gwtdirectplanning/";
const ENTRY: &str = "direct/myplanning.jsp";
const FEED: &str = "jsp/custom/modules/plannings/anonymous_cal.jsp";
/// Au-delà, l'URL du flux deviendrait trop longue.
const MAX_RESOURCES: usize = 200;

/// Dossiers ouverts au plus pour lister les groupes (MyPlanning en ouvre 25 pour 14 ressources).
const MAX_FOLDERS: usize = 80;

// Récupération

pub(crate) async fn fetch_timetable(
    ctx: &mut Context<'_>,
    base_url: &Url,
    project_id: u32,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Course>, Problem> {
    let tz = university_tz(ctx)?;
    let base = root(base_url);
    let key = token_key("ade-myplanning", &base);
    let stored: Option<Vec<u32>> = ctx
        .session
        .tokens
        .get(&key)
        .map(|raw| raw.split(',').filter_map(|v| v.parse().ok()).collect())
        .filter(|ids: &Vec<u32>| !ids.is_empty());

    let mut net = Net::open(ctx, PROVIDER)?;
    let result = read(&mut net, &base, project_id, from, to, stored).await;
    net.save(ctx);
    let (ics, ids) = result?;
    ctx.session.tokens.insert(
        key,
        ids.iter().map(u32::to_string).collect::<Vec<_>>().join(","),
    );
    parse_timetable(&ics, tz, from, to)
}

/// Flux des ressources mémorisées, sinon des ressources relues dans MyPlanning (une fois).
async fn read(
    net: &mut Net,
    base: &Url,
    project_id: u32,
    from: NaiveDate,
    to: NaiveDate,
    stored: Option<Vec<u32>>,
) -> Result<(String, Vec<u32>), Problem> {
    if let Some(ids) = stored
        && let Some(ics) = download(net, base, project_id, &ids, from, to).await?
    {
        return Ok((ics, ids));
    }
    let (_, ids) = discover(net, base, project_id).await?;
    match download(net, base, project_id, &ids, from, to).await? {
        Some(ics) => Ok((ics, ids)),
        None => Err(upstream(
            PROVIDER,
            "le flux iCal d'ADE ne renvoie pas un calendrier",
        )),
    }
}

/// `anonymous_cal.jsp` pour la plage demandée, sans cookie de la session.
async fn download(
    net: &mut Net,
    base: &Url,
    project_id: u32,
    ids: &[u32],
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Option<String>, Problem> {
    let mut url = base
        .join(FEED)
        .map_err(|_| upstream(PROVIDER, "adresse ADE illisible"))?;
    url.query_pairs_mut()
        .append_pair(
            "resources",
            &ids.iter().map(u32::to_string).collect::<Vec<_>>().join(","),
        )
        .append_pair("projectId", &project_id.to_string())
        .append_pair("calType", "ical")
        .append_pair("firstDate", &from.format("%Y-%m-%d").to_string())
        .append_pair("lastDate", &to.format("%Y-%m-%d").to_string());
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("text/calendar, text/*;q=0.9"),
    );
    let jar = std::mem::take(&mut net.browser.jar);
    let reply = net.request(Method::GET, url, headers, None).await;
    net.browser.jar = jar;
    let (status, _, body) = reply?;
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        return Err(Problem::new(ErrorCode::ServiceUnavailable)
            .detail(format!("{PROVIDER} : le serveur ADE répond HTTP {status}")));
    }
    Ok((status.is_success() && is_calendar(&body)).then_some(body))
}

/// Groupes de l'étudiant : ses ressources ADE, nommées, avec le chemin des dossiers.
pub(crate) async fn fetch_groups(
    ctx: &mut Context<'_>,
    base_url: &Url,
    project_id: u32,
) -> Result<Vec<TimetableGroup>, Problem> {
    let base = root(base_url);
    let mut net = Net::open(ctx, PROVIDER)?;
    let result = groups(&mut net, &base, project_id).await;
    net.save(ctx);
    let (groups, ids) = result?;
    ctx.session.tokens.insert(
        token_key("ade-myplanning", &base),
        ids.iter().map(u32::to_string).collect::<Vec<_>>().join(","),
    );
    Ok(groups)
}

async fn groups(
    net: &mut Net,
    base: &Url,
    project_id: u32,
) -> Result<(Vec<TimetableGroup>, Vec<u32>), Problem> {
    let (rpc, ids) = discover(net, base, project_id).await?;
    let reply = rpc
        .call(net, "ConfigurationServiceProxy", &rpc.search_body(&ids)?)
        .await?;
    let folders = path_folders(reply.text().ok_or_else(bad_tree)?)?;
    let wanted: HashSet<i64> = ids.iter().map(|&id| i64::from(id)).collect();

    let mut found: BTreeMap<i64, TimetableGroup> = BTreeMap::new();
    let mut queue = VecDeque::from([(TREE_ROOT.to_owned(), Vec::<String>::new())]);
    let mut opened = 0;
    while let Some((node, path)) = queue.pop_front() {
        opened += 1;
        if opened > MAX_FOLDERS {
            break;
        }
        let key = format!("{}|{node}", rpc.module);
        let nodes = if let Some(nodes) = CHILDREN.get(&key) {
            nodes
        } else {
            let reply = rpc
                .call(
                    net,
                    "DirectPlanningServiceProxy",
                    &rpc.children_body(&node)?,
                )
                .await?;
            let nodes = children(reply.text().ok_or_else(bad_tree)?)?;
            CHILDREN.put(key, nodes.clone());
            nodes
        };
        for child in nodes {
            if child.folder && folders.contains(&child.id) {
                let mut inner = path.clone();
                // Les numéros négatifs sont les catégories d'ADE, pas des dossiers nommés.
                if child.id >= 0 {
                    inner.push(child.name.clone());
                }
                queue.push_back((child.raw, inner));
            } else if !child.folder && wanted.contains(&child.id) {
                found.entry(child.id).or_insert_with(|| TimetableGroup {
                    id: child.id.to_string(),
                    name: child.name,
                    path: path.clone(),
                    selected: true,
                });
            }
        }
    }
    if found.is_empty() {
        return Err(upstream(
            PROVIDER,
            "aucune ressource de l'étudiant dans l'arbre ADE",
        ));
    }
    Ok((found.into_values().collect(), ids))
}

fn bad_tree() -> Problem {
    upstream(PROVIDER, "arbre des ressources ADE illisible")
}

/// Numéros des ressources de l'étudiant, par les appels GWT de MyPlanning.
async fn discover(net: &mut Net, base: &Url, project_id: u32) -> Result<(Rpc, Vec<u32>), Problem> {
    let entry = base
        .join(ENTRY)
        .map_err(|_| upstream(PROVIDER, "adresse ADE illisible"))?;
    let page = net.open_protected(entry).await?;
    if !page.status.is_success() {
        return Err(upstream(
            PROVIDER,
            format!("la page MyPlanning répond HTTP {}", page.status),
        ));
    }
    let module = base
        .join(MODULE)
        .map_err(|_| upstream(PROVIDER, "adresse ADE illisible"))?;
    let cached = POLICIES.get(module.as_str());
    let reused = cached.is_some();
    let policies = match cached {
        Some(policies) => policies,
        None => fresh_policies(net, &module).await?,
    };
    let mut rpc = Rpc {
        module,
        policies,
        client: gwt_long(chrono::Utc::now().timestamp_millis()),
    };

    let login = match rpc
        .call(net, "MyPlanningClientServiceProxy", &rpc.login_body()?)
        .await
    {
        // Signatures gardées mais refusées : ADE a changé de version, on les relit une fois.
        Err(problem) if reused && problem.code == ErrorCode::UpstreamChanged => {
            POLICIES.remove(rpc.module.as_str());
            rpc.policies = fresh_policies(net, &rpc.module).await?;
            rpc.call(net, "MyPlanningClientServiceProxy", &rpc.login_body()?)
                .await?
        }
        other => other?,
    };
    let criteria = login_criteria(&login)?;
    rpc.call(
        net,
        "WebClientServiceProxy",
        &rpc.load_project_body(project_id)?,
    )
    .await?;
    let body = rpc.resource_ids_body(&login, &criteria)?;
    let reply = rpc.call(net, "DirectPlanningServiceProxy", &body).await?;
    let ids = integer_list(&reply)?;
    if ids.is_empty() {
        return Err(Problem::new(ErrorCode::ServiceAbsent)
            .detail("ADE ne trouve aucune ressource pour cet étudiant"));
    }
    if ids.len() > MAX_RESOURCES {
        return Err(upstream(
            PROVIDER,
            "trop de ressources ADE pour un étudiant",
        ));
    }
    Ok((rpc, ids))
}

/// Signatures relues dans le code public, puis gardées pour les étudiants suivants.
async fn fresh_policies(net: &mut Net, module: &Url) -> Result<Vec<(String, String)>, Problem> {
    let found = policies(net, module).await?;
    POLICIES.put(module.to_string(), found.clone());
    Ok(found)
}

/// Signatures (`policy`) de chaque service, lues dans le code de la première permutation
/// disponible annoncée par `*.nocache.js`.
async fn policies(net: &mut Net, module: &Url) -> Result<Vec<(String, String)>, Problem> {
    let nocache = module
        .join("gwtdirectplanning.nocache.js")
        .map_err(|_| upstream(PROVIDER, "adresse ADE illisible"))?;
    let (status, _, script) = net
        .request(Method::GET, nocache, HeaderMap::new(), None)
        .await?;
    if !status.is_success() {
        return Err(upstream(PROVIDER, "chargeur GWT introuvable"));
    }
    for permutation in permutations(&script).into_iter().take(16) {
        let Ok(url) = module.join(&format!("{permutation}.cache.html")) else {
            continue;
        };
        let (status, _, code) = net
            .request(Method::GET, url, HeaderMap::new(), None)
            .await?;
        if !status.is_success() {
            continue;
        }
        let found = service_policies(&code);
        if !found.is_empty() {
            return Ok(found);
        }
    }
    Err(upstream(PROVIDER, "signatures GWT introuvables"))
}

/// Client GWT-RPC : module, signatures et identifiant de client.
struct Rpc {
    module: Url,
    policies: Vec<(String, String)>,
    client: String,
}

impl Rpc {
    fn policy(&self, service: &str) -> Result<&str, Problem> {
        self.policies
            .iter()
            .find(|(name, _)| name == service)
            .map(|(_, hash)| hash.as_str())
            .ok_or_else(|| upstream(PROVIDER, format!("signature GWT absente pour {service}")))
    }

    async fn call(&self, net: &mut Net, service: &str, body: &str) -> Result<GwtResponse, Problem> {
        let url = self
            .module
            .join(service)
            .map_err(|_| upstream(PROVIDER, "adresse ADE illisible"))?;
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/x-gwt-rpc; charset=utf-8"),
        );
        headers.insert("X-GWT-Permutation", HeaderValue::from_static("HostedMode"));
        if let Ok(value) = HeaderValue::from_str(self.module.as_str()) {
            headers.insert("X-GWT-Module-Base", value);
        }
        let (status, _, text) = net
            .request(Method::POST, url, headers, Some(body.to_owned()))
            .await?;
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(Problem::new(ErrorCode::SessionExpired));
        }
        if !status.is_success() {
            return Err(upstream(
                PROVIDER,
                format!("appel GWT {service} : HTTP {status}"),
            ));
        }
        GwtResponse::parse(&text).ok_or_else(|| {
            upstream(
                PROVIDER,
                format!("réponse GWT {service} refusée ou illisible"),
            )
        })
    }
}
