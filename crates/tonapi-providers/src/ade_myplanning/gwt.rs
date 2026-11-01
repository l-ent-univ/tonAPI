//! GWT-RPC de MyPlanning, sans réseau : corps des appels (`7|0|…`), réponses `//OK[…]`, critères
//! de connexion, numéros de ressources et signatures lues dans le JavaScript public.

use tonapi_core::Problem;

use super::{PROVIDER, Rpc};
use crate::net::upstream;

/// Signatures de types GWT connues d'ADE 6.x ; les autres sont lues dans les réponses.
const LOGIN_REQUEST: &str = "com.adesoft.gwt.core.client.rpc.data.LoginRequest/3705388826";
const DIRECT_LOGIN_REQUEST: &str =
    "com.adesoft.gwt.directplan.client.rpc.data.DirectLoginRequest/635437471";
const STRING_TYPE: &str = "java.lang.String/2004016611";
const ARRAY_LIST: &str = "java.util.ArrayList/4159755760";
const INTEGER_TYPE: &str = "java.lang.Integer/3438268394";
const FILTER_TYPE: &str = "com.adesoft.gwt.core.client.rpc.config.FilterType/1396315430";
const TREE_CONFIG: &str = "com.adesoft.gwt.directplan.client.ui.tree.TreeResourceConfig/2234901663";

impl Rpc {
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

    pub(super) fn login_body(&self) -> Result<String, Problem> {
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

    pub(super) fn load_project_body(&self, project_id: u32) -> Result<String, Problem> {
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
    pub(super) fn resource_ids_body(
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
    pub(super) fn search_body(&self, ids: &[u32]) -> Result<String, Problem> {
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
    pub(super) fn children_body(&self, node: &str) -> Result<String, Problem> {
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

/// Réponse `//OK[entiers…,[chaînes…],drapeaux,version]`.
#[derive(Debug)]
pub(super) struct GwtResponse {
    /// Flux d'entiers dans l'ordre de lecture (la réponse est écrite à l'envers).
    ints: Vec<i64>,
    strings: Vec<String>,
}

impl GwtResponse {
    pub(super) fn parse(text: &str) -> Option<Self> {
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
    pub(super) fn text(&self) -> Option<&str> {
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

/// Un critère de MyPlanning : champ ADE et valeurs (codes d'enseignement).
#[derive(Debug, PartialEq)]
pub(super) struct Criterion {
    field: i64,
    codes: Vec<String>,
}

/// Critères de la réponse de connexion : chaque `ResourceFieldCriteria(champ)` suivi d'une
/// `ArrayList` de chaînes.
pub(super) fn login_criteria(login: &GwtResponse) -> Result<Vec<Criterion>, Problem> {
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
        // Taille annoncée par ADE : jamais au-delà de ce que la réponse contient.
        let mut codes = Vec::with_capacity(n.min(ints.len() / 2));
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
pub(super) fn integer_list(reply: &GwtResponse) -> Result<Vec<u32>, Problem> {
    let bad = || upstream(PROVIDER, "liste des ressources ADE illisible");
    let list = reply.index("java.util.ArrayList/").ok_or_else(bad)?;
    let integer = reply.index("java.lang.Integer/").ok_or_else(bad)?;
    let ints = &reply.ints;
    let start = ints.iter().position(|&v| v == list).ok_or_else(bad)?;
    let n = ints.get(start + 1).copied().ok_or_else(bad)?;
    let n = usize::try_from(n).map_err(|_| bad())?;
    // Taille annoncée par ADE : refusée si la réponse ne peut pas la contenir (une allocation
    // impossible arrêterait tout le processus).
    if n > ints.len() / 2 {
        return Err(bad());
    }
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
pub(super) fn permutations(script: &str) -> Vec<String> {
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
                // Seules les 16 premières sont essayées.
                if out.len() == 16 {
                    break;
                }
            }
            i += 32;
        } else {
            i += 1;
        }
    }
    out
}

/// Couples `'…ServiceProxy','<signature>'` du code d'une permutation.
pub(super) fn service_policies(code: &str) -> Vec<(String, String)> {
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
pub(super) fn gwt_long(value: i64) -> String {
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
    use crate::ade_myplanning::tree::TREE_ROOT;
    use url::Url;

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
