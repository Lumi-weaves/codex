use anyhow::Result;
use app_test_support::TestAppServer;
use codex_app_server_protocol::ModelListResponse;
use codex_app_server_protocol::ModelRoute;
use codex_app_server_protocol::ModelRouteCreateResponse;
use codex_app_server_protocol::ModelRouteReadResponse;
use codex_app_server_protocol::ModelRouteRetireResponse;
use codex_app_server_protocol::ModelRouteSetTargetsResponse;
use codex_app_server_protocol::ModelRouteTarget;
use codex_app_server_protocol::ModelRouteTargetStatus;
use codex_app_server_protocol::RequestId;
use pretty_assertions::assert_eq;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const INVALID_PARAMS_ERROR_CODE: i64 = -32602;
const INTERNAL_ERROR_CODE: i64 = -32603;

#[tokio::test]
async fn model_route_requests_report_a_static_error_without_a_backend() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .without_auto_env()
        .with_env_overrides(&[("RICHCX_MODEL_BACKEND_PATH", None)])
        .build_initialized_with_timeout(DEFAULT_TIMEOUT)
        .await?;

    let request_id = server
        .send_raw_request("modelRoute/read", Some(serde_json::json!({})))
        .await?;
    let error = timeout(
        DEFAULT_TIMEOUT,
        server.read_stream_until_error_message(RequestId::Integer(request_id)),
    )
    .await??;

    assert_eq!(error.error.code, INTERNAL_ERROR_CODE);
    assert_eq!(
        error.error.message,
        "RichCodex model-route backend is unavailable"
    );
    Ok(())
}

#[tokio::test]
async fn model_route_rejects_a_forged_revision_before_contacting_the_backend() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .without_auto_env()
        .with_env_overrides(&[("RICHCX_MODEL_BACKEND_PATH", None)])
        .build_initialized_with_timeout(DEFAULT_TIMEOUT)
        .await?;

    let request_id = server
        .send_raw_request(
            "modelRoute/retire",
            Some(serde_json::json!({
                "expectedRevision": "01",
                "modelTag": "gpt-primary",
            })),
        )
        .await?;
    let error = timeout(
        DEFAULT_TIMEOUT,
        server.read_stream_until_error_message(RequestId::Integer(request_id)),
    )
    .await??;

    assert_eq!(error.error.code, INVALID_PARAMS_ERROR_CODE);
    assert_eq!(
        error.error.message,
        "expectedRevision must be an opaque decimal revision returned by RichCodex"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn model_route_read_create_and_retire_round_trip_through_the_backend() -> Result<()> {
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

    let read_id = server
        .send_raw_request("modelRoute/read", Some(serde_json::json!({})))
        .await?;
    let read: ModelRouteReadResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(read_id)).await??;
    assert_eq!(
        read,
        ModelRouteReadResponse {
            data: vec![route(
                "gpt-existing",
                "GPT Existing",
                false,
                "gpt-5.6-luna",
                "target-existing",
                "gpt-existing",
            )],
            desired_state_revision: "1".to_string(),
            catalog_revision: "1".to_string(),
        }
    );
    let startup_model_list_id = server
        .send_raw_request(
            "model/list",
            Some(serde_json::json!({ "includeHidden": false })),
        )
        .await?;
    let startup_model_list: ModelListResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(startup_model_list_id)).await??;
    assert!(
        startup_model_list
            .data
            .iter()
            .any(|model| { model.model == "gpt-existing" && model.display_name == "GPT Existing" })
    );

    let chained_id = server
        .send_raw_request(
            "modelRoute/create",
            Some(serde_json::json!({
                "expectedRevision": "1",
                "modelTag": "gpt-chained",
                "displayName": "GPT Chained",
                "semanticModel": "gpt-existing",
                "providerId": "openai",
                "accountId": "account-local",
                "upstreamModelId": "gpt-chained",
            })),
        )
        .await?;
    let chained = timeout(
        DEFAULT_TIMEOUT,
        server.read_stream_until_error_message(RequestId::Integer(chained_id)),
    )
    .await??;
    assert_eq!(chained.error.code, INVALID_PARAMS_ERROR_CODE);
    assert_eq!(
        chained.error.message,
        "semanticModel must identify a native model, not another RichCodex route"
    );

    let create_id = server
        .send_raw_request(
            "modelRoute/create",
            Some(serde_json::json!({
                "expectedRevision": "1",
                "modelTag": "gpt-primary",
                "displayName": "GPT Primary",
                "semanticModel": "gpt-5.6-luna",
                "providerId": "openai",
                "accountId": "account-local",
                "upstreamModelId": "gpt-primary-2026-08-13",
            })),
        )
        .await?;
    let created: ModelRouteCreateResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(create_id)).await??;
    assert_eq!(
        created,
        ModelRouteCreateResponse {
            route: route(
                "gpt-primary",
                "GPT Primary",
                false,
                "gpt-5.6-luna",
                "target-created",
                "gpt-primary-2026-08-13",
            ),
            desired_state_revision: "2".to_string(),
            catalog_revision: "2".to_string(),
        }
    );
    let model_list_id = server
        .send_raw_request(
            "model/list",
            Some(serde_json::json!({ "includeHidden": false })),
        )
        .await?;
    let model_list: ModelListResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(model_list_id)).await??;
    assert!(
        model_list
            .data
            .iter()
            .any(|model| { model.model == "gpt-primary" && model.display_name == "GPT Primary" })
    );

    let set_targets_id = server
        .send_raw_request(
            "modelRoute/targets/set",
            Some(serde_json::json!({
                "expectedRevision": "2",
                "modelTag": "gpt-primary",
                "targets": [
                    {
                        "id": null,
                        "providerId": "openai",
                        "accountId": "account-backup",
                        "upstreamModelId": "gpt-primary-backup",
                    },
                    {
                        "id": "target-created",
                        "providerId": "openai",
                        "accountId": "account-local",
                        "upstreamModelId": "gpt-primary-revised",
                    }
                ],
            })),
        )
        .await?;
    let updated: ModelRouteSetTargetsResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(set_targets_id)).await??;
    assert_eq!(updated.desired_state_revision, "3");
    assert_eq!(updated.route.targets.len(), 2);
    assert_eq!(updated.route.targets[0].id, "target-backup");
    assert_eq!(updated.route.targets[0].priority, 0);
    assert_eq!(updated.route.targets[1].id, "target-created");
    assert_eq!(updated.route.targets[1].priority, 1);
    let mut expected_retired_route = updated.route.clone();
    expected_retired_route.retired = true;

    let conflict_id = server
        .send_raw_request(
            "modelRoute/create",
            Some(serde_json::json!({
                "expectedRevision": "0",
                "modelTag": "gpt-conflict",
                "displayName": "GPT Conflict",
                "semanticModel": "gpt-5.6-luna",
                "providerId": "openai",
                "accountId": "account-local",
                "upstreamModelId": "gpt-conflict",
            })),
        )
        .await?;
    let conflict = timeout(
        DEFAULT_TIMEOUT,
        server.read_stream_until_error_message(RequestId::Integer(conflict_id)),
    )
    .await??;
    assert_eq!(conflict.error.code, INVALID_PARAMS_ERROR_CODE);
    assert_eq!(
        conflict.error.message,
        "model plane revision does not match"
    );

    let retire_id = server
        .send_raw_request(
            "modelRoute/retire",
            Some(serde_json::json!({
                "expectedRevision": "3",
                "modelTag": "gpt-primary",
            })),
        )
        .await?;
    let retired: ModelRouteRetireResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(retire_id)).await??;
    assert_eq!(
        retired,
        ModelRouteRetireResponse {
            route: expected_retired_route,
            desired_state_revision: "4".to_string(),
            catalog_revision: "4".to_string(),
        }
    );
    let model_list_id = server
        .send_raw_request(
            "model/list",
            Some(serde_json::json!({ "includeHidden": false })),
        )
        .await?;
    let model_list: ModelListResponse =
        timeout(DEFAULT_TIMEOUT, server.read_response(model_list_id)).await??;
    assert!(
        !model_list
            .data
            .iter()
            .any(|model| model.model == "gpt-primary")
    );

    let recorded = std::fs::read_to_string(
        codex_home
            .path()
            .join("richcodex/model-backend/model-route-create.json"),
    )?;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&recorded)?,
        serde_json::json!({
            "type": "modelRouteCreate",
            "requestId": "app-server-4",
            "expectedRevision": 1,
            "modelTag": "gpt-primary",
            "displayName": "GPT Primary",
            "semanticModel": "gpt-5.6-luna",
            "providerId": "openai",
            "accountId": "account-local",
            "upstreamModelId": "gpt-primary-2026-08-13",
        })
    );
    let recorded_targets = std::fs::read_to_string(
        codex_home
            .path()
            .join("richcodex/model-backend/model-route-set-targets.json"),
    )?;
    let recorded_targets: serde_json::Value = serde_json::from_str(&recorded_targets)?;
    assert_eq!(recorded_targets["type"], "modelRouteSetTargets");
    assert_eq!(recorded_targets["expectedRevision"], 2);
    assert_eq!(recorded_targets["modelTag"], "gpt-primary");
    assert_eq!(recorded_targets["targets"].as_array().unwrap().len(), 2);
    assert!(server.shutdown_gracefully().await?.success());
    Ok(())
}

#[cfg(unix)]
fn route(
    model_tag: &str,
    display_name: &str,
    retired: bool,
    semantic_model: &str,
    target_id: &str,
    upstream_model_id: &str,
) -> ModelRoute {
    ModelRoute {
        model_tag: model_tag.to_string(),
        display_name: display_name.to_string(),
        retired,
        semantic_model: semantic_model.to_string(),
        targets: vec![ModelRouteTarget {
            id: target_id.to_string(),
            provider_id: "openai".to_string(),
            account_id: "account-local".to_string(),
            upstream_model_id: upstream_model_id.to_string(),
            priority: 0,
            status: ModelRouteTargetStatus::Unverified,
        }],
    }
}

#[cfg(unix)]
const FAKE_BACKEND: &str = r#"#!/bin/sh
set -eu
test "$1" = "--state-root"
state_root=$2
mkdir -p "$state_root"
route_state=0
printf '%s\n' '{"type":"ready","protocolVersion":6,"instanceId":"fixture-routes","desiredStateRevision":1,"catalogRevision":1,"dataPlanePort":48767,"kernel":{"sourceRepository":"https://github.com/lidge-jun/opencodex","sourceCommit":"cbbfdd8773e68a5dc2391ddeb32f33a225373c1a","contentDigest":"sha256:65672062788957661574aafd6d32d571d0a33afb0575f6a12e19801d72874b78","selectionDigest":"sha256:fed70f36cf8a71e495e647db03480d5f5213fdc2760c231e6d7e8a414d84edbf","compositionVersion":3},"providers":[],"models":[{"modelTag":"gpt-existing","displayName":"GPT Existing","retired":false,"semanticModel":"gpt-5.6-luna","targets":[{"id":"target-existing","providerId":"openai","accountId":"account-local","upstreamModelId":"gpt-existing","priority":0,"status":"unverified"}]}]}'
while IFS= read -r line; do
  request_id=$(printf '%s\n' "$line" | sed -n 's/.*"requestId":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"modelRouteRead"'*)
      if [ "$route_state" -eq 0 ]; then
        printf '{"type":"modelRouteReadResult","requestId":"%s","desiredStateRevision":1,"catalogRevision":1,"data":[{"modelTag":"gpt-existing","displayName":"GPT Existing","retired":false,"semanticModel":"gpt-5.6-luna","targets":[{"id":"target-existing","providerId":"openai","accountId":"account-local","upstreamModelId":"gpt-existing","priority":0,"status":"unverified"}]}]}\n' "$request_id"
      elif [ "$route_state" -eq 1 ]; then
        printf '{"type":"modelRouteReadResult","requestId":"%s","desiredStateRevision":2,"catalogRevision":2,"data":[{"modelTag":"gpt-existing","displayName":"GPT Existing","retired":false,"semanticModel":"gpt-5.6-luna","targets":[{"id":"target-existing","providerId":"openai","accountId":"account-local","upstreamModelId":"gpt-existing","priority":0,"status":"unverified"}]},{"modelTag":"gpt-primary","displayName":"GPT Primary","retired":false,"semanticModel":"gpt-5.6-luna","targets":[{"id":"target-created","providerId":"openai","accountId":"account-local","upstreamModelId":"gpt-primary-2026-08-13","priority":0,"status":"unverified"}]}]}\n' "$request_id"
      elif [ "$route_state" -eq 2 ]; then
        printf '{"type":"modelRouteReadResult","requestId":"%s","desiredStateRevision":3,"catalogRevision":3,"data":[{"modelTag":"gpt-existing","displayName":"GPT Existing","retired":false,"semanticModel":"gpt-5.6-luna","targets":[{"id":"target-existing","providerId":"openai","accountId":"account-local","upstreamModelId":"gpt-existing","priority":0,"status":"unverified"}]},{"modelTag":"gpt-primary","displayName":"GPT Primary","retired":false,"semanticModel":"gpt-5.6-luna","targets":[{"id":"target-backup","providerId":"openai","accountId":"account-backup","upstreamModelId":"gpt-primary-backup","priority":0,"status":"unverified"},{"id":"target-created","providerId":"openai","accountId":"account-local","upstreamModelId":"gpt-primary-revised","priority":1,"status":"unverified"}]}]}\n' "$request_id"
      else
        printf '{"type":"modelRouteReadResult","requestId":"%s","desiredStateRevision":4,"catalogRevision":4,"data":[{"modelTag":"gpt-existing","displayName":"GPT Existing","retired":false,"semanticModel":"gpt-5.6-luna","targets":[{"id":"target-existing","providerId":"openai","accountId":"account-local","upstreamModelId":"gpt-existing","priority":0,"status":"unverified"}]},{"modelTag":"gpt-primary","displayName":"GPT Primary","retired":true,"semanticModel":"gpt-5.6-luna","targets":[{"id":"target-backup","providerId":"openai","accountId":"account-backup","upstreamModelId":"gpt-primary-backup","priority":0,"status":"unverified"},{"id":"target-created","providerId":"openai","accountId":"account-local","upstreamModelId":"gpt-primary-revised","priority":1,"status":"unverified"}]}]}\n' "$request_id"
      fi
      ;;
    *'"type":"modelRouteCreate"'*'"expectedRevision":0'*)
      printf '{"type":"operationError","requestId":"%s","code":"revision_conflict","message":"must-not-be-reflected"}\n' "$request_id"
      ;;
    *'"type":"modelRouteCreate"'*)
      printf '%s\n' "$line" > "$state_root/model-route-create.json"
      route_state=1
      printf '{"type":"modelRouteCreateResult","requestId":"%s","desiredStateRevision":2,"catalogRevision":2,"route":{"modelTag":"gpt-primary","displayName":"GPT Primary","retired":false,"semanticModel":"gpt-5.6-luna","targets":[{"id":"target-created","providerId":"openai","accountId":"account-local","upstreamModelId":"gpt-primary-2026-08-13","priority":0,"status":"unverified"}]}}\n' "$request_id"
      ;;
    *'"type":"modelRouteSetTargets"'*)
      printf '%s\n' "$line" > "$state_root/model-route-set-targets.json"
      route_state=2
      printf '{"type":"modelRouteSetTargetsResult","requestId":"%s","desiredStateRevision":3,"catalogRevision":3,"route":{"modelTag":"gpt-primary","displayName":"GPT Primary","retired":false,"semanticModel":"gpt-5.6-luna","targets":[{"id":"target-backup","providerId":"openai","accountId":"account-backup","upstreamModelId":"gpt-primary-backup","priority":0,"status":"unverified"},{"id":"target-created","providerId":"openai","accountId":"account-local","upstreamModelId":"gpt-primary-revised","priority":1,"status":"unverified"}]}}\n' "$request_id"
      ;;
    *'"type":"modelRouteRetire"'*)
      route_state=3
      printf '{"type":"modelRouteRetireResult","requestId":"%s","desiredStateRevision":4,"catalogRevision":4,"route":{"modelTag":"gpt-primary","displayName":"GPT Primary","retired":true,"semanticModel":"gpt-5.6-luna","targets":[{"id":"target-backup","providerId":"openai","accountId":"account-backup","upstreamModelId":"gpt-primary-backup","priority":0,"status":"unverified"},{"id":"target-created","providerId":"openai","accountId":"account-local","upstreamModelId":"gpt-primary-revised","priority":1,"status":"unverified"}]}}\n' "$request_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdownComplete","requestId":"%s"}\n' "$request_id"
      exit 0
      ;;
  esac
done
"#;
