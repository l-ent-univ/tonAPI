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

use crate::Context;
use crate::ade_ical::{is_calendar, parse_timetable, university_tz};
use crate::net::{Net, upstream};

const PROVIDER: &str = "ade-myplanning";
const MODULE: &str = "direct/gwtdirectplanning/";
const ENTRY: &str = "direct/myplanning.jsp";
const FEED: &str = "jsp/custom/modules/plannings/anonymous_cal.jsp";
/// Au-delà, l'URL du flux deviendrait trop longue.
const MAX_RESOURCES: usize = 200;

/// Signatures de types GWT connues d'ADE 6.x ; les autres sont lues dans les réponses.
const LOGIN_REQUEST: &str = "com.adesoft.gwt.core.client.rpc.data.LoginRequest/3705388826";
const DIRECT_LOGIN_REQUEST: &str =
    "com.adesoft.gwt.directplan.client.rpc.data.DirectLoginRequest/635437471";
const STRING_TYPE: &str = "java.lang.String/2004016611";
const ARRAY_LIST: &str = "java.util.ArrayList/4159755760";
const INTEGER_TYPE: &str = "java.lang.Integer/3438268394";
const FILTER_TYPE: &str = "com.adesoft.gwt.core.client.rpc.config.FilterType/1396315430";
const TREE_CONFIG: &str = "com.adesoft.gwt.directplan.client.ui.tree.TreeResourceConfig/2234901663";

/// Racine de l'arbre des ressources, telle que MyPlanning la demande.
const TREE_ROOT: &str = r#"{"-100""true""-1""-1""-1""-1""0""false"[0]"""""0""0"[0][0]"#;
/// Dossiers ouverts au plus pour lister les groupes (MyPlanning en ouvre 25 pour 14 ressources).
const MAX_FOLDERS: usize = 80;

fn token_key(base: &Url) -> String {
    match base.port() {
        Some(port) => format!(
            "ade-myplanning.{}:{port}",
            base.host_str().unwrap_or_default()
        ),
        None => format!("ade-myplanning.{}", base.host_str().unwrap_or_default()),
    }
}

fn root(base_url: &Url) -> Url {
    let mut base = base_url.clone();
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base.set_query(None);
    base.set_fragment(None);
    base
}

// ---------------------------------------------------------------------------------------------
// Récupération
// ---------------------------------------------------------------------------------------------

pub(crate) async fn fetch_timetable(
    ctx: &mut Context<'_>,
    base_url: &Url,
    project_id: u32,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Course>, Problem> {
    let tz = university_tz(ctx)?;
    let base = root(base_url);
    let key = token_key(&base);
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
        token_key(&base),
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
        let reply = rpc
            .call(
                net,
                "DirectPlanningServiceProxy",
                &rpc.children_body(&node)?,
            )
            .await?;
        for child in children(reply.text().ok_or_else(bad_tree)?)? {
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
    let policies = policies(net, &module).await?;
    let rpc = Rpc {
        module,
        policies,
        client: gwt_long(chrono::Utc::now().timestamp_millis()),
    };

    let login = rpc
        .call(net, "MyPlanningClientServiceProxy", &rpc.login_body()?)
        .await?;
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

    /// En-tête commun : version 7, drapeaux 0, puis la table de chaînes.
    fn body(table: &[&str], values: &[String]) -> String {
        let mut out = format!("7|0|{}|", table.len());
        for entry in table {
            out.push_str(&entry.replace('\\', "\\\\").replace('|', "\\!"));
            out.push('|');
        }
        for value in values {
            out.push_str(value);
            out.push('|');
        }
        out
    }

    fn login_body(&self) -> Result<String, Problem> {
        let service = "MyPlanningClientServiceProxy";
        let table = [
            self.module.as_str(),
            self.policy(service)?,
            "com.adesoft.gwt.directplan.client.rpc.MyPlanningClientServiceProxy",
            "method1login",
            "J",
            LOGIN_REQUEST,
            STRING_TYPE,
            DIRECT_LOGIN_REQUEST,
            "",
            "fr",
        ];
        let values: Vec<String> = [
            "1",
            "2",
            "3",
            "4",
            "3",
            "5",
            "6",
            "7",
            &self.client,
            "8",
            "0",
            "0",
            "0",
            "1",
            "1",
            "9",
            "9",
            "-1",
            "0",
            "0",
            "10",
        ]
        .iter()
        .map(|v| (*v).to_owned())
        .collect();
        Ok(Self::body(&table, &values))
    }

    fn load_project_body(&self, project_id: u32) -> Result<String, Problem> {
        let service = "WebClientServiceProxy";
        let table = [
            self.module.as_str(),
            self.policy(service)?,
            "com.adesoft.gwt.core.client.rpc.WebClientServiceProxy",
            "method6loadProject",
            "J",
            "I",
            "Z",
        ];
        let values: Vec<String> = ["1", "2", "3", "4", "3", "5", "6", "7", &self.client]
            .iter()
            .map(|v| (*v).to_owned())
            .chain([project_id.to_string(), "0".to_owned()])
            .collect();
        Ok(Self::body(&table, &values))
    }

    /// `getResourceIds(client, [], {critère → codes, …}, false)`. Les signatures des types
    /// (liste, table, critère) sont reprises de la réponse de connexion.
    fn resource_ids_body(
        &self,
        login: &GwtResponse,
        criteria: &[Criterion],
    ) -> Result<String, Problem> {
        let service = "DirectPlanningServiceProxy";
        let list = login.type_name("java.util.ArrayList/")?;
        let map = login.type_name("java.util.HashMap/")?;
        let criterion =
            login.type_name("com.adesoft.gwt.directplan.client.rpc.ResourceFieldCriteria/")?;
        let string = login.type_name("java.lang.String/")?;
        let mut table: Vec<&str> = vec![
            self.module.as_str(),
            self.policy(service)?,
            "com.adesoft.gwt.directplan.client.rpc.DirectPlanningServiceProxy",
            "method7getResourceIds",
            "J",
            "java.util.List",
            "java.util.Map",
            "Z",
            list,
            map,
            criterion,
            string,
        ];
        let mut values: Vec<String> = ["1", "2", "3", "4", "4", "5", "6", "7", "8", &self.client]
            .iter()
            .map(|v| (*v).to_owned())
            .collect();
        // Liste vide, puis la table des critères.
        values.extend(["9", "0", "10"].map(str::to_owned));
        values.push(criteria.len().to_string());
        for c in criteria {
            values.extend(["11".to_owned(), c.field.to_string(), "9".to_owned()]);
            values.push(c.codes.len().to_string());
            for code in &c.codes {
                table.push(code);
                values.extend(["12".to_owned(), table.len().to_string()]);
            }
        }
        values.push("0".to_owned());
        Ok(Self::body(&table, &values))
    }

    /// `searchDataById(client, FilterType#3, [numéros])` : dossiers qui mènent aux ressources.
    fn search_body(&self, ids: &[u32]) -> Result<String, Problem> {
        let service = "ConfigurationServiceProxy";
        let table = [
            self.module.as_str(),
            self.policy(service)?,
            "com.adesoft.gwt.core.client.rpc.ConfigurationServiceProxy",
            "method25searchDataById",
            "J",
            FILTER_TYPE,
            "java.util.List",
            ARRAY_LIST,
            INTEGER_TYPE,
        ];
        let mut values: Vec<String> = ["1", "2", "3", "4", "3", "5", "6", "7", &self.client]
            .iter()
            .map(|v| (*v).to_owned())
            .collect();
        values.extend(["6", "3", "8"].map(str::to_owned));
        values.push(ids.len().to_string());
        for id in ids {
            values.extend(["9".to_owned(), id.to_string()]);
        }
        Ok(Self::body(&table, &values))
    }

    /// `getChildren(client, nœud, configuration)` : enfants d'un dossier avec leur nom. Le nœud
    /// est renvoyé tel qu'ADE l'a écrit ; la configuration demande les champs couleur et nom.
    fn children_body(&self, node: &str) -> Result<String, Problem> {
        const CONFIG: &str = "8|7|0|9|2|-1|-1|10|0|2|6|11|12|0|13|11|14|15|11|0|0|6|16|12|0|17|16|14|15|4|0|0|18|0|18|0|19|20|1|16|18|0";
        let service = "DirectPlanningServiceProxy";
        let table = [
            self.module.as_str(),
            self.policy(service)?,
            "com.adesoft.gwt.directplan.client.rpc.DirectPlanningServiceProxy",
            "method4getChildren",
            "J",
            STRING_TYPE,
            TREE_CONFIG,
            node,
            "[I/2970817851",
            "java.util.LinkedHashMap/3008245022",
            "COLOR",
            "com.adesoft.gwt.core.client.rpc.config.OutputField/870745015",
            "LabelColor",
            "",
            "com.adesoft.gwt.core.client.rpc.config.FieldType/1797283245",
            "NAME",
            "LabelName",
            ARRAY_LIST,
            "com.extjs.gxt.ui.client.data.SortInfo/1143517771",
            "com.extjs.gxt.ui.client.Style$SortDir/3873584144",
        ];
        let values: Vec<String> = ["1", "2", "3", "4", "3", "5", "6", "7", &self.client]
            .into_iter()
            .chain(CONFIG.split('|'))
            .map(str::to_owned)
            .collect();
        Ok(Self::body(&table, &values))
    }
}

// ---------------------------------------------------------------------------------------------
// GWT-RPC (sans réseau)
// ---------------------------------------------------------------------------------------------

/// Réponse `//OK[entiers…,[chaînes…],drapeaux,version]`.
#[derive(Debug)]
struct GwtResponse {
    /// Flux d'entiers dans l'ordre de lecture (la réponse est écrite à l'envers).
    ints: Vec<i64>,
    strings: Vec<String>,
}

impl GwtResponse {
    fn parse(text: &str) -> Option<Self> {
        let body = text.strip_prefix("//OK")?;
        let value: serde_json::Value = serde_json::from_str(&json_escapes(body)).ok()?;
        let items = value.as_array()?;
        // Les deux derniers éléments sont les drapeaux et la version ; la table de chaînes précède.
        let table_at = items.len().checked_sub(3)?;
        let strings = items
            .get(table_at)?
            .as_array()?
            .iter()
            .map(|s| s.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()?;
        let mut ints = items
            .get(..table_at)?
            .iter()
            .map(serde_json::Value::as_i64)
            .collect::<Option<Vec<_>>>()?;
        ints.reverse();
        Some(GwtResponse { ints, strings })
    }

    /// Position (à partir de 1) de la chaîne qui commence par `prefix`.
    fn index(&self, prefix: &str) -> Option<i64> {
        self.strings
            .iter()
            .position(|s| s.starts_with(prefix))
            .and_then(|i| i64::try_from(i + 1).ok())
    }

    fn type_name(&self, prefix: &str) -> Result<&str, Problem> {
        self.strings
            .iter()
            .find(|s| s.starts_with(prefix))
            .map(String::as_str)
            .ok_or_else(|| {
                upstream(
                    PROVIDER,
                    "type GWT attendu absent de la réponse de connexion",
                )
            })
    }

    fn string(&self, index: i64) -> Option<&str> {
        let i = usize::try_from(index).ok()?.checked_sub(1)?;
        self.strings.get(i).map(String::as_str)
    }

    /// Chaîne renvoyée seule (`//OK[1,["…"],0,7]`).
    fn text(&self) -> Option<&str> {
        self.string(*self.ints.first()?)
    }
}

/// GWT écrit certains caractères en `\xHH` (apostrophe, esperluette), que JSON refuse.
fn json_escapes(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('x') => out.push_str("\\u00"),
            Some(next) => {
                out.push('\\');
                out.push(next);
            }
            None => out.push('\\'),
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Arbre des ressources (format texte d'ADE dans une chaîne GWT)
// ---------------------------------------------------------------------------------------------

/// Élément du format d'arbre : `{` ouvre un nœud, `[n]` annonce n éléments, `"…"` une valeur.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Token<'a> {
    Open,
    Count(usize),
    Text(&'a str),
}

/// Découpe du format ; chaque élément garde sa position de début et de fin.
fn tokens(text: &str) -> Option<Vec<(Token<'_>, usize, usize)>> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&b) = text.as_bytes().get(i) {
        let (token, end) = match b {
            b'{' => (Token::Open, i + 1),
            b'[' => {
                let close = i + 1 + text.get(i + 1..)?.find(']')?;
                (
                    Token::Count(text.get(i + 1..close)?.parse().ok()?),
                    close + 1,
                )
            }
            b'"' => {
                let close = i + 1 + text.get(i + 1..)?.find('"')?;
                (Token::Text(text.get(i + 1..close)?), close + 1)
            }
            _ => return None,
        };
        out.push((token, i, end));
        i = end;
    }
    Some(out)
}

/// Dossiers de la réponse de `searchDataById` : tous les nœuds marqués `"true"`.
fn path_folders(text: &str) -> Result<HashSet<i64>, Problem> {
    let tokens = tokens(text).ok_or_else(bad_tree)?;
    let mut out = HashSet::new();
    for window in tokens.windows(3) {
        if let [
            (Token::Open, ..),
            (Token::Text(id), ..),
            (Token::Text("true"), ..),
        ] = window
        {
            out.insert(id.parse().map_err(|_| bad_tree())?);
        }
    }
    if out.is_empty() {
        return Err(bad_tree());
    }
    Ok(out)
}

/// Enfant d'un dossier : numéro, dossier ou ressource, nom et texte du nœud.
#[derive(Debug, PartialEq)]
struct TreeNode {
    id: i64,
    folder: bool,
    name: String,
    raw: String,
}

/// Lecteur des nœuds de `getChildren` :
/// `{` 8 valeurs, `[n]` n champs (`{` 6 valeurs), 4 valeurs, `[0][0]`.
struct Reader<'a> {
    text: &'a str,
    tokens: Vec<(Token<'a>, usize, usize)>,
    at: usize,
}

impl<'a> Reader<'a> {
    fn next(&mut self) -> Option<(Token<'a>, usize, usize)> {
        let token = *self.tokens.get(self.at)?;
        self.at += 1;
        Some(token)
    }

    fn open(&mut self) -> Option<usize> {
        match self.next()? {
            (Token::Open, start, _) => Some(start),
            _ => None,
        }
    }

    fn count(&mut self) -> Option<(usize, usize)> {
        match self.next()? {
            (Token::Count(n), _, end) => Some((n, end)),
            _ => None,
        }
    }

    fn texts(&mut self, n: usize) -> Option<Vec<&'a str>> {
        (0..n)
            .map(|_| match self.next()? {
                (Token::Text(t), ..) => Some(t),
                _ => None,
            })
            .collect()
    }

    fn node(&mut self) -> Option<TreeNode> {
        let start = self.open()?;
        let head = self.texts(8)?;
        let mut name = None;
        for _ in 0..self.count()?.0 {
            self.open()?;
            let field = self.texts(6)?;
            if field.get(1) == Some(&"NAME") {
                name = field.get(3).map(|s| (*s).to_owned());
            }
        }
        self.texts(4)?;
        // Enfants déjà chargés : MyPlanning n'en renvoie pas, on refuse plutôt que deviner.
        let (first, _) = self.count()?;
        let (second, end) = self.count()?;
        if first != 0 || second != 0 {
            return None;
        }
        Some(TreeNode {
            id: head.first()?.parse().ok()?,
            folder: *head.get(1)? == "true",
            name: name.unwrap_or_default(),
            raw: self.text.get(start..end)?.to_owned(),
        })
    }
}

/// Réponse de `getChildren` : `{"0"`, le dossier ouvert, `[n]`, ses n enfants, puis des valeurs
/// finales (`"false"` à Paul-Valéry [vu] 27/10/2026, sens [inconnu]) qui sont ignorées.
fn children(text: &str) -> Result<Vec<TreeNode>, Problem> {
    let parse = || {
        let mut reader = Reader {
            text,
            tokens: tokens(text)?,
            at: 0,
        };
        reader.open()?;
        reader.texts(1)?;
        reader.node()?;
        let (n, _) = reader.count()?;
        let nodes = (0..n).map(|_| reader.node()).collect::<Option<Vec<_>>>()?;
        let rest = reader.tokens.get(reader.at..)?;
        rest.iter()
            .all(|(token, ..)| matches!(token, Token::Text(_)))
            .then_some(nodes)
    };
    parse().ok_or_else(bad_tree)
}

/// Un critère de MyPlanning : champ ADE et valeurs (codes d'enseignement).
#[derive(Debug, PartialEq)]
struct Criterion {
    field: i64,
    codes: Vec<String>,
}

/// Critères de la réponse de connexion : chaque `ResourceFieldCriteria(champ)` suivi d'une
/// `ArrayList` de chaînes.
fn login_criteria(login: &GwtResponse) -> Result<Vec<Criterion>, Problem> {
    let missing = || {
        upstream(
            PROVIDER,
            "critères MyPlanning absents de la réponse de connexion",
        )
    };
    let criterion = login
        .index("com.adesoft.gwt.directplan.client.rpc.ResourceFieldCriteria/")
        .ok_or_else(missing)?;
    let list = login.index("java.util.ArrayList/").ok_or_else(missing)?;
    let string = login.index("java.lang.String/").ok_or_else(missing)?;
    let ints = &login.ints;
    let mut out = Vec::new();
    for i in 0..ints.len() {
        let window = ints.get(i..i + 4);
        let Some(&[t, field, l, n]) = window else {
            break;
        };
        if t != criterion || l != list {
            continue;
        }
        let Ok(n) = usize::try_from(n) else { continue };
        let mut codes = Vec::with_capacity(n);
        for k in 0..n {
            let at = i + 4 + 2 * k;
            match (ints.get(at), ints.get(at + 1)) {
                (Some(&s), Some(&idx)) if s == string => {
                    if let Some(code) = login.string(idx) {
                        codes.push(code.to_owned());
                    }
                }
                _ => break,
            }
        }
        if !codes.is_empty() {
            out.push(Criterion { field, codes });
        }
    }
    if out.is_empty() {
        return Err(missing());
    }
    Ok(out)
}

/// `ArrayList<Integer>` d'une réponse : taille puis paires (type `Integer`, valeur).
fn integer_list(reply: &GwtResponse) -> Result<Vec<u32>, Problem> {
    let bad = || upstream(PROVIDER, "liste des ressources ADE illisible");
    let list = reply.index("java.util.ArrayList/").ok_or_else(bad)?;
    let integer = reply.index("java.lang.Integer/").ok_or_else(bad)?;
    let ints = &reply.ints;
    let start = ints.iter().position(|&v| v == list).ok_or_else(bad)?;
    let n = ints.get(start + 1).copied().ok_or_else(bad)?;
    let n = usize::try_from(n).map_err(|_| bad())?;
    let mut ids = Vec::with_capacity(n);
    for k in 0..n {
        let at = start + 2 + 2 * k;
        match (ints.get(at), ints.get(at + 1)) {
            (Some(&t), Some(&v)) if t == integer => {
                ids.push(u32::try_from(v).map_err(|_| bad())?);
            }
            _ => return Err(bad()),
        }
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// Noms de permutation (32 chiffres hexadécimaux majuscules) cités par le chargeur GWT.
fn permutations(script: &str) -> Vec<String> {
    let bytes = script.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i + 32 <= bytes.len() {
        let candidate = &bytes[i..i + 32];
        let boundary_before = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let boundary_after = bytes.get(i + 32).is_none_or(|b| !b.is_ascii_alphanumeric());
        if boundary_before
            && boundary_after
            && candidate
                .iter()
                .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(b))
        {
            let name = String::from_utf8_lossy(candidate).into_owned();
            if !out.contains(&name) {
                out.push(name);
            }
            i += 32;
        } else {
            i += 1;
        }
    }
    out
}

/// Couples `'…ServiceProxy','<signature>'` du code d'une permutation.
fn service_policies(code: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (at, _) in code.match_indices("ServiceProxy','") {
        let Some(before) = code.get(..at) else {
            continue;
        };
        let Some(start) = before.rfind('\'') else {
            continue;
        };
        let name = format!("{}ServiceProxy", &before[start + 1..]);
        let Some(hash) = code.get(at + "ServiceProxy','".len()..at + "ServiceProxy','".len() + 32)
        else {
            continue;
        };
        if name.chars().all(|c| c.is_ascii_alphanumeric())
            && hash
                .chars()
                .all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c))
            && !out.iter().any(|(n, _)| n == &name)
        {
            out.push((name, hash.to_owned()));
        }
    }
    out
}

/// Entier long au format GWT (base 64, alphabet `A–Z a–z 0–9 $ _`).
fn gwt_long(value: i64) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789$_";
    let mut v = u64::try_from(value).unwrap_or(0);
    if v == 0 {
        return "A".to_owned();
    }
    let mut out = Vec::new();
    while v > 0 {
        out.push(ALPHABET[usize::try_from(v & 63).unwrap_or(0)]);
        v >>= 6;
    }
    out.reverse();
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Réponse de connexion synthétique : un critère (champ 1) de deux codes inventés.
    const LOGIN: &str = r#"//OK[0,1,11,9,10,9,2,2,1,8,1,7,6,5,0,2,4,3,1,2,1,["com.adesoft.gwt.directplan.client.rpc.data.MyPlanningLoginReponse/1769343849","java.util.ArrayList/4159755760","com.adesoft.gwt.directplan.client.rpc.data.ProjectParameter/3599371664","com.adesoft.gwt.directplan.client.UrlParameter/970611799","web","","java.util.HashMap/1797211028","com.adesoft.gwt.directplan.client.rpc.ResourceFieldCriteria/1324434193","java.lang.String/2004016611","S1-DEMO-001","S1-DEMO-002"],0,7]"#;

    #[test]
    fn criteres_lus_dans_la_reponse_de_connexion() {
        let login = GwtResponse::parse(LOGIN).unwrap();
        let criteria = login_criteria(&login).unwrap();
        assert_eq!(
            criteria,
            [Criterion {
                field: 1,
                codes: vec!["S1-DEMO-001".to_owned(), "S1-DEMO-002".to_owned()]
            }]
        );
    }

    #[test]
    fn corps_de_get_resource_ids() {
        let login = GwtResponse::parse(LOGIN).unwrap();
        let rpc = Rpc {
            module: Url::parse("https://ade.example.test/direct/gwtdirectplanning/").unwrap(),
            policies: vec![(
                "DirectPlanningServiceProxy".to_owned(),
                "0123456789ABCDEF0123456789ABCDEF".to_owned(),
            )],
            client: "aF_MmK8".to_owned(),
        };
        let body = rpc
            .resource_ids_body(&login, &login_criteria(&login).unwrap())
            .unwrap();
        assert_eq!(
            body,
            "7|0|14|https://ade.example.test/direct/gwtdirectplanning/|0123456789ABCDEF0123456789ABCDEF|com.adesoft.gwt.directplan.client.rpc.DirectPlanningServiceProxy|method7getResourceIds|J|java.util.List|java.util.Map|Z|java.util.ArrayList/4159755760|java.util.HashMap/1797211028|com.adesoft.gwt.directplan.client.rpc.ResourceFieldCriteria/1324434193|java.lang.String/2004016611|S1-DEMO-001|S1-DEMO-002|1|2|3|4|4|5|6|7|8|aF_MmK8|9|0|10|1|11|1|9|2|12|13|12|14|0|"
        );
        assert!(
            rpc.login_body().is_err(),
            "signature absente : erreur, pas de corps"
        );
    }

    #[test]
    fn numeros_de_ressources() {
        let reply = GwtResponse::parse(
            r#"//OK[1250,2,11870,2,1846,2,3,1,["java.util.ArrayList/4159755760","java.lang.Integer/3438268394"],0,7]"#,
        )
        .unwrap();
        assert_eq!(integer_list(&reply).unwrap(), [1250, 1846, 11870]);
        assert!(GwtResponse::parse("//EX[2,[\"x\"],0,7]").is_none());
        let bad = GwtResponse::parse(
            r#"//OK[5,1,["java.util.ArrayList/4159755760","java.lang.Integer/3438268394"],0,7]"#,
        )
        .unwrap();
        assert!(integer_list(&bad).is_err());
    }

    #[test]
    fn signatures_et_permutations() {
        let code = "function kbc(){Yx.call(this,EEd(),'DirectPlanningServiceProxy','AF8E80FD952FAE1F1E1D64FCA0763541',Xac)} function lG(){Yx.call(this,EEd(),'WebClientServiceProxy','5EB99F51B592E97DD3B5315B286394D6',SF)}";
        assert_eq!(
            service_policies(code),
            [
                (
                    "DirectPlanningServiceProxy".to_owned(),
                    "AF8E80FD952FAE1F1E1D64FCA0763541".to_owned()
                ),
                (
                    "WebClientServiceProxy".to_owned(),
                    "5EB99F51B592E97DD3B5315B286394D6".to_owned()
                ),
            ]
        );
        let script = "var a='1987F1EF14DA1BA12007E141147356DC',b='C4EA130FD0ED44BE513FEEDDE13614DA.cache.png',c='x0123456789ABCDEF0123456789ABCDEF';";
        assert_eq!(
            permutations(script),
            [
                "1987F1EF14DA1BA12007E141147356DC",
                "C4EA130FD0ED44BE513FEEDDE13614DA"
            ]
        );
    }

    /// Arbre inventé : catégorie -1, dossier 10, ressources 41 et 42.
    const SEARCH: &str = r#"{"-100""true""0""-1""-1""-1""0""false"[0]""[0][1]{"-1""true""0""-1""-1""-1""0""false"[0]""[0][1]{"10""true""0""-1""-1""-1""0""false"[0]""[2]{"41""false""0""-1""-1""-1""0""false"[0]""[0]{"42""false""0""-1""-1""-1""0""false"[0]""[0][0]"#;
    const CHILDREN: &str = r#"{"0"{"10""true""1""-1""0""0""0""false"[1]{"StringField""NAME""LabelName""L1 Démo""false""false""L1 Démo""trainee""1""0"[0][0][2]{"41""false""2""-1""0""0""0""false"[2]{"ColorField""COLOR""LabelColor""255,0,0""false""false"{"StringField""NAME""LabelName""Groupe A""false""false""L1 Démo.Groupe A""trainee""1""0"[0][0]{"42""false""2""-1""1""1""0""false"[2]{"ColorField""COLOR""LabelColor""0,0,255""false""false"{"StringField""NAME""LabelName""Groupe B""false""false""L1 Démo.Groupe B""trainee""1""0"[0][0]"false""#;

    #[test]
    fn dossiers_du_chemin() {
        let folders = path_folders(SEARCH).unwrap();
        assert_eq!(folders, HashSet::from([-100, -1, 10]));
        assert!(path_folders("{\"1\"").is_err());
    }

    #[test]
    fn enfants_d_un_dossier() {
        let nodes = children(CHILDREN).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!((nodes[0].id, nodes[0].folder), (41, false));
        assert_eq!(nodes[1].name, "Groupe B");
        // Le nœud est recopié tel quel pour être rouvert.
        assert!(nodes[0].raw.starts_with(r#"{"41""false""2""#));
        assert!(nodes[0].raw.ends_with(r#""trainee""1""0"[0][0]"#));
        assert!(CHILDREN.contains(&nodes[1].raw));
        // Racine demandée par MyPlanning : un nœud sans champ.
        let root = format!(r#"{{"0"{TREE_ROOT}[0]"false""#);
        assert_eq!(children(&root).unwrap(), []);
        // Après les enfants, seules des valeurs sont admises.
        assert!(children(&format!(r#"{{"0"{TREE_ROOT}[0][0]"#)).is_err());
        assert!(children(&CHILDREN[..CHILDREN.len() - 3]).is_err());
    }

    #[test]
    fn echappements_gwt() {
        let reply =
            GwtResponse::parse(r#"//OK[1,["Groupe d\x27\xE9tudiants \x26 co"],0,7]"#).unwrap();
        assert_eq!(reply.text(), Some("Groupe d'étudiants & co"));
    }

    #[test]
    fn corps_de_get_children() {
        let rpc = Rpc {
            module: Url::parse("https://ade.example.test/direct/gwtdirectplanning/").unwrap(),
            policies: vec![(
                "DirectPlanningServiceProxy".to_owned(),
                "0123456789ABCDEF0123456789ABCDEF".to_owned(),
            )],
            client: "aF_MmK8".to_owned(),
        };
        let body = rpc.children_body(TREE_ROOT).unwrap();
        let parts: Vec<&str> = body.split('|').collect();
        assert_eq!(parts[2], "20");
        assert_eq!(parts[10], TREE_ROOT);
        assert!(body.ends_with("|1|2|3|4|3|5|6|7|aF_MmK8|8|7|0|9|2|-1|-1|10|0|2|6|11|12|0|13|11|14|15|11|0|0|6|16|12|0|17|16|14|15|4|0|0|18|0|18|0|19|20|1|16|18|0|"));
        assert!(rpc.search_body(&[1]).is_err(), "signature absente");
    }

    #[test]
    fn entier_long_gwt() {
        assert_eq!(gwt_long(0), "A");
        assert_eq!(gwt_long(63), "_");
        assert_eq!(gwt_long(64), "BA");
    }
}
