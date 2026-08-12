use anyhow::Result;
use app_test_support::TestAppServer;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ModelWorkbenchPublication;
use codex_app_server_protocol::ModelWorkbenchPublicationStatus;
use codex_app_server_protocol::ModelWorkbenchReadParams;
use codex_app_server_protocol::ModelWorkbenchReadResponse;
use codex_app_server_protocol::ModelWorkbenchStoredEntry;
use codex_app_server_protocol::ModelWorkbenchUpsertParams;
use codex_app_server_protocol::ModelWorkbenchUpsertResponse;
use pretty_assertions::assert_eq;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;
use tempfile::TempDir;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::body_json;
use wiremock::matchers::method;
use wiremock::matchers::path;

const RUNTIME_PID: u32 = 4242;
const CAPABILITY_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

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
