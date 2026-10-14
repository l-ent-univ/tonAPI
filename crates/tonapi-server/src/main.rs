//! `tonapi-server` : lance l'API.
//!
//! Variables d'environnement :
//! - `TONAPI_ADDR` (défaut `127.0.0.1:8080`)
//! - `TONAPI_UNIVERSITIES` (défaut `universities`)
//! - `TONAPI_SESSION_KEYS` : `kid:base64,…`, générées par `tonapi keygen`. Sans elles, une clé
//!   temporaire est créée et les jetons ne survivent pas au redémarrage.
//! - `TONAPI_SESSION_TTL_SECS` (défaut 43200, soit 12 h)
//! - `RUST_LOG` (défaut `info`)

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
    let ttl = match std::env::var("TONAPI_SESSION_TTL_SECS") {
        Ok(v) => v.parse().context("TONAPI_SESSION_TTL_SECS")?,
        Err(_) => 43_200,
    };
    let keys = if let Ok(v) = std::env::var("TONAPI_SESSION_KEYS") {
        SessionKeys::parse(&v).context("TONAPI_SESSION_KEYS")?
    } else {
        tracing::warn!(
            "TONAPI_SESSION_KEYS absente : clé temporaire, les jetons seront invalides au redémarrage"
        );
        SessionKeys::ephemeral()?
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
        },
    );
    let _watcher = watch(dir, state.registry.clone()).context("surveillance des universités")?;

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("écoute sur {addr}"))?;
    tracing::info!("http://{addr}/docs");
    axum::serve(listener, app(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
