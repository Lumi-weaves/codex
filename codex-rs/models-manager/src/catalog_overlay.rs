use crate::manager::ModelsManager;
use crate::manager::ModelsManagerFuture;
use crate::manager::RefreshStrategy;
use crate::manager::SharedModelsManager;
use codex_http_client::HttpClientFactory;
use codex_login::AuthManager;
use codex_protocol::config_types::CollaborationModeMask;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelsResponse;
use std::sync::Arc;
use tokio::sync::TryLockError;

#[derive(Debug)]
struct CatalogOverlayModelsManager {
    inner: SharedModelsManager,
    overlay: Vec<ModelInfo>,
}

impl CatalogOverlayModelsManager {
    fn merge(&self, models: Vec<ModelInfo>) -> Vec<ModelInfo> {
        merge_catalog(models, &self.overlay)
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
            if let Some(model) = model
                && self
                    .overlay
                    .iter()
                    .any(|candidate| candidate.slug == *model)
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
}

pub fn with_catalog_overlay(
    manager: SharedModelsManager,
    overlay: Option<ModelsResponse>,
) -> SharedModelsManager {
    let Some(overlay) = overlay else {
        return manager;
    };
    Arc::new(CatalogOverlayModelsManager {
        inner: manager,
        overlay: overlay.models,
    })
}

#[cfg(test)]
#[path = "catalog_overlay_tests.rs"]
mod tests;
