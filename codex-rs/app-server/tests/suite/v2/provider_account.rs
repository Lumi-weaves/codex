use anyhow::Result;
use app_test_support::TestAppServer;
use codex_app_server_protocol::ProviderAccount;
use codex_app_server_protocol::ProviderAccountAddApiKeyResponse;
use codex_app_server_protocol::ProviderAccountCredentialKind;
use codex_app_server_protocol::ProviderAccountImportResponse;
use codex_app_server_protocol::ProviderAccountLoginCancelResponse;
use codex_app_server_protocol::ProviderAccountLoginStartResponse;
use codex_app_server_protocol::ProviderAccountLoginStatus;
use codex_app_server_protocol::ProviderAccountLoginStatusResponse;
use codex_app_server_protocol::ProviderAccountRemovalPreviewResponse;
use codex_app_server_protocol::ProviderAccountRemoveResponse;
use codex_app_server_protocol::ProviderAccountRenameResponse;
use codex_app_server_protocol::ProviderAccountReplaceApiKeyResponse;
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
                credential_kind: ProviderAccountCredentialKind::OAuth,
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
#[tokio::test]
async fn provider_account_api_key_add_returns_only_safe_account_state() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let codex_home = TempDir::new()?;
    let fixture_root = TempDir::new()?;
    let backend_path = fixture_root.path().join("fake-richcodex-model-backend");
    std::fs::write(&backend_path, FAKE_BACKEND)?;
    std::fs::set_permissions(&backend_path, std::fs::Permissions::from_mode(0o700))?;

    let backend_path = backend_path.display().to_string();
    let mut server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .without_auto_env()
        .with_env_overrides(&[("RICHCX_MODEL_BACKEND_PATH", Some(&backend_path))])
        .build_initialized_with_timeout(DEFAULT_TIMEOUT)
        .await?;
    let api_key = "sk-api-key-canary-must-not-return";
    let request_id = server
        .send_raw_request(
            "providerAccount/apiKey/add",
            Some(serde_json::json!({
                "providerId": "alibaba",
                "providerDisplayName": "Alibaba Model Studio",
                "apiBaseUrl": "https://dashscope.aliyuncs.com/compatible-mode/v1",
                "apiKey": api_key,
                "userLabel": "Alibaba Primary",
            })),
        )
        .await?;
    let response: ProviderAccountAddApiKeyResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(request_id)).await??;

    assert_eq!(
        response.account.credential_kind,
        ProviderAccountCredentialKind::ApiKey
    );
    assert_eq!(response.account.provider_id, "alibaba");
    assert_eq!(response.account.user_label, "Alibaba Primary");
    assert!(!serde_json::to_string(&response)?.contains(api_key));
    let recorded = std::fs::read_to_string(
        codex_home
            .path()
            .join("richcodex/model-backend/provider-api-key-add.json"),
    )?;
    let recorded: serde_json::Value = serde_json::from_str(&recorded)?;
    assert_eq!(recorded["providerId"], "alibaba");
    assert_eq!(
        recorded["apiBaseUrl"],
        "https://dashscope.aliyuncs.com/compatible-mode/v1"
    );
    let replacement = "sk-replacement-canary-must-not-return";
    let replace_id = server
        .send_raw_request(
            "providerAccount/apiKey/replace",
            Some(serde_json::json!({
                "accountId": response.account.id,
                "expectedRevision": response.desired_state_revision,
                "apiKey": replacement,
            })),
        )
        .await?;
    let replaced: ProviderAccountReplaceApiKeyResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(replace_id)).await??;
    assert_eq!(replaced.account.id, response.account.id);
    assert!(!serde_json::to_string(&replaced)?.contains(replacement));

    let rename_id = server
        .send_raw_request(
            "providerAccount/rename",
            Some(serde_json::json!({
                "accountId": response.account.id,
                "expectedRevision": replaced.desired_state_revision,
                "userLabel": "Alibaba Renamed",
            })),
        )
        .await?;
    let renamed: ProviderAccountRenameResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(rename_id)).await??;
    assert_eq!(
        renamed.account,
        ProviderAccount {
            user_label: "Alibaba Renamed".to_string(),
            ..replaced.account.clone()
        }
    );

    let preview_id = server
        .send_raw_request(
            "providerAccount/removalPreview",
            Some(serde_json::json!({ "accountId": response.account.id })),
        )
        .await?;
    let preview: ProviderAccountRemovalPreviewResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(preview_id)).await??;
    assert!(preview.can_remove);
    assert!(preview.affected_targets.is_empty());

    let remove_id = server
        .send_raw_request(
            "providerAccount/remove",
            Some(serde_json::json!({
                "accountId": response.account.id,
                "expectedRevision": preview.desired_state_revision,
            })),
        )
        .await?;
    let removed: ProviderAccountRemoveResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(remove_id)).await??;
    assert_eq!(removed.account.id, response.account.id);
    assert!(server.shutdown_gracefully().await?.success());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn provider_account_device_login_exposes_only_safe_lifecycle_state() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let codex_home = TempDir::new()?;
    let fixture_root = TempDir::new()?;
    let backend_path = fixture_root.path().join("fake-richcodex-model-backend");
    std::fs::write(&backend_path, FAKE_BACKEND)?;
    std::fs::set_permissions(&backend_path, std::fs::Permissions::from_mode(0o700))?;

    let backend_path = backend_path.display().to_string();
    let mut server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .without_auto_env()
        .with_env_overrides(&[("RICHCX_MODEL_BACKEND_PATH", Some(&backend_path))])
        .build_initialized_with_timeout(DEFAULT_TIMEOUT)
        .await?;

    let start_id = server
        .send_raw_request(
            "providerAccount/login/start",
            Some(serde_json::json!({
                "userLabel": "Third Codex",
                "mode": "deviceCode",
            })),
        )
        .await?;
    let started: ProviderAccountLoginStartResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(start_id)).await??;
    assert_eq!(
        started.login.status,
        ProviderAccountLoginStatus::AwaitingUser
    );
    assert_eq!(
        started.login.verification_url.as_deref(),
        Some("https://auth.openai.com/codex/device")
    );
    assert_eq!(started.login.user_code.as_deref(), Some("SAFE-CODE"));

    let status_id = server
        .send_raw_request(
            "providerAccount/login/status",
            Some(serde_json::json!({ "loginId": started.login.login_id.clone() })),
        )
        .await?;
    let completed: ProviderAccountLoginStatusResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(status_id)).await??;
    assert_eq!(
        completed.login.status,
        ProviderAccountLoginStatus::Completed
    );
    assert_eq!(
        completed
            .login
            .account
            .as_ref()
            .map(|account| account.user_label.as_str()),
        Some("Third Codex")
    );

    let cancel_id = server
        .send_raw_request(
            "providerAccount/login/cancel",
            Some(serde_json::json!({ "loginId": completed.login.login_id.clone() })),
        )
        .await?;
    let cancelled: ProviderAccountLoginCancelResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(cancel_id)).await??;
    assert_eq!(
        cancelled.login.status,
        ProviderAccountLoginStatus::Cancelled
    );
    let serialized = serde_json::to_string(&(started, completed, cancelled))?;
    assert!(!serialized.contains("private-device-auth-id"));
    assert!(!serialized.contains("private-refresh-token"));

    assert!(server.shutdown_gracefully().await?.success());
    Ok(())
}

#[cfg(unix)]
const FAKE_BACKEND: &str = r#"#!/bin/sh
set -eu
test "$1" = "--state-root"
state_root=$2
mkdir -p "$state_root"
printf '%s\n' '{"type":"ready","protocolVersion":11,"instanceId":"fixture-1","desiredStateRevision":1,"catalogRevision":1,"dataPlanePort":48767,"kernel":{"sourceRepository":"https://github.com/lidge-jun/opencodex","sourceCommit":"cbbfdd8773e68a5dc2391ddeb32f33a225373c1a","contentDigest":"sha256:65672062788957661574aafd6d32d571d0a33afb0575f6a12e19801d72874b78","selectionDigest":"sha256:5e7c03c78ba23105858523d923f000bfcb0ba6f352395fd5f72cdf823c49c97a","compositionVersion":4},"providers":[],"models":[]}'
while IFS= read -r line; do
  request_id=$(printf '%s\n' "$line" | sed -n 's/.*"requestId":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"providerAccountImport"'*)
      printf '%s\n' "$line" > "$state_root/provider-account-import.json"
      printf '{"type":"providerAccountImportResult","requestId":"%s","desiredStateRevision":2,"catalogRevision":3,"account":{"id":"local-secondary","providerId":"openai","userLabel":"Secondary","credentialKind":"oauth","status":"verificationRequired","addedAt":123}}\n' "$request_id"
      ;;
    *'"type":"providerAccountAddApiKey"'*'sk-api-key-canary-must-not-return'*)
      printf '%s\n' "$line" > "$state_root/provider-api-key-add.json"
      printf '{"type":"providerAccountAddApiKeyResult","requestId":"%s","desiredStateRevision":2,"catalogRevision":2,"account":{"id":"local-api-key","providerId":"alibaba","userLabel":"Alibaba Primary","credentialKind":"apiKey","status":"verificationRequired","addedAt":124}}\n' "$request_id"
      ;;
    *'"type":"providerAccountReplaceApiKey"'*'sk-replacement-canary-must-not-return'*)
      printf '{"type":"providerAccountReplaceApiKeyResult","requestId":"%s","desiredStateRevision":3,"catalogRevision":3,"account":{"id":"local-api-key","providerId":"alibaba","userLabel":"Alibaba Primary","credentialKind":"apiKey","status":"verificationRequired","addedAt":124}}\n' "$request_id"
      ;;
    *'"type":"providerAccountRename"'*)
      printf '{"type":"providerAccountRenameResult","requestId":"%s","desiredStateRevision":4,"catalogRevision":4,"account":{"id":"local-api-key","providerId":"alibaba","userLabel":"Alibaba Renamed","credentialKind":"apiKey","status":"verificationRequired","addedAt":124}}\n' "$request_id"
      ;;
    *'"type":"providerAccountRemovalPreview"'*)
      printf '{"type":"providerAccountRemovalPreviewResult","requestId":"%s","desiredStateRevision":4,"catalogRevision":4,"account":{"id":"local-api-key","providerId":"alibaba","userLabel":"Alibaba Renamed","credentialKind":"apiKey","status":"verificationRequired","addedAt":124},"affectedTargets":[],"canRemove":true}\n' "$request_id"
      ;;
    *'"type":"providerAccountRemove"'*)
      printf '{"type":"providerAccountRemoveResult","requestId":"%s","desiredStateRevision":5,"catalogRevision":5,"account":{"id":"local-api-key","providerId":"alibaba","userLabel":"Alibaba Renamed","credentialKind":"apiKey","status":"verificationRequired","addedAt":124}}\n' "$request_id"
      ;;
    *'"type":"providerAccountLoginStart"'*)
      printf '{"type":"providerAccountLoginStartResult","requestId":"%s","loginId":"login-safe-handle","status":"awaitingUser","verificationUrl":"https://auth.openai.com/codex/device","userCode":"SAFE-CODE","expiresAt":2000,"failure":null,"account":null,"desiredStateRevision":1,"catalogRevision":1}\n' "$request_id"
      ;;
    *'"type":"providerAccountLoginStatus"'*)
      printf '{"type":"providerAccountLoginStatusResult","requestId":"%s","loginId":"login-safe-handle","status":"completed","verificationUrl":null,"userCode":null,"expiresAt":2000,"failure":null,"account":{"id":"local-third","providerId":"openai","userLabel":"Third Codex","credentialKind":"oauth","status":"verificationRequired","addedAt":125},"desiredStateRevision":2,"catalogRevision":2}\n' "$request_id"
      ;;
    *'"type":"providerAccountLoginCancel"'*)
      printf '{"type":"providerAccountLoginCancelResult","requestId":"%s","loginId":"login-safe-handle","status":"cancelled","verificationUrl":null,"userCode":null,"expiresAt":2000,"failure":null,"account":null,"desiredStateRevision":2,"catalogRevision":2}\n' "$request_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdownComplete","requestId":"%s"}\n' "$request_id"
      exit 0
      ;;
  esac
done
"#;
