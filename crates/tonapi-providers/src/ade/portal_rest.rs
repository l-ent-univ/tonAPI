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
//! Les ressources à interroger sont la sélection personnelle de l'étudiant
//! (`PortalResourcesAllSelection`), sinon celles qu'ADE lui attribue dans la réponse de `ssologin`.
//! Les jetons ADE restent dans `session.tokens["ade-rest.<hôte>"]`, jamais le mot de passe.
//!
//! Faits non confirmés par la documentation, marqués `[inconnu]` ci-dessous : format exact de la
//! sélection, champs de l'arbre au-delà de `id`/`name`/`branch`/`leaf`, type de séance.

use chrono::NaiveDate;
use chrono_tz::Tz;
use reqwest::Method;
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tonapi_core::{Course, ErrorCode, Problem, TimetableGroup};
use url::Url;

use crate::Context;
use crate::common::token_key;
use crate::net::{Net, upstream};

mod parse;

use parse::{
    events_filter, normalize, parse_events, parse_groups, parse_refresh, parse_selection,
    parse_sso_token, parse_ssologin,
};

/// Nom du fournisseur dans les messages d'erreur.
const PROVIDER: &str = "ade-portal-rest";
/// Nombre maximal de ressources sélectionnées interrogées pour une même plage (une requête chacune).
const MAX_RESOURCES: usize = 30;

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
}

/// Réponse d'ADE : des données, ou une session perdue (401 ou redirection, p. ex. vers le CAS).
enum Reply {
    Data(Value),
    Lost,
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

    /// Une requête. 401 et redirections donnent [`Reply::Lost`] ; les autres statuts d'erreur et les
    /// corps non JSON donnent une erreur qui ne cite ni corps ni jeton.
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
            Reply::Lost => Ok(None),
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
            Reply::Lost => Err(Problem::new(ErrorCode::SessionExpired)
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
                });
                self.fresh = true;
                return Ok(());
            }
        }
        self.login().await
    }

    /// `GET` d'une ressource du projet, avec le contrat « un seul nouvel essai sur 401 ».
    async fn get_project(&mut self, path: &str) -> Result<Value, Problem> {
        if self.tokens.is_none() {
            self.login().await?;
        }
        let url = |client: &Self| -> Result<Url, Problem> {
            let project = client.tokens.as_ref().map_or(0, |t| t.project);
            client.url(&format!("RestApi/portal/projects/{project}/{path}"))
        };
        let access = |client: &Self| client.tokens.as_ref().map(|t| t.access.clone());

        let first = url(self)?;
        if let Reply::Data(value) = self
            .send(Method::GET, first, access(self).as_deref(), None, true)
            .await?
        {
            self.fresh = false;
            return Ok(value);
        }
        let expired = || {
            Problem::new(ErrorCode::SessionExpired).detail("ADE refuse le jeton : se reconnecter")
        };
        if self.fresh {
            self.tokens = None;
            return Err(expired());
        }
        self.renew().await?;
        let second = url(self)?;
        match self
            .send(Method::GET, second, access(self).as_deref(), None, true)
            .await?
        {
            Reply::Data(value) => {
                self.fresh = false;
                Ok(value)
            }
            Reply::Lost => {
                self.tokens = None;
                Err(expired())
            }
        }
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
) -> Result<Vec<Course>, Problem> {
    if from > to {
        return Err(Problem::new(ErrorCode::InvalidRequest)
            .detail("la date de début est après la date de fin"));
    }
    let resources = client.selection().await?;
    if resources.len() > MAX_RESOURCES {
        return Err(Problem::new(ErrorCode::InvalidRequest).detail(format!(
            "plus de {MAX_RESOURCES} groupes sélectionnés dans ADE : en retirer"
        )));
    }
    // Le front envoie une requête par ressource ; on fait de même.
    let mut courses = Vec::new();
    for resource in resources {
        let path = format!(
            "resourcesEventsPortal/{}",
            events_filter(resource, from, to)
        );
        let response = client.get_project(&path).await?;
        courses.extend(parse_events(&response, tz, from, to)?);
    }
    Ok(normalize(courses))
}

async fn groups_with(client: &mut Client) -> Result<Vec<TimetableGroup>, Problem> {
    let selected = client.selection().await?;
    let tree = client.get_project("resourcestree").await?;
    parse_groups(&tree, &selected)
}

/// Cours de la plage `from..=to` pour les groupes sélectionnés par l'étudiant dans ADE.
pub(crate) async fn fetch_timetable(
    ctx: &mut Context<'_>,
    base_url: &Url,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Course>, Problem> {
    let (mut client, tz) = Client::open(ctx, base_url)?;
    let result = timetable_with(&mut client, tz, from, to).await;
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
