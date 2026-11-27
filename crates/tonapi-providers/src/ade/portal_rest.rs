//! Emploi du temps ADE, API REST du portail (`<portail>/RestApi/`), derrière le CAS de l'université.
//!
//! Parcours, d'après `docs/recherche/rennes/apis/ade-portal-rest.md` :
//!
//! 1. `POST sso/accessToken` donne un jeton SSO court ; s'il est vide (HTTP 200 + `errMsg`), on
//!    traverse le CAS avec le TGC de la session ([`Net::open_protected`]) puis on réessaie ;
//! 2. `POST RestApi/portal/ssologin` (Bearer = jeton SSO) donne le jeton d'API (JWT, 30 min), le
//!    jeton de rafraîchissement et le numéro de projet ;
//! 3. les données se lisent sous `RestApi/portal/projects/<projet>/` avec le jeton d'API. Sur 401 (ou
//!    redirection), le jeton est renouvelé (`refreshtoken`, sinon nouvelle ouverture de session) et
//!    la requête n'est rejouée **qu'une fois**.
//!
//! Les ressources à interroger sont celles que le client demande ; sans demande, la sélection
//! personnelle de l'étudiant (`PortalResourcesAllSelection`), sinon celles qu'ADE lui attribue dans
//! la réponse de `ssologin`. Toutes partent dans une seule requête `resourcesEventsPortal`.
//! Les jetons ADE restent dans `session.tokens["ade-rest.<hôte>"]`, jamais le mot de passe.
//!
//! `resourcestree` rend d'un coup l'arbre de toute l'université (étudiants, enseignants, salles) :
//! l'arbre, la liste des ressources, leur recherche et les groupes en sont tous tirés.
//!
//! L'arbre lu est gardé une demi-heure en mémoire ([`TREES`]), par portail, projet et **profil
//! ADE** du compte (`authData.profile`, `etudiant` pour un étudiant). ADE donne ses droits de
//! lecture par profil, et un compte étudiant d'une composante voit les ressources de toutes les
//! autres ([vu] Rennes, 27/11/2026) : deux comptes du même profil lisent le même arbre [déduit].
//! Un compte dont le profil est inconnu ne lit ni n'écrit ce cache. L'arbre ne contient que des
//! noms de groupes, d'enseignants et de salles ; l'emploi du temps d'une ressource reste demandé
//! à ADE avec le jeton du compte, qui le refuse à qui n'y a pas droit.
//!
//! Formes vues sur le portail de l'Université de Rennes le 27/11/2026, sans valeur recopiée :
//! `docs/recherche/rennes/apis/ade-portal-rest.md`, section 11. Reste `[inconnu]` : le format d'une
//! sélection personnelle non vide et le type de séance (CM, TD, TP).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::NaiveDate;
use chrono_tz::Tz;
use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tonapi_config::Signal;
use tonapi_core::{
    Course, ErrorCode, Problem, ResourceKind, ResourceNode, TimetableGroup, TimetableResource,
};
use url::Url;

use crate::cache::Shared;
use crate::common::token_key;
use crate::net::{Net, upstream};
use crate::{Context, ResourceQuery, resource_tree};

mod parse;

use parse::{
    events_filter, group_signals, parse_events, parse_refresh, parse_requested, parse_selection,
    parse_sso_token, parse_ssologin, parse_tree,
};

/// Nom du fournisseur dans les messages d'erreur.
const PROVIDER: &str = "ade-portal-rest";
/// Nombre maximal de ressources interrogées pour une même plage.
const MAX_RESOURCES: usize = 30;

/// Arbres des ressources déjà lus, par portail, projet et profil ADE : les mêmes pour tous les
/// comptes d'un profil. Une demi-heure : l'arbre change quand la scolarité crée un groupe ou une
/// salle, pas dans la journée d'un étudiant.
static TREES: Shared<Arc<Vec<ResourceNode>>> = Shared::new(Duration::from_secs(30 * 60));

/// Jetons ADE gardés dans la session de l'étudiant (`ade-rest.<hôte>`).
#[derive(Clone, Serialize, Deserialize)]
struct Tokens {
    /// Jeton d'API (JWT) pour les appels de données.
    access: String,
    /// Jeton de rafraîchissement (UUID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh: Option<String>,
    /// Projet ADE de l'année (`authData.projectId`).
    project: i64,
    /// Ressources qu'ADE attribue à l'étudiant (`authData.resources`). Absent des sessions émises
    /// avant que ce champ existe : la session ADE est alors rouverte pour le lire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resources: Option<Vec<i64>>,
    /// Profil ADE du compte (`authData.profile`), vide si ADE n'en donne pas. Absent des sessions
    /// émises avant que ce champ existe : la session ADE est alors rouverte pour le lire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
}

/// Réponse d'ADE : des données, une session perdue (401 ou redirection, p. ex. vers le CAS), ou
/// un refus (403).
enum Reply {
    Data(Value),
    Lost,
    Denied,
}

struct Client {
    net: Net,
    /// Racine du portail, avec une barre finale.
    base: Url,
    timezone: String,
    tokens: Option<Tokens>,
    /// Les jetons viennent d'être obtenus : un 401 ne justifie pas de les renouveler.
    fresh: bool,
    key: String,
}

impl Client {
    fn open(ctx: &Context<'_>, base_url: &Url) -> Result<(Self, Tz), Problem> {
        let timezone = ctx.university.university.timezone.clone();
        let tz: Tz = timezone.parse().map_err(|_| {
            Problem::new(ErrorCode::Internal).detail(format!(
                "fuseau horaire `{timezone}` inconnu dans la configuration"
            ))
        })?;
        let mut base = base_url.clone();
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        let key = token_key("ade-rest", &base);
        let tokens = ctx
            .session
            .tokens
            .get(&key)
            .and_then(|raw| serde_json::from_str::<Tokens>(raw).ok());
        let net = Net::open(ctx, PROVIDER)?;
        Ok((
            Client {
                net,
                base,
                timezone,
                tokens,
                fresh: false,
                key,
            },
            tz,
        ))
    }

    /// Remet jetons et cookies dans la session de l'étudiant.
    fn finish(self, ctx: &mut Context<'_>) {
        match self.tokens.and_then(|t| serde_json::to_string(&t).ok()) {
            Some(raw) => {
                ctx.session.tokens.insert(self.key, raw);
            }
            None => {
                ctx.session.tokens.remove(&self.key);
            }
        }
        self.net.save(ctx);
    }

    fn url(&self, path: &str) -> Result<Url, Problem> {
        self.base
            .join(path)
            .map_err(|_| upstream(PROVIDER, "adresse du portail inutilisable"))
    }

    fn headers(
        &self,
        bearer: Option<&str>,
        body: bool,
        timezone: bool,
    ) -> Result<HeaderMap, Problem> {
        let invalid = || upstream(PROVIDER, "en-tête de requête inutilisable");
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(
            header::REFERER,
            HeaderValue::from_str(self.base.as_str()).map_err(|_| invalid())?,
        );
        if body {
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
        }
        if timezone {
            headers.insert(
                HeaderName::from_static("clienttimezone"),
                HeaderValue::from_str(&self.timezone).map_err(|_| invalid())?,
            );
        }
        if let Some(token) = bearer {
            let mut value =
                HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| invalid())?;
            value.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, value);
        }
        Ok(headers)
    }

    /// Une requête. 401 et redirections donnent [`Reply::Lost`], 403 [`Reply::Denied`] ; les autres
    /// statuts d'erreur et les corps non JSON donnent une erreur qui ne cite ni corps ni jeton.
    async fn send(
        &mut self,
        method: Method,
        url: Url,
        bearer: Option<&str>,
        body: Option<&str>,
        timezone: bool,
    ) -> Result<Reply, Problem> {
        let headers = self.headers(bearer, body.is_some(), timezone)?;
        let (status, _, text) = self
            .net
            .request(method, url, headers, body.map(str::to_owned))
            .await?;
        if status == reqwest::StatusCode::UNAUTHORIZED || status.is_redirection() {
            return Ok(Reply::Lost);
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return Ok(Reply::Denied);
        }
        if !status.is_success() {
            return Err(Problem::new(ErrorCode::ServiceUnavailable)
                .detail(format!("{PROVIDER} : réponse HTTP {status}")));
        }
        serde_json::from_str(&text)
            .map(Reply::Data)
            .map_err(|_| upstream(PROVIDER, format!("réponse non JSON (HTTP {status})")))
    }

    /// Jeton SSO, ou `None` si ADE n'a pas de session pour ce navigateur.
    async fn sso_token(&mut self) -> Result<Option<String>, Problem> {
        let url = self.url("sso/accessToken")?;
        match self
            .send(Method::POST, url, None, Some("{}"), false)
            .await?
        {
            Reply::Data(value) => Ok(parse_sso_token(&value)),
            Reply::Lost | Reply::Denied => Ok(None),
        }
    }

    /// Ouvre la session ADE : jeton SSO (après le CAS si besoin), puis `ssologin`.
    async fn login(&mut self) -> Result<(), Problem> {
        let mut sso = self.sso_token().await?;
        if sso.is_none() {
            // Pas de session sur le portail : la chaîne CAS silencieuse (TGC) la crée.
            self.net.open_protected(self.base.clone()).await?;
            sso = self.sso_token().await?;
        }
        let Some(sso) = sso else {
            return Err(upstream(PROVIDER, "jeton SSO vide après le CAS"));
        };
        let url = self.url("RestApi/portal/ssologin")?;
        match self
            .send(Method::POST, url, Some(&sso), Some("{}"), false)
            .await?
        {
            Reply::Data(value) => {
                self.tokens = Some(parse_ssologin(&value)?);
                self.fresh = true;
                Ok(())
            }
            Reply::Lost | Reply::Denied => Err(Problem::new(ErrorCode::SessionExpired)
                .detail("ADE a refusé le jeton SSO : se reconnecter")),
        }
    }

    /// Renouvelle le jeton d'API : `refreshtoken`, sinon nouvelle ouverture de session.
    async fn renew(&mut self) -> Result<(), Problem> {
        if let Some(tokens) = self.tokens.clone()
            && let Some(refresh) = tokens.refresh.as_deref()
        {
            let mut url = self.url("RestApi/portal/refreshtoken")?;
            url.query_pairs_mut().append_pair("refresh_token", refresh);
            // Sans `Authorization`, comme le front.
            if let Ok(Reply::Data(value)) = self.send(Method::GET, url, None, None, true).await
                && let Some((access, new_refresh)) = parse_refresh(&value)
            {
                self.tokens = Some(Tokens {
                    access,
                    refresh: new_refresh.or(tokens.refresh),
                    project: tokens.project,
                    resources: tokens.resources,
                    profile: tokens.profile,
                });
                self.fresh = true;
                return Ok(());
            }
        }
        self.login().await
    }

    /// `GET` d'une ressource du projet, avec le contrat « un seul nouvel essai sur 401 ».
    ///
    /// Un refus (403) n'est pas une session perdue : le jeton est bon, c'est ce qu'on demande
    /// qu'ADE ne donne pas à ce compte. [vu] Rennes, 27/11/2026 : `resourcesEventsPortal` répond
    /// 403 (`error.code` 1805) pour une ressource inconnue. La réponse est `not_found`.
    async fn get_project(&mut self, path: &str) -> Result<Value, Problem> {
        if self.tokens.is_none() {
            self.login().await?;
        }
        let expired = || {
            Problem::new(ErrorCode::SessionExpired).detail("ADE refuse le jeton : se reconnecter")
        };
        match self.get_once(path).await? {
            Reply::Data(value) => return Ok(value),
            Reply::Denied => return Err(denied()),
            Reply::Lost if self.fresh => {
                self.tokens = None;
                return Err(expired());
            }
            Reply::Lost => {}
        }
        self.renew().await?;
        match self.get_once(path).await? {
            Reply::Data(value) => Ok(value),
            Reply::Denied => Err(denied()),
            Reply::Lost => {
                self.tokens = None;
                Err(expired())
            }
        }
    }

    /// Un `GET` sous le projet, avec le jeton d'API du moment.
    async fn get_once(&mut self, path: &str) -> Result<Reply, Problem> {
        let project = self.tokens.as_ref().map_or(0, |t| t.project);
        let access = self.tokens.as_ref().map(|t| t.access.clone());
        let url = self.url(&format!("RestApi/portal/projects/{project}/{path}"))?;
        let reply = self
            .send(Method::GET, url, access.as_deref(), None, true)
            .await?;
        if matches!(reply, Reply::Data(_)) {
            self.fresh = false;
        }
        Ok(reply)
    }

    /// Clé de l'arbre de ce compte dans [`TREES`] ; `None` si son profil ADE n'est pas connu.
    fn tree_key(&self) -> Option<String> {
        let tokens = self.tokens.as_ref()?;
        let profile = tokens.profile.as_deref().filter(|p| !p.is_empty())?;
        Some(format!("{}|{}|{profile}", self.base, tokens.project))
    }

    /// L'arbre des ressources de l'université, tel que le profil de ce compte le voit : celui
    /// qu'un compte du même profil a lu depuis moins d'une demi-heure, sinon relu dans ADE.
    async fn tree(&mut self) -> Result<Arc<Vec<ResourceNode>>, Problem> {
        // Le profil vient de `ssologin` : sans session ADE, ou avec une session émise avant que
        // le profil y soit gardé, on l'ouvre pour le connaître.
        if self.tokens.as_ref().is_none_or(|t| t.profile.is_none()) {
            self.login().await?;
        }
        let key = self.tree_key();
        if let Some(tree) = key.as_deref().and_then(|key| TREES.get(key)) {
            return Ok(tree);
        }
        let tree = Arc::new(parse_tree(&self.get_project("resourcestree").await?)?);
        if let Some(key) = key {
            TREES.put(key, Arc::clone(&tree));
        }
        Ok(tree)
    }

    /// Ressources à afficher : la sélection personnelle de l'étudiant, sinon celles qu'ADE lui
    /// attribue. Le portail fait de même [vu] Rennes, 15/11/2026 : sélection vide, agenda rempli
    /// avec la ressource attribuée.
    async fn selection(&mut self) -> Result<Vec<i64>, Problem> {
        let value = self
            .get_project("user/properties/name/PortalResourcesAllSelection")
            .await?;
        let chosen = parse_selection(&value)?;
        if !chosen.is_empty() {
            return Ok(chosen);
        }
        if self.tokens.as_ref().is_some_and(|t| t.resources.is_none()) {
            self.login().await?;
        }
        Ok(self
            .tokens
            .as_ref()
            .and_then(|t| t.resources.clone())
            .unwrap_or_default())
    }
}

async fn timetable_with(
    client: &mut Client,
    tz: Tz,
    from: NaiveDate,
    to: NaiveDate,
    requested: &[String],
) -> Result<Vec<Course>, Problem> {
    if from > to {
        return Err(Problem::new(ErrorCode::InvalidRequest)
            .detail("la date de début est après la date de fin"));
    }
    // Refusées avant toute requête : une demande mal formée ne coûte rien à ADE.
    let requested = parse_requested(requested)?;
    if requested.len() > MAX_RESOURCES {
        return Err(Problem::new(ErrorCode::InvalidRequest).detail(format!(
            "plus de {MAX_RESOURCES} ressources demandées : en retirer"
        )));
    }
    let resources = if requested.is_empty() {
        client.selection().await?
    } else {
        requested
    };
    if resources.len() > MAX_RESOURCES {
        return Err(Problem::new(ErrorCode::InvalidRequest).detail(format!(
            "plus de {MAX_RESOURCES} groupes sélectionnés dans ADE : en retirer"
        )));
    }
    if resources.is_empty() {
        return Ok(Vec::new());
    }
    let path = format!(
        "resourcesEventsPortal/{}",
        events_filter(&resources, from, to)
    );
    // La même séance revient sous chaque ressource qui y participe : `parse_events` n'en garde
    // qu'une.
    parse_events(&client.get_project(&path).await?, tz, from, to)
}

/// Ce qu'ADE refuse de donner à ce compte, sans dire s'il s'agit d'une ressource inconnue ou
/// d'un droit qui manque.
fn denied() -> Problem {
    Problem::new(ErrorCode::NotFound).detail(format!(
        "{PROVIDER} : ressource inconnue, ou fermée à ce compte"
    ))
}

/// Signaux de rattachement : chemin, dans l'arbre ADE, de chaque groupe de l'étudiant.
pub(crate) async fn fetch_signals(
    ctx: &mut Context<'_>,
    base_url: &Url,
) -> Result<Vec<Signal>, Problem> {
    let (mut client, _) = Client::open(ctx, base_url)?;
    let result = groups_with(&mut client).await;
    client.finish(ctx);
    Ok(group_signals(&result?))
}

async fn groups_with(client: &mut Client) -> Result<Vec<TimetableGroup>, Problem> {
    let selected = client.selection().await?;
    let selected: HashSet<String> = selected.iter().map(i64::to_string).collect();
    Ok(resource_tree::groups(&client.tree().await?, &selected))
}

/// Arbre des ressources (étudiants, enseignants, salles), réduit aux natures demandées ; sans
/// nature demandée, l'arbre entier. Les groupes que l'étudiant suit sont marqués `selected`.
pub(crate) async fn fetch_tree(
    ctx: &mut Context<'_>,
    base_url: &Url,
    kinds: &[ResourceKind],
) -> Result<Vec<ResourceNode>, Problem> {
    let (mut client, _) = Client::open(ctx, base_url)?;
    let result = tree_with(&mut client, kinds).await;
    client.finish(ctx);
    result
}

async fn tree_with(
    client: &mut Client,
    kinds: &[ResourceKind],
) -> Result<Vec<ResourceNode>, Problem> {
    let shared = client.tree().await?;
    // L'arbre gardé sert à tous : la réponse en reçoit une copie, réduite aux natures voulues.
    let wanted = |node: &&ResourceNode| kinds.is_empty() || kinds.contains(&node.kind);
    let mut tree: Vec<ResourceNode> = shared.iter().filter(wanted).cloned().collect();
    // Les groupes de l'étudiant sont marqués sur sa copie : seulement si des groupes sont rendus,
    // la sélection coûtant une requête de plus à ADE.
    if kinds.is_empty() || kinds.contains(&ResourceKind::Group) {
        let followed: HashSet<String> = client
            .selection()
            .await?
            .iter()
            .map(i64::to_string)
            .collect();
        resource_tree::mark_followed(&mut tree, &followed);
    }
    Ok(tree)
}

/// Ressources de l'arbre qui répondent à la recherche, chacune avec son chemin.
pub(crate) async fn fetch_resources(
    ctx: &mut Context<'_>,
    base_url: &Url,
    query: &ResourceQuery,
) -> Result<Vec<TimetableResource>, Problem> {
    let (mut client, _) = Client::open(ctx, base_url)?;
    let result = client.tree().await;
    client.finish(ctx);
    Ok(resource_tree::resources(&result?, query))
}

/// Cours de la plage `from..=to` pour les ressources ADE demandées (`resources`, des ids de
/// l'arbre) ; sans demande, pour les groupes sélectionnés par l'étudiant dans ADE.
pub(crate) async fn fetch_timetable(
    ctx: &mut Context<'_>,
    base_url: &Url,
    from: NaiveDate,
    to: NaiveDate,
    resources: &[String],
) -> Result<Vec<Course>, Problem> {
    let (mut client, tz) = Client::open(ctx, base_url)?;
    let result = timetable_with(&mut client, tz, from, to, resources).await;
    client.finish(ctx);
    result
}

/// Groupes d'étudiants de l'arbre ADE, ceux de la sélection de l'étudiant marqués `selected`.
pub(crate) async fn fetch_groups(
    ctx: &mut Context<'_>,
    base_url: &Url,
) -> Result<Vec<TimetableGroup>, Problem> {
    let (mut client, _) = Client::open(ctx, base_url)?;
    let result = groups_with(&mut client).await;
    client.finish(ctx);
    result
}

#[cfg(test)]
mod tests;
