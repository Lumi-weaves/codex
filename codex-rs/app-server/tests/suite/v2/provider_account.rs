use anyhow::Result;
use app_test_support::TestAppServer;
use codex_app_server_protocol::ProviderAccount;
use codex_app_server_protocol::ProviderAccountImportResponse;
use codex_app_server_protocol::ProviderAccountStatus;
use codex_app_server_protocol::RequestId;
use pretty_assertions::assert_eq;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const INTERNAL_ERROR_CODE: i64 = -32603;

#[tokio::test]
async fn provider_account_requests_report_a_static_error_without_a_backend() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .without_auto_env()
        .with_env_overrides(&[("RICHCX_MODEL_BACKEND_PATH", None)])
        .build_initialized_with_timeout(DEFAULT_TIMEOUT)
        .await?;

    let request_id = server
        .send_raw_request(
            "providerAccount/list",
            Some(serde_json::json!({ "limit": 20 })),
        )
        .await?;
    let error = timeout(
        DEFAULT_TIMEOUT,
        server.read_stream_until_error_message(RequestId::Integer(request_id)),
    )
    .await??;

    assert_eq!(error.error.code, INTERNAL_ERROR_CODE);
    assert_eq!(
        error.error.message,
        "RichCodex provider account backend is unavailable"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn provider_account_import_forwards_but_does_not_open_the_selected_path() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let codex_home = TempDir::new()?;
    let fixture_root = TempDir::new()?;
    let backend_path = fixture_root.path().join("fake-richcodex-model-backend");
    std::fs::write(&backend_path, FAKE_BACKEND)?;
    std::fs::set_permissions(&backend_path, std::fs::Permissions::from_mode(0o700))?;

    // A read-open on this FIFO would block forever because there is no writer.
    // The fake backend only records the path carried in its JSONL request.
    let auth_json_path = fixture_root.path().join("selected-auth.json");
    let status = std::process::Command::new("mkfifo")
        .arg(&auth_json_path)
        .status()?;
    assert!(status.success());

    let backend_path = backend_path.display().to_string();
    let mut server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .without_auto_env()
        .with_env_overrides(&[("RICHCX_MODEL_BACKEND_PATH", Some(&backend_path))])
        .build_initialized_with_timeout(DEFAULT_TIMEOUT)
        .await?;

    let request_id = server
        .send_raw_request(
            "providerAccount/import",
            Some(serde_json::json!({
                "authJsonPath": auth_json_path,
                "userLabel": "Secondary",
            })),
        )
        .await?;
    let response: ProviderAccountImportResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(request_id)).await??;

    assert_eq!(
        response,
        ProviderAccountImportResponse {
            account: ProviderAccount {
                id: "local-secondary".to_string(),
                provider_id: "openai".to_string(),
                user_label: "Secondary".to_string(),
                status: ProviderAccountStatus::VerificationRequired,
                added_at: 123,
            },
            desired_state_revision: "2".to_string(),
            catalog_revision: "3".to_string(),
        }
    );
    let recorded = std::fs::read_to_string(
        codex_home
            .path()
            .join("richcodex/model-backend/provider-account-import.json"),
    )?;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&recorded)?,
        serde_json::json!({
            "type": "providerAccountImport",
            "requestId": "app-server-1",
            "authJsonPath": auth_json_path,
            "userLabel": "Secondary",
        })
    );
    assert!(server.shutdown_gracefully().await?.success());
    Ok(())
}

#[cfg(unix)]
const FAKE_BACKEND: &str = r#"#!/bin/sh
set -eu
test "$1" = "--state-root"
state_root=$2
mkdir -p "$state_root"
printf '%s\n' '{"type":"ready","protocolVersion":4,"instanceId":"fixture-1","desiredStateRevision":1,"catalogRevision":1,"dataPlanePort":48767,"kernel":{"sourceRepository":"https://github.com/lidge-jun/opencodex","sourceCommit":"cbbfdd8773e68a5dc2391ddeb32f33a225373c1a","contentDigest":"sha256:65672062788957661574aafd6d32d571d0a33afb0575f6a12e19801d72874b78","selectionDigest":"sha256:fed70f36cf8a71e495e647db03480d5f5213fdc2760c231e6d7e8a414d84edbf","compositionVersion":3},"providers":[],"models":[]}'
while IFS= read -r line; do
  request_id=$(printf '%s\n' "$line" | sed -n 's/.*"requestId":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"providerAccountImport"'*)
      printf '%s\n' "$line" > "$state_root/provider-account-import.json"
      printf '{"type":"providerAccountImportResult","requestId":"%s","desiredStateRevision":2,"catalogRevision":3,"account":{"id":"local-secondary","providerId":"openai","userLabel":"Secondary","status":"verificationRequired","addedAt":123}}\n' "$request_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdownComplete","requestId":"%s"}\n' "$request_id"
      exit 0
      ;;
  esac
done
"#;
