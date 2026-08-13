use crate::manager::ModelsManager;
use crate::manager::ModelsManagerFuture;
use crate::manager::RefreshStrategy;
use crate::manager::SharedModelsManager;
use codex_http_client::HttpClientFactory;
use codex_login::AuthManager;
use codex_protocol::config_types::CollaborationModeMask;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ModelsResponse;
use std::sync::Arc;
use std::sync::RwLock;
use tokio::sync::TryLockError;

#[derive(Debug)]
struct CatalogOverlayModelsManager {
    inner: SharedModelsManager,
    configured_overlay: RwLock<Arc<Vec<ModelInfo>>>,
    runtime_overlay: RwLock<Arc<Vec<ModelInfo>>>,
}

impl CatalogOverlayModelsManager {
    fn configured_overlay(&self) -> Arc<Vec<ModelInfo>> {
        self.configured_overlay
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn runtime_overlay(&self) -> Arc<Vec<ModelInfo>> {
        self.runtime_overlay
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn merge(&self, models: Vec<ModelInfo>) -> Vec<ModelInfo> {
        let models = merge_catalog(models, self.configured_overlay().as_ref());
        merge_catalog(models, self.runtime_overlay().as_ref())
    }
}

fn merge_catalog(mut models: Vec<ModelInfo>, overlay: &[ModelInfo]) -> Vec<ModelInfo> {
    for overlay in overlay {
        if let Some(existing) = models.iter_mut().find(|model| model.slug == overlay.slug) {
            *existing = overlay.clone();
        } else {
            models.push(overlay.clone());
        }
    }
    models
}

impl ModelsManager for CatalogOverlayModelsManager {
    fn get_default_model<'a>(
        &'a self,
        model: &'a Option<String>,
        allow_provider_model_fallback: bool,
        refresh_strategy: RefreshStrategy,
        http_client_factory: HttpClientFactory,
    ) -> ModelsManagerFuture<'a, String> {
        Box::pin(async move {
            let configured_overlay = self.configured_overlay();
            let runtime_overlay = self.runtime_overlay();
            if let Some(model) = model
                && (configured_overlay
                    .iter()
                    .any(|candidate| candidate.slug == *model)
                    || runtime_overlay.iter().any(|candidate| {
                        candidate.slug == *model && candidate.visibility == ModelVisibility::List
                    }))
            {
                return model.clone();
            }
            if model.is_none() {
                let available = self
                    .list_models(refresh_strategy, http_client_factory)
                    .await;
                return available
                    .iter()
                    .find(|preset| preset.is_default)
                    .or_else(|| available.first())
                    .map(|preset| preset.model.clone())
                    .unwrap_or_default();
            }
            self.inner
                .get_default_model(
                    model,
                    allow_provider_model_fallback,
                    refresh_strategy,
                    http_client_factory,
                )
                .await
        })
    }

    fn raw_model_catalog(
        &self,
        refresh_strategy: RefreshStrategy,
        http_client_factory: HttpClientFactory,
    ) -> ModelsManagerFuture<'_, ModelsResponse> {
        Box::pin(async move {
            let catalog = self
                .inner
                .raw_model_catalog(refresh_strategy, http_client_factory)
                .await;
            ModelsResponse {
                models: self.merge(catalog.models),
            }
        })
    }

    fn get_remote_models(&self) -> ModelsManagerFuture<'_, Vec<ModelInfo>> {
        Box::pin(async move { self.merge(self.inner.get_remote_models().await) })
    }

    fn try_get_remote_models(&self) -> Result<Vec<ModelInfo>, TryLockError> {
        self.inner
            .try_get_remote_models()
            .map(|models| self.merge(models))
    }

    fn auth_manager(&self) -> Option<&AuthManager> {
        self.inner.auth_manager()
    }

    fn list_collaboration_modes(&self) -> Vec<CollaborationModeMask> {
        self.inner.list_collaboration_modes()
    }

    fn refresh_if_new_etag(
        &self,
        etag: String,
        http_client_factory: HttpClientFactory,
    ) -> ModelsManagerFuture<'_, ()> {
        self.inner.refresh_if_new_etag(etag, http_client_factory)
    }

    fn replace_catalog_overlay(
        &self,
        overlay: Option<ModelsResponse>,
    ) -> ModelsManagerFuture<'_, bool> {
        Box::pin(async move {
            let replacement = overlay.map_or_else(Vec::new, |overlay| overlay.models);
            let mut current = self
                .configured_overlay
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if current.as_ref() == &replacement {
                return false;
            }
            *current = Arc::new(replacement);
            true
        })
    }

    fn replace_runtime_catalog_overlay(&self, overlay: Option<ModelsResponse>) -> bool {
        let replacement = overlay.map_or_else(Vec::new, |overlay| overlay.models);
        let mut current = self
            .runtime_overlay
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current.as_ref() == &replacement {
            return false;
        }
        *current = Arc::new(replacement);
        true
    }
}

pub fn with_catalog_overlay(
    manager: SharedModelsManager,
    overlay: Option<ModelsResponse>,
) -> SharedModelsManager {
    Arc::new(CatalogOverlayModelsManager {
        inner: manager,
        configured_overlay: RwLock::new(Arc::new(
            overlay.map_or_else(Vec::new, |overlay| overlay.models),
        )),
        runtime_overlay: RwLock::new(Arc::new(Vec::new())),
    })
}

#[cfg(test)]
#[path = "catalog_overlay_tests.rs"]
mod tests;
