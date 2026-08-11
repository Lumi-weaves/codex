use std::sync::Arc;
use std::time::Duration;

use crate::model_list_catalog::ModelListCatalog;
use codex_models_manager::manager::RefreshStrategy;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const MODELS_REFRESH_INTERVAL: Duration = Duration::from_secs(3 * 60);

#[derive(Debug)]
pub(crate) struct ModelsRefreshWorker {
    shutdown: CancellationToken,
    _task: JoinHandle<()>,
}

impl ModelsRefreshWorker {
    pub(crate) fn shutdown(&self) {
        self.shutdown.cancel();
    }
}

impl Drop for ModelsRefreshWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(crate) fn spawn(model_list_catalog: &Arc<ModelListCatalog>) -> ModelsRefreshWorker {
    spawn_with_interval(model_list_catalog, MODELS_REFRESH_INTERVAL)
}

fn spawn_with_interval(
    model_list_catalog: &Arc<ModelListCatalog>,
    refresh_interval: Duration,
) -> ModelsRefreshWorker {
    let model_list_catalog = Arc::downgrade(model_list_catalog);
    let shutdown = CancellationToken::new();
    let worker_shutdown = shutdown.clone();
    let task = tokio::spawn(async move {
        loop {
            if worker_shutdown.is_cancelled() {
                break;
            }
            let Some(model_list_catalog) = model_list_catalog.upgrade() else {
                break;
            };
            model_list_catalog.refresh(RefreshStrategy::Online).await;
            drop(model_list_catalog);

            tokio::select! {
                _ = worker_shutdown.cancelled() => break,
                _ = tokio::time::sleep(refresh_interval) => {}
            }
        }
    });
    ModelsRefreshWorker {
        shutdown,
        _task: task,
    }
}

#[cfg(test)]
#[path = "models_refresh_worker_tests.rs"]
mod tests;
