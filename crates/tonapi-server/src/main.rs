//! `tonapi-server` : lance l'API.
//!
//! Variables d'environnement :
//! - `TONAPI_ADDR` (défaut `127.0.0.1:8080`)
//! - `TONAPI_UNIVERSITIES` (défaut `universities`)
//! - `TONAPI_SESSION_KEYS` : `kid:base64,…`, générées par `tonapi keygen`. Obligatoires, sauf avec
//!   `TONAPI_ENV=dev` : une clé temporaire est alors créée et les jetons ne survivent pas au
//!   redémarrage.
//! - `TONAPI_SESSION_TTL_SECS` (défaut 43200, soit 12 h ; entre 60 s et 7 jours)
//! - `TONAPI_TRUST_PROXY=1` : derrière un répartiteur de confiance, l'adresse du client est la
//!   dernière de `X-Forwarded-For` (limite des connexions par adresse)
//! - `TONAPI_DOCS=0` : ne pas servir `/docs` ni `/openapi.json`
//! - `RUST_LOG` (défaut `info`)

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use tonapi_config::Registry;
use tonapi_server::{AppState, Settings, app, watch};
use tonapi_session::SessionKeys;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

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
    let dev = std::env::var("TONAPI_ENV").is_ok_and(|v| v == "dev");
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
            trust_proxy: flag("TONAPI_TRUST_PROXY", false),
            docs: flag("TONAPI_DOCS", true),
        },
    );
    let _watcher = watch(dir, state.registry.clone()).context("surveillance des universités")?;

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("écoute sur {addr}"))?;
    tracing::info!("http://{addr}/docs");
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await?;
    Ok(())
}

/// Ctrl+C ou SIGTERM (envoyé par Docker, Kubernetes et la plupart des hébergeurs) : le serveur
/// n'accepte plus de connexion et termine les requêtes en cours.
async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!("arrêt demandé : fin des requêtes en cours");
}
