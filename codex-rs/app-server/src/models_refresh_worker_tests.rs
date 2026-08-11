use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use codex_http_client::HttpClientFactory;
use codex_http_client::OutboundProxyPolicy;
use codex_models_manager::manager::ModelsEndpointClient;
use codex_models_manager::manager::ModelsEndpointFuture;
use codex_models_manager::manager::OpenAiModelsManager;
use codex_models_manager::manager::SharedModelsManager;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CoreResult;
use codex_protocol::openai_models::ModelInfo;
use pretty_assertions::assert_eq;
use tempfile::tempdir;
use tokio::sync::Notify;

use super::*;

#[derive(Debug)]
struct TestModelsEndpoint {
    fetch_count: AtomicUsize,
    fetched: Notify,
    release_second_fetch: Notify,
}

#[derive(Debug)]
struct FixedModelsEndpoint {
    model: ModelInfo,
    fetch_count: AtomicUsize,
}

impl ModelsEndpointClient for FixedModelsEndpoint {
    fn has_command_auth(&self) -> bool {
        true
    }

    fn uses_codex_backend(&self) -> ModelsEndpointFuture<'_, bool> {
        Box::pin(async { false })
    }

    fn list_models<'a>(
        &'a self,
        _client_version: &'a str,
        _http_client_factory: HttpClientFactory,
    ) -> ModelsEndpointFuture<'a, CoreResult<(Vec<ModelInfo>, Option<String>)>> {
        Box::pin(async move {
            self.fetch_count.fetch_add(1, Ordering::SeqCst);
            Ok((vec![self.model.clone()], None))
        })
    }
}

impl TestModelsEndpoint {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            fetch_count: AtomicUsize::new(0),
            fetched: Notify::new(),
            release_second_fetch: Notify::new(),
        })
    }

    async fn wait_for_fetch_count(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while self.fetch_count.load(Ordering::SeqCst) < expected {
                self.fetched.notified().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("expected {expected} model fetches"));
    }
}

impl ModelsEndpointClient for TestModelsEndpoint {
    fn has_command_auth(&self) -> bool {
        true
    }

    fn uses_codex_backend(&self) -> ModelsEndpointFuture<'_, bool> {
        Box::pin(async { false })
    }

    fn list_models<'a>(
        &'a self,
        _client_version: &'a str,
        _http_client_factory: HttpClientFactory,
    ) -> ModelsEndpointFuture<'a, CoreResult<(Vec<ModelInfo>, Option<String>)>> {
        Box::pin(async move {
            let fetch_index = self.fetch_count.fetch_add(1, Ordering::SeqCst);
            self.fetched.notify_one();
            if fetch_index == 0 {
                return Err(CodexErr::Io(std::io::Error::other("test failure")));
            }
            if fetch_index == 1 {
                self.release_second_fetch.notified().await;
            }
            Ok((Vec::new(), None))
        })
    }
}

#[tokio::test]
async fn refreshes_immediately_periodically_and_stops_when_dropped() {
    let codex_home = tempdir().expect("temp dir");
    let endpoint = TestModelsEndpoint::new();
    let models_manager: SharedModelsManager = Arc::new(OpenAiModelsManager::new(
        codex_home.path().to_path_buf(),
        endpoint.clone(),
        /*auth_manager*/ None,
    ));
    let (outgoing_tx, _outgoing_rx) = tokio::sync::mpsc::channel(8);
    let model_list_catalog = Arc::new(ModelListCatalog::new(
        models_manager,
        HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
        Arc::new(crate::outgoing_message::OutgoingMessageSender::new(
            outgoing_tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        )),
    ));
    let worker = spawn_with_interval(&model_list_catalog, Duration::from_millis(10));

    endpoint.wait_for_fetch_count(/*expected*/ 2).await;
    drop(worker);
    endpoint.release_second_fetch.notify_one();
    tokio::time::sleep(Duration::from_millis(30)).await;

    assert_eq!(endpoint.fetch_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn identical_remote_refreshes_emit_one_semantic_invalidation() {
    let mut model = codex_models_manager::bundled_models_response()
        .expect("bundled model catalog")
        .models
        .into_iter()
        .next()
        .expect("bundled model");
    model.description = Some("remote revision".to_string());
    let endpoint = Arc::new(FixedModelsEndpoint {
        model,
        fetch_count: AtomicUsize::new(0),
    });
    let models_manager: SharedModelsManager = Arc::new(OpenAiModelsManager::new_without_cache(
        endpoint.clone(),
        /*auth_manager*/ None,
    ));
    let (outgoing_tx, mut outgoing_rx) = tokio::sync::mpsc::channel(8);
    let model_list_catalog = ModelListCatalog::new(
        models_manager,
        HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
        Arc::new(crate::outgoing_message::OutgoingMessageSender::new(
            outgoing_tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        )),
    );

    model_list_catalog.refresh(RefreshStrategy::Online).await;
    tokio::time::timeout(Duration::from_secs(1), outgoing_rx.recv())
        .await
        .expect("changed catalog notification")
        .expect("outgoing notification");

    model_list_catalog.refresh(RefreshStrategy::Online).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), outgoing_rx.recv())
            .await
            .is_err(),
        "identical refresh emitted a duplicate invalidation"
    );
    assert_eq!(endpoint.fetch_count.load(Ordering::SeqCst), 2);
}
