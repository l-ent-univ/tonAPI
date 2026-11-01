//! Rechargement à chaud du dossier des universités.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use notify::{RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tonapi_config::Registry;

/// Surveille `dir` et recharge les universités après chaque modification.
///
/// Le nouveau contenu n'est servi que si tous les fichiers sont valides : un fichier en cours
/// d'édition ne fait jamais disparaître une université. Les problèmes sont journalisés.
/// Le surveillant vit tant que la valeur renvoyée n'est pas abandonnée.
pub fn watch(
    dir: PathBuf,
    registry: Arc<ArcSwap<Registry>>,
) -> notify::Result<notify::RecommendedWatcher> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|e| !e.kind.is_access()) {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(&dir, RecursiveMode::Recursive)?;

    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            // Regroupe les événements d'une même sauvegarde.
            tokio::time::sleep(Duration::from_millis(300)).await;
            while rx.try_recv().is_ok() {}

            // Lecture de fichiers synchrone : hors des fils du runtime.
            let source = dir.clone();
            let Ok((next, report)) =
                tokio::task::spawn_blocking(move || Registry::load(&source)).await
            else {
                tracing::warn!(
                    "rechargement interrompu : la configuration précédente reste servie"
                );
                continue;
            };
            if report.is_ok() {
                tracing::info!(universities = next.len(), "universités rechargées");
                registry.store(Arc::new(next));
            } else {
                for issue in &report.rejected {
                    tracing::warn!("{issue}");
                }
                tracing::warn!(
                    problems = report.rejected.len(),
                    "rechargement refusé : la configuration précédente reste servie"
                );
            }
        }
    });
    Ok(watcher)
}
