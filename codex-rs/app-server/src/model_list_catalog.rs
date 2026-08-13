use crate::error_code::invalid_request;
use crate::models::model_from_preset;
use crate::outgoing_message::OutgoingMessageSender;
use crate::richcodex_backend::ModelSummary;
use crate::richcodex_model_routes::ModelRouteProjectionError;
use crate::richcodex_model_routes::project_model_routes;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::Model;
use codex_app_server_protocol::ModelListParams;
use codex_app_server_protocol::ModelListResponse;
use codex_app_server_protocol::ModelListUpdatedNotification;
use codex_app_server_protocol::ServerNotification;
use codex_core::RuntimeModelProviderRoutes;
use codex_http_client::HttpClientFactory;
use codex_models_manager::manager::RefreshStrategy;
use codex_models_manager::manager::SharedModelsManager;
use codex_protocol::openai_models::ModelsResponse;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use tokio::sync::Mutex;

const CURSOR_VERSION: &str = "v1";

#[derive(Clone, Debug)]
struct ModelListSnapshot {
    models: Arc<Vec<Model>>,
    revision: u64,
}

#[derive(Debug)]
struct PublishedModelList {
    models: Arc<Vec<Model>>,
    revision: u64,
    last_observation: u64,
}

/// Process-scoped authority for the app-server model picker snapshot.
///
/// Every config publication, periodic refresh, and foreground list request reports through this
/// coordinator. It turns semantic model-list changes into one revisioned invalidation stream and
/// prevents a slower, older observation from overwriting a newer published snapshot.
pub(crate) struct ModelListCatalog {
    models_manager: SharedModelsManager,
    http_client_factory: HttpClientFactory,
    outgoing: Arc<OutgoingMessageSender>,
    next_observation: AtomicU64,
    published: Mutex<PublishedModelList>,
    runtime_model_provider_routes: Option<RuntimeModelProviderRoutes>,
}

impl ModelListCatalog {
    #[cfg(test)]
    pub(crate) fn new(
        models_manager: SharedModelsManager,
        http_client_factory: HttpClientFactory,
        outgoing: Arc<OutgoingMessageSender>,
    ) -> Self {
        Self::new_with_runtime_routes(models_manager, http_client_factory, outgoing, &[], None)
    }

    pub(crate) fn new_with_runtime_routes(
        models_manager: SharedModelsManager,
        http_client_factory: HttpClientFactory,
        outgoing: Arc<OutgoingMessageSender>,
        routes: &[ModelSummary],
        runtime_model_provider_routes: Option<RuntimeModelProviderRoutes>,
    ) -> Self {
        if !routes.is_empty() {
            match models_manager
                .try_get_remote_models()
                .map_err(|_| ModelRouteProjectionError)
                .and_then(|candidates| project_model_routes(&candidates, routes))
            {
                Ok(overlay) => {
                    models_manager.replace_runtime_catalog_overlay(Some(overlay));
                    replace_active_runtime_tags(runtime_model_provider_routes.as_ref(), routes);
                }
                Err(ModelRouteProjectionError) => {
                    tracing::warn!(
                        "preserving the native model catalog because stored RichCodex routes could not be projected"
                    );
                }
            }
        }
        let models = models_manager
            .try_list_models()
            .unwrap_or_default()
            .into_iter()
            .map(model_from_preset)
            .collect();
        Self {
            models_manager,
            http_client_factory,
            outgoing,
            next_observation: AtomicU64::new(1),
            published: Mutex::new(PublishedModelList {
                models: Arc::new(models),
                revision: 1,
                last_observation: 0,
            }),
            runtime_model_provider_routes,
        }
    }

    pub(crate) async fn list(
        &self,
        params: ModelListParams,
    ) -> Result<ModelListResponse, JSONRPCErrorError> {
        let snapshot = self.observe(RefreshStrategy::OnlineIfUncached).await;
        paginate(snapshot, params)
    }

    pub(crate) async fn refresh(&self, strategy: RefreshStrategy) {
        self.observe(strategy).await;
    }

    pub(crate) async fn replace_overlay(&self, overlay: Option<ModelsResponse>) -> bool {
        if !self.models_manager.replace_catalog_overlay(overlay).await {
            return false;
        }
        self.observe(RefreshStrategy::Offline).await;
        true
    }

    pub(crate) async fn validate_route_semantic_model(
        &self,
        semantic_model: &str,
    ) -> Result<(), ModelRouteProjectionError> {
        self.models_manager
            .get_remote_models()
            .await
            .iter()
            .any(|candidate| candidate.slug == semantic_model)
            .then_some(())
            .ok_or(ModelRouteProjectionError)
    }

    pub(crate) async fn publish_runtime_routes(
        &self,
        routes: &[ModelSummary],
    ) -> Result<bool, ModelRouteProjectionError> {
        let candidates = self.models_manager.get_remote_models().await;
        let overlay = project_model_routes(&candidates, routes)?;
        let changed = self
            .models_manager
            .replace_runtime_catalog_overlay(Some(overlay));
        replace_active_runtime_tags(self.runtime_model_provider_routes.as_ref(), routes);
        if !changed {
            return Ok(false);
        }
        self.observe(RefreshStrategy::Offline).await;
        Ok(true)
    }

    async fn observe(&self, strategy: RefreshStrategy) -> ModelListSnapshot {
        let observation = self.next_observation.fetch_add(1, Ordering::Relaxed);
        let models = self
            .models_manager
            .list_models(strategy, self.http_client_factory.clone())
            .await
            .into_iter()
            .map(model_from_preset)
            .collect::<Vec<_>>();

        let (snapshot, notification) = {
            let mut published = self.published.lock().await;
            if observation < published.last_observation {
                return snapshot(&published);
            }
            published.last_observation = observation;
            if published.models.as_ref() == &models {
                return snapshot(&published);
            }

            published.models = Arc::new(models);
            published.revision = published.revision.saturating_add(1);
            let snapshot = snapshot(&published);
            let notification = ModelListUpdatedNotification {
                revision: published.revision.to_string(),
            };
            (snapshot, notification)
        };
        self.outgoing
            .send_server_notification(ServerNotification::ModelListUpdated(notification))
            .await;
        snapshot
    }
}

fn replace_active_runtime_tags(
    runtime_routes: Option<&RuntimeModelProviderRoutes>,
    routes: &[ModelSummary],
) {
    if let Some(runtime_routes) = runtime_routes {
        runtime_routes.replace_active_model_tags(
            routes
                .iter()
                .filter(|route| !route.retired)
                .map(|route| route.model_tag.clone()),
        );
    }
}

fn snapshot(published: &PublishedModelList) -> ModelListSnapshot {
    ModelListSnapshot {
        models: Arc::clone(&published.models),
        revision: published.revision,
    }
}

fn paginate(
    snapshot: ModelListSnapshot,
    params: ModelListParams,
) -> Result<ModelListResponse, JSONRPCErrorError> {
    let ModelListParams {
        limit,
        cursor,
        include_hidden,
    } = params;
    let include_hidden = include_hidden.unwrap_or(false);
    let models = snapshot
        .models
        .iter()
        .filter(|model| include_hidden || !model.hidden)
        .cloned()
        .collect::<Vec<_>>();
    let total = models.len();
    let start = match cursor {
        Some(cursor) => parse_cursor(&cursor, snapshot.revision, include_hidden)?,
        None => 0,
    };
    if start > total {
        return Err(invalid_request(format!(
            "cursor offset {start} exceeds total models {total}"
        )));
    }

    let effective_limit = limit.unwrap_or(total as u32).max(1) as usize;
    let end = start.saturating_add(effective_limit).min(total);
    let next_cursor = (end < total).then(|| encode_cursor(snapshot.revision, include_hidden, end));
    Ok(ModelListResponse {
        data: models[start..end].to_vec(),
        revision: snapshot.revision.to_string(),
        next_cursor,
    })
}

fn encode_cursor(revision: u64, include_hidden: bool, offset: usize) -> String {
    format!(
        "{CURSOR_VERSION}:{revision}:{}:{offset}",
        u8::from(include_hidden)
    )
}

fn parse_cursor(
    cursor: &str,
    expected_revision: u64,
    expected_include_hidden: bool,
) -> Result<usize, JSONRPCErrorError> {
    let mut parts = cursor.split(':');
    let (Some(version), Some(revision), Some(include_hidden), Some(offset), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return Err(invalid_request(format!("invalid model cursor: {cursor}")));
    };
    if version != CURSOR_VERSION {
        return Err(invalid_request(format!(
            "unsupported model cursor version: {version}"
        )));
    }
    let revision = revision
        .parse::<u64>()
        .map_err(|_| invalid_request(format!("invalid model cursor revision: {cursor}")))?;
    if revision != expected_revision {
        return Err(invalid_request(
            "model catalog changed; restart pagination from the first page",
        ));
    }
    let include_hidden = match include_hidden {
        "0" => false,
        "1" => true,
        _ => return Err(invalid_request(format!("invalid model cursor: {cursor}"))),
    };
    if include_hidden != expected_include_hidden {
        return Err(invalid_request(
            "model cursor does not match the includeHidden selection",
        ));
    }
    offset
        .parse::<usize>()
        .map_err(|_| invalid_request(format!("invalid model cursor offset: {cursor}")))
}
