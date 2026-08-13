use anyhow::Result;
use app_test_support::TestAppServer;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ModelListParams;
use codex_app_server_protocol::ModelListResponse;
use codex_app_server_protocol::ModelListUpdatedNotification;
use codex_app_server_protocol::ModelWorkbenchPublication;
use codex_app_server_protocol::ModelWorkbenchPublicationStatus;
use codex_app_server_protocol::ModelWorkbenchReadParams;
use codex_app_server_protocol::ModelWorkbenchReadResponse;
use codex_app_server_protocol::ModelWorkbenchRetireParams;
use codex_app_server_protocol::ModelWorkbenchRetireResponse;
use codex_app_server_protocol::ModelWorkbenchStoredEntry;
use codex_app_server_protocol::ModelWorkbenchUpsertParams;
use codex_app_server_protocol::ModelWorkbenchUpsertResponse;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ModelsResponse;
use pretty_assertions::assert_eq;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;
use tempfile::TempDir;
use tokio::time::Duration;
use tokio::time::timeout;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::body_json;
use wiremock::matchers::method;
use wiremock::matchers::path;

const RUNTIME_PID: u32 = 4242;
const CAPABILITY_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const WORKBENCH_MODEL_TAG: &str = "provider/workbench-model";

#[tokio::test]
async fn model_workbench_requests_use_the_local_capability_bridge() -> Result<()> {
    let backend = MockServer::start().await;
    let opencodex_home = TempDir::new()?;
    write_runtime_record(&opencodex_home, &backend)?;

    Mock::given(method("GET"))
        .and(path("/healthz"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "service": "opencodex",
            "pid": RUNTIME_PID,
            "port": backend.address().port(),
        })))
        .expect(2)
        .mount(&backend)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/model-workbench/entries"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "revision": 7,
            "entries": [{
                "modelTag": "gpt-5.5",
                "displayName": "My Codex",
            }],
        })))
        .expect(1)
        .mount(&backend)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/model-workbench/entries"))
        .and(body_json(json!({
            "displayName": "Fast Codex",
            "modelTag": "gpt-5.5-fast",
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({
            "revision": 8,
            "changed": true,
            "entry": {
                "modelTag": "gpt-5.5-fast",
                "displayName": "Fast Codex",
                "retired": false,
            },
            "publication": {
                "registryRevision": 8,
                "catalogRevision": 7,
                "modelsCacheRevision": 7,
                "synchronized": false,
                "catalogRefresh": {"status": "skipped", "reason": "busy"},
            },
        })))
        .expect(1)
        .mount(&backend)
        .await;

    let opencodex_home = opencodex_home.path().to_string_lossy().into_owned();
    let mut app_server = TestAppServer::builder()
        .without_auto_env()
        .with_env_overrides(&[("OPENCODEX_HOME", Some(opencodex_home.as_str()))])
        .build_initialized()
        .await?;

    let read: ModelWorkbenchReadResponse = app_server
        .request(|request_id| ClientRequest::ModelWorkbenchRead {
            request_id,
            params: ModelWorkbenchReadParams::default(),
        })
        .await?;
    assert_eq!(read.revision, 7);
    assert_eq!(read.entries[0].model_tag, "gpt-5.5");
    assert_eq!(read.entries[0].display_name, "My Codex");

    let upsert: ModelWorkbenchUpsertResponse = app_server
        .request(|request_id| ClientRequest::ModelWorkbenchUpsert {
            request_id,
            params: ModelWorkbenchUpsertParams {
                model_tag: "gpt-5.5-fast".to_string(),
                display_name: "Fast Codex".to_string(),
                expected_revision: Some(7),
            },
        })
        .await?;
    assert_eq!(upsert.revision, 8);
    assert!(upsert.changed);
    assert_eq!(
        upsert.entry,
        ModelWorkbenchStoredEntry {
            model_tag: "gpt-5.5-fast".to_string(),
            display_name: "Fast Codex".to_string(),
            retired: false,
        }
    );
    assert_eq!(
        upsert.publication,
        ModelWorkbenchPublication {
            registry_revision: 8,
            catalog_revision: Some(7),
            models_cache_revision: Some(7),
            synchronized: false,
            status: ModelWorkbenchPublicationStatus::Pending,
        }
    );

    let requests = backend
        .received_requests()
        .await
        .expect("request recording is enabled");
    let read_request = requests
        .iter()
        .find(|request| {
            request.method.as_str() == "GET" && request.url.path() == "/api/model-workbench/entries"
        })
        .expect("read request");
    assert_capability_headers(read_request, empty_body_sha256(), None);

    let upsert_request = requests
        .iter()
        .find(|request| request.method.as_str() == "PUT")
        .expect("upsert request");
    let body_hash = base64_url_sha256(&upsert_request.body);
    assert_capability_headers(upsert_request, body_hash, Some("\"7\""));
    Ok(())
}

#[tokio::test]
async fn model_workbench_retire_reloads_the_authoritative_catalog() -> Result<()> {
    let backend = MockServer::start().await;
    let opencodex_home = TempDir::new()?;
    let codex_home = TempDir::new()?;
    let catalog_path = codex_home.path().join("catalog.json");
    write_workbench_catalog(&catalog_path, ModelVisibility::List)?;
    std::fs::write(
        codex_home.path().join("config.toml"),
        format!(
            "model_catalog_json = {}\n",
            serde_json::to_string(&catalog_path)?
        ),
    )?;
    write_runtime_record(&opencodex_home, &backend)?;

    Mock::given(method("GET"))
        .and(path("/healthz"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "service": "opencodex",
            "pid": RUNTIME_PID,
            "port": backend.address().port(),
        })))
        .mount(&backend)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/model-workbench/entries/retire"))
        .and(body_json(json!({"modelTag": WORKBENCH_MODEL_TAG})))
        .respond_with({
            let catalog_path = catalog_path.clone();
            move |_request: &wiremock::Request| {
                write_workbench_catalog(&catalog_path, ModelVisibility::Hide)
                    .expect("write retired catalog");
                ResponseTemplate::new(200).set_body_json(json!({
                    "revision": 2,
                    "changed": true,
                    "entry": {
                        "modelTag": WORKBENCH_MODEL_TAG,
                        "displayName": "Workbench Model",
                        "retired": true,
                    },
                    "publication": {
                        "registryRevision": 2,
                        "catalogRevision": 2,
                        "modelsCacheRevision": 2,
                        "synchronized": true,
                        "catalogRefresh": {"status": "refreshed"},
                    },
                }))
            }
        })
        .expect(1)
        .mount(&backend)
        .await;

    let opencodex_home = opencodex_home.path().to_string_lossy().into_owned();
    let mut app_server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .without_auto_env()
        .with_env_overrides(&[("OPENCODEX_HOME", Some(opencodex_home.as_str()))])
        .build_initialized()
        .await?;

    let before = request_model_list(&mut app_server).await?;
    assert_eq!(model_hidden(&before, WORKBENCH_MODEL_TAG), Some(false));
    app_server.clear_message_buffer();

    let retired: ModelWorkbenchRetireResponse = app_server
        .request(|request_id| ClientRequest::ModelWorkbenchRetire {
            request_id,
            params: ModelWorkbenchRetireParams {
                model_tag: WORKBENCH_MODEL_TAG.to_string(),
                expected_revision: Some(1),
            },
        })
        .await?;
    assert_eq!(
        retired.publication.status,
        ModelWorkbenchPublicationStatus::Synchronized
    );

    let updated: ModelListUpdatedNotification = timeout(
        Duration::from_secs(10),
        app_server.read_notification("model/list/updated"),
    )
    .await??;
    let after = request_model_list(&mut app_server).await?;
    assert_eq!(updated.revision, after.revision);
    assert_eq!(model_hidden(&after, WORKBENCH_MODEL_TAG), Some(true));
    Ok(())
}

async fn request_model_list(app_server: &mut TestAppServer) -> Result<ModelListResponse> {
    app_server
        .request(|request_id| ClientRequest::ModelList {
            request_id,
            params: ModelListParams {
                cursor: None,
                limit: None,
                include_hidden: Some(true),
            },
        })
        .await
}

fn model_hidden(response: &ModelListResponse, model_tag: &str) -> Option<bool> {
    response
        .data
        .iter()
        .find(|model| model.model == model_tag)
        .map(|model| model.hidden)
}

fn write_workbench_catalog(path: &std::path::Path, visibility: ModelVisibility) -> Result<()> {
    let mut model = codex_models_manager::bundled_models_response()?
        .models
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("bundled catalog is empty"))?;
    model.slug = WORKBENCH_MODEL_TAG.to_string();
    model.display_name = "Workbench Model".to_string();
    model.visibility = visibility;
    std::fs::write(
        path,
        serde_json::to_vec(&ModelsResponse {
            models: vec![model],
        })?,
    )?;
    Ok(())
}

fn write_runtime_record(home: &TempDir, backend: &MockServer) -> Result<()> {
    let path = home.path().join("runtime-port.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({
            "pid": RUNTIME_PID,
            "port": backend.address().port(),
            "hostname": "127.0.0.1",
            "modelWorkbenchCapabilitySecret": CAPABILITY_SECRET,
        }))?,
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn assert_capability_headers(
    request: &wiremock::Request,
    expected_body_hash: String,
    expected_revision: Option<&str>,
) {
    let expected_pid = RUNTIME_PID.to_string();
    assert_eq!(
        request
            .headers
            .get("x-opencodex-workbench-expected-pid")
            .and_then(|value| value.to_str().ok()),
        Some(expected_pid.as_str()),
    );
    assert_eq!(
        request
            .headers
            .get("x-opencodex-workbench-content-sha256")
            .and_then(|value| value.to_str().ok()),
        Some(expected_body_hash.as_str()),
    );
    assert!(request.headers.contains_key("x-opencodex-workbench-nonce"));
    assert!(
        request
            .headers
            .contains_key("x-opencodex-workbench-expires-at")
    );
    assert!(
        request
            .headers
            .contains_key("x-opencodex-workbench-capability")
    );
    assert_eq!(
        request
            .headers
            .get("if-match")
            .and_then(|value| value.to_str().ok()),
        expected_revision,
    );
    assert!(request.headers.get("authorization").is_none());
}

fn empty_body_sha256() -> String {
    base64_url_sha256(&[])
}

fn base64_url_sha256(body: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(body))
}
