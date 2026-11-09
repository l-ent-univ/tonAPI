//! `tonapi-server` : lance l'API.
//!
//! Variables d'environnement :
//! - `TONAPI_ADDR` (défaut `127.0.0.1:8080`)
//! - `TONAPI_UNIVERSITIES` (défaut `universities`)
//! - `TONAPI_SESSION_KEYS` : `kid:base64,…`, générées par `tonapi keygen`. Obligatoires, sauf avec
//!   `TONAPI_ENV=dev` : une clé temporaire est alors créée et les jetons ne survivent pas au
//!   redémarrage.
//! - `TONAPI_SESSION_TTL_SECS` (défaut 43200, soit 12 h ; entre 60 s et 7 jours)
//! - `TONAPI_TRUSTED_PROXIES` : adresses ou plages (`10.0.0.0/8,192.0.2.7`) des répartiteurs de
//!   confiance ; seules leurs connexions font lire `X-Forwarded-For` (limite par adresse)
//! - `TONAPI_DOCS` : sert `/docs` et `/openapi.json` (défaut : oui en dev, non sinon)
//! - `TONAPI_RELOAD` : recharge `universities/` à chaque modification (défaut : oui en dev, non
//!   sinon ; en production, la configuration change par redéploiement)
//! - `TONAPI_AUTH_PER_IP` (défaut 120, entre 1 et 10 000) : demandes de connexion par adresse et
//!   par minute
//! - `TONAPI_LOG_FORMAT` : `json` (une ligne JSON par événement) ou `text` (défaut : `text` en
//!   dev, `json` sinon)
//! - `TONAPI_METRICS_ADDR` : adresse interne de `GET /metrics` (`127.0.0.1:9090`) ; absente,
//!   pas de métriques servies
//! - `TONAPI_DRAIN_SECS` (défaut 0 en dev, 5 sinon ; au plus 300) : après SIGTERM, durée pendant
//!   laquelle `/readyz` répond 503 alors que le serveur accepte encore des requêtes
//! - `TONAPI_GRACE_SECS` (défaut 130, entre 1 et 3600) : attente maximale des requêtes en cours
//!   une fois les nouvelles connexions refusées
//! - `RUST_LOG` (défaut `info`)

use std::future::IntoFuture;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tonapi_config::Registry;
use tonapi_server::{AppState, Cidr, DEFAULT_AUTH_PER_IP, Settings, app, metrics_app, watch};
use tonapi_session::SessionKeys;
use tracing_subscriber::EnvFilter;

/// Journaux sur la sortie standard, en JSON hors dev.
fn init_logs(dev: bool) -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let json = match std::env::var("TONAPI_LOG_FORMAT").as_deref() {
        Ok("json") => true,
        Ok("text") => false,
        Ok(other) => bail!("TONAPI_LOG_FORMAT : `{other}` (attendu : json ou text)"),
        Err(_) => !dev,
    };
    if json {
        tracing_subscriber::fmt()
            .json()
            .flatten_event(true)
            .with_current_span(false)
            .with_env_filter(filter)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let dev = std::env::var("TONAPI_ENV").is_ok_and(|v| v == "dev");
    init_logs(dev)?;

    let addr = std::env::var("TONAPI_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let dir = PathBuf::from(
        std::env::var("TONAPI_UNIVERSITIES").unwrap_or_else(|_| "universities".into()),
    );
    let ttl: i64 = match std::env::var("TONAPI_SESSION_TTL_SECS") {
        Ok(v) => v.parse().context("TONAPI_SESSION_TTL_SECS")?,
        Err(_) => 43_200,
    };
    if !(60..=7 * 86_400).contains(&ttl) {
        bail!("TONAPI_SESSION_TTL_SECS doit être entre 60 et 604800 secondes");
    }
    let keys = if let Ok(v) = std::env::var("TONAPI_SESSION_KEYS") {
        SessionKeys::parse(&v).context("TONAPI_SESSION_KEYS")?
    } else if dev {
        tracing::warn!(
            "TONAPI_SESSION_KEYS absente : clé temporaire, les jetons seront invalides au redémarrage"
        );
        SessionKeys::ephemeral()?
    } else {
        // Une clé par instance donnerait des 401 au hasard derrière un répartiteur, et chaque
        // redémarrage déconnecterait tout le monde.
        bail!("TONAPI_SESSION_KEYS manquante (générer avec `tonapi keygen`, ou TONAPI_ENV=dev)");
    };
    let flag = |name: &str, default: bool| {
        std::env::var(name).map_or(default, |v| matches!(v.as_str(), "1" | "true" | "yes"))
    };

    if std::env::var_os("TONAPI_TRUST_PROXY").is_some() {
        bail!(
            "TONAPI_TRUST_PROXY n'existe plus : déclarer les répartiteurs avec \
             TONAPI_TRUSTED_PROXIES=<adresse ou plage>,…"
        );
    }
    let trusted_proxies = match std::env::var("TONAPI_TRUSTED_PROXIES") {
        Ok(list) => list
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| Cidr::parse(s).with_context(|| format!("TONAPI_TRUSTED_PROXIES : `{s}`")))
            .collect::<Result<Vec<_>>>()?,
        Err(_) => Vec::new(),
    };
    let auth_per_ip: u32 = match std::env::var("TONAPI_AUTH_PER_IP") {
        Ok(v) => v.parse().context("TONAPI_AUTH_PER_IP")?,
        Err(_) => DEFAULT_AUTH_PER_IP,
    };
    if !(1..=10_000).contains(&auth_per_ip) {
        bail!("TONAPI_AUTH_PER_IP doit être entre 1 et 10000");
    }
    let drain = seconds("TONAPI_DRAIN_SECS", if dev { 0 } else { 5 }, 0..=300)?;
    let grace = seconds("TONAPI_GRACE_SECS", 130, 1..=3600)?;
    let metrics_addr = std::env::var("TONAPI_METRICS_ADDR").ok();

    let (registry, report) = Registry::load(&dir);
    for issue in &report.rejected {
        tracing::warn!("{issue}");
    }
    if registry.is_empty() {
        bail!("aucune université valide dans {}", dir.display());
    }
    tracing::info!(universities = registry.len(), dir = %dir.display(), "configuration chargée");

    let state = AppState::new(
        registry,
        keys,
        Settings {
            universities_dir: dir.clone(),
            session_ttl_secs: ttl,
            trusted_proxies,
            docs: flag("TONAPI_DOCS", dev),
            auth_per_ip,
        },
    );
    let _watcher = if flag("TONAPI_RELOAD", dev) {
        Some(watch(dir, state.registry.clone()).context("surveillance des universités")?)
    } else {
        None
    };

    if let Some(metrics_addr) = metrics_addr {
        serve_metrics(&metrics_addr).await?;
    }

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("écoute sur {addr}"))?;
    if state.settings.docs {
        tracing::info!("http://{addr}/docs");
    } else {
        tracing::info!("écoute sur http://{addr}");
    }

    serve(listener, state, drain, grace).await
}

/// Durée en secondes lue dans `name`, bornée.
fn seconds(name: &str, default: u64, range: std::ops::RangeInclusive<u64>) -> Result<Duration> {
    let value = match std::env::var(name) {
        Ok(v) => v.parse().with_context(|| name.to_owned())?,
        Err(_) => default,
    };
    if !range.contains(&value) {
        bail!(
            "{name} doit être entre {} et {} secondes",
            range.start(),
            range.end()
        );
    }
    Ok(Duration::from_secs(value))
}

/// Ctrl+C ou SIGTERM (envoyé par Docker, Kubernetes et la plupart des hébergeurs).
async fn signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            // Sans SIGTERM, seul Ctrl+C arrête le serveur : ne jamais s'arrêter dès le départ.
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

/// `GET /metrics` sur une adresse interne, en tâche de fond.
async fn serve_metrics(addr: &str) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("écoute des métriques sur {addr}"))?;
    tracing::info!("métriques : http://{addr}/metrics");
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, metrics_app()).await {
            tracing::error!("serveur de métriques arrêté : {error}");
        }
    });
    Ok(())
}

/// Sert l'API. Arrêt en trois temps : `/readyz` à 503 pendant `drain`, puis plus de nouvelle
/// connexion, puis au plus `grace` pour finir les requêtes en cours.
async fn serve(
    listener: tokio::net::TcpListener,
    state: AppState,
    drain: Duration,
    grace: Duration,
) -> Result<()> {
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let readiness = state.readiness();
    tokio::spawn(async move {
        signal().await;
        readiness.drain();
        tracing::info!(
            drain_secs = drain.as_secs(),
            "arrêt demandé : /readyz répond 503"
        );
        tokio::time::sleep(drain).await;
        tracing::info!("nouvelles connexions refusées : fin des requêtes en cours");
        let _ = stop.send(true);
    });
    let mut until_stop = stopped.clone();
    let server = axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = until_stop.wait_for(|stop| *stop).await;
    })
    .into_future();
    let mut until_grace = stopped;
    tokio::select! {
        result = server => result?,
        () = async move {
            let _ = until_grace.wait_for(|stop| *stop).await;
            tokio::time::sleep(grace).await;
        } => {
            tracing::warn!(
                grace_secs = grace.as_secs(),
                "délai de grâce écoulé : requêtes encore en cours interrompues"
            );
        }
    }
    Ok(())
}
