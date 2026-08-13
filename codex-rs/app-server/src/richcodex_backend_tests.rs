use super::*;
use pretty_assertions::assert_eq;
use std::ffi::OsString;
use tokio::io::BufReader;

const TEST_WAIT: Duration = Duration::from_millis(100);

#[tokio::test]
async fn reads_bounded_ready_snapshot() {
    let input = br#"{"type":"ready","protocolVersion":7,"instanceId":"backend-1","desiredStateRevision":3,"catalogRevision":7,"dataPlanePort":48767,"kernel":{"sourceRepository":"https://github.com/lidge-jun/opencodex","sourceCommit":"cbbfdd8773e68a5dc2391ddeb32f33a225373c1a","contentDigest":"sha256:65672062788957661574aafd6d32d571d0a33afb0575f6a12e19801d72874b78","selectionDigest":"sha256:fed70f36cf8a71e495e647db03480d5f5213fdc2760c231e6d7e8a414d84edbf","compositionVersion":3},"providers":[{"id":"openai","displayName":"OpenAI","accountCount":2,"status":"ready"}],"models":[{"modelTag":"gpt-5.6-luna","displayName":"Luna","retired":false,"semanticModel":"gpt-5.6-luna","targets":[{"id":"target-1","providerId":"openai","accountId":"account-1","upstreamModelId":"gpt-5.6-luna","priority":0,"status":"unverified"}]}]}
"#;
    let mut reader = BufReader::new(&input[..]);

    let snapshot = read_ready(&mut reader, TEST_WAIT).await.unwrap();

    assert_eq!(
        snapshot,
        BackendSnapshot {
            instance_id: "backend-1".to_string(),
            desired_state_revision: 3,
            catalog_revision: 7,
            data_plane_port: 48767,
            kernel: expected_kernel_provenance().unwrap(),
            providers: vec![ProviderSummary {
                id: "openai".to_string(),
                display_name: "OpenAI".to_string(),
                account_count: 2,
                status: "ready".to_string(),
            }],
            models: vec![ModelSummary {
                model_tag: "gpt-5.6-luna".to_string(),
                display_name: "Luna".to_string(),
                retired: false,
                semantic_model: "gpt-5.6-luna".to_string(),
                targets: vec![ModelTargetSummary {
                    id: "target-1".to_string(),
                    provider_id: "openai".to_string(),
                    account_id: "account-1".to_string(),
                    upstream_model_id: "gpt-5.6-luna".to_string(),
                    priority: 0,
                    status: "unverified".to_string(),
                }],
            }],
        }
    );
}

#[tokio::test]
async fn rejects_a_kernel_provenance_mismatch() {
    let input = br#"{"type":"ready","protocolVersion":7,"instanceId":"backend-1","desiredStateRevision":0,"catalogRevision":0,"dataPlanePort":48767,"kernel":{"sourceRepository":"https://github.com/lidge-jun/opencodex","sourceCommit":"floating-main","contentDigest":"sha256:untrusted","selectionDigest":"sha256:untrusted-selection","compositionVersion":3},"providers":[],"models":[]}
"#;
    let mut reader = BufReader::new(&input[..]);

    let err = read_ready(&mut reader, TEST_WAIT).await.unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(!err.to_string().contains("floating-main"));
}

#[tokio::test]
async fn rejects_incompatible_malformed_and_oversized_handshakes_without_echoing_input() {
    for input in [
        b"{\"type\":\"ready\",\"protocolVersion\":9}\n".to_vec(),
        b"secret-token-that-must-not-be-echoed\n".to_vec(),
        vec![b'x'; MAX_PROTOCOL_LINE_BYTES + 1],
    ] {
        let mut reader = BufReader::new(input.as_slice());
        let err = read_ready(&mut reader, TEST_WAIT).await.unwrap_err();
        assert!(
            !err.to_string()
                .contains("secret-token-that-must-not-be-echoed")
        );
    }
}

#[tokio::test]
async fn shutdown_request_and_acknowledgement_are_correlated() {
    let (mut app_side, backend_side) = tokio::io::duplex(1024);
    let (backend_read, mut backend_write) = tokio::io::split(backend_side);
    let mut backend_read = BufReader::new(backend_read);
    let backend = tokio::spawn(async move {
        let mut request = String::new();
        backend_read.read_line(&mut request).await.unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&request).unwrap(),
            serde_json::json!({"type": "shutdown", "requestId": "request-7"})
        );
        backend_write
            .write_all(b"{\"type\":\"shutdownComplete\",\"requestId\":\"request-7\"}\n")
            .await
            .unwrap();
    });

    write_message(
        &mut app_side,
        &AppServerMessage::Shutdown {
            request_id: "request-7",
        },
    )
    .await
    .unwrap();
    let mut app_side = BufReader::new(app_side);
    read_shutdown_complete(&mut app_side, "request-7", TEST_WAIT)
        .await
        .unwrap();
    backend.await.unwrap();
}

#[tokio::test]
async fn provider_account_list_is_correlated_and_secret_free() {
    let (mut app_side, backend_side) = tokio::io::duplex(4096);
    let (backend_read, mut backend_write) = tokio::io::split(backend_side);
    let mut backend_read = BufReader::new(backend_read);
    let backend = tokio::spawn(async move {
        let mut request = String::new();
        backend_read.read_line(&mut request).await.unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&request).unwrap(),
            serde_json::json!({
                "type": "providerAccountList",
                "requestId": "request-8",
                "cursor": "1",
                "limit": 20
            })
        );
        backend_write
            .write_all(br#"{"type":"providerAccountListResult","requestId":"request-8","desiredStateRevision":2,"catalogRevision":3,"providers":[{"id":"openai","displayName":"OpenAI","accountCount":1,"status":"ready"}],"data":[{"id":"local-1","providerId":"openai","userLabel":"Secondary","credentialKind":"oauth","status":"verificationRequired","addedAt":123}],"nextCursor":null}
"#)
            .await
            .unwrap();
    });

    let (app_read, mut app_write) = tokio::io::split(&mut app_side);
    let mut app_read = BufReader::new(app_read);
    let result = request_provider_account_list(
        &mut app_write,
        &mut app_read,
        "request-8",
        Some("1"),
        Some(20),
    )
    .await
    .unwrap();

    assert_eq!(
        result,
        ProviderAccountListResult {
            desired_state_revision: 2,
            catalog_revision: 3,
            providers: vec![ProviderSummary {
                id: "openai".to_string(),
                display_name: "OpenAI".to_string(),
                account_count: 1,
                status: "ready".to_string(),
            }],
            data: vec![ProviderAccountSummary {
                id: "local-1".to_string(),
                provider_id: "openai".to_string(),
                user_label: "Secondary".to_string(),
                credential_kind: "oauth".to_string(),
                status: "verificationRequired".to_string(),
                added_at: 123,
            }],
            next_cursor: None,
        }
    );
    backend.await.unwrap();
}

#[tokio::test]
async fn provider_account_import_maps_static_operation_errors() {
    let (mut app_side, backend_side) = tokio::io::duplex(4096);
    let (backend_read, mut backend_write) = tokio::io::split(backend_side);
    let mut backend_read = BufReader::new(backend_read);
    let backend = tokio::spawn(async move {
        let mut request = String::new();
        backend_read.read_line(&mut request).await.unwrap();
        let request = serde_json::from_str::<serde_json::Value>(&request).unwrap();
        assert_eq!(request["type"], "providerAccountImport");
        assert_eq!(request["requestId"], "request-9");
        backend_write
            .write_all(br#"{"type":"operationError","requestId":"request-9","code":"invalid_auth_document","message":"selected credential source is not a supported Codex login"}
"#)
            .await
            .unwrap();
    });

    let (app_read, mut app_write) = tokio::io::split(&mut app_side);
    let mut app_read = BufReader::new(app_read);
    let error = request_provider_account_import(
        &mut app_write,
        &mut app_read,
        "request-9",
        "/selected/auth.json",
        "Secondary",
    )
    .await
    .unwrap_err();

    assert_eq!(error, RichCodexBackendClientError::InvalidAuthDocument);
    backend.await.unwrap();
}

#[tokio::test]
async fn provider_account_response_rejects_mismatched_correlation_or_secret_fields() {
    for input in [
        br#"{"type":"providerAccountListResult","requestId":"other","desiredStateRevision":0,"catalogRevision":0,"providers":[],"data":[],"nextCursor":null}
"#
        .as_slice(),
        br#"{"type":"providerAccountListResult","requestId":"request-10","desiredStateRevision":0,"catalogRevision":0,"providers":[],"data":[{"id":"local-1","providerId":"openai","userLabel":"Secondary","credentialKind":"oauth","status":"verificationRequired","addedAt":123,"accessToken":"must-not-cross"}],"nextCursor":null}
"#
        .as_slice(),
    ] {
        let mut reader = BufReader::new(input);
        let mut writer = tokio::io::sink();

        let error =
            request_provider_account_list(&mut writer, &mut reader, "request-10", None, None)
                .await
                .unwrap_err();

        assert_eq!(error, RichCodexBackendClientError::Unavailable);
    }
}

#[tokio::test]
async fn provider_account_login_start_is_correlated_and_secret_free() {
    let (mut app_side, backend_side) = tokio::io::duplex(4096);
    let (backend_read, mut backend_write) = tokio::io::split(backend_side);
    let mut backend_read = BufReader::new(backend_read);
    let backend = tokio::spawn(async move {
        let mut request = String::new();
        backend_read.read_line(&mut request).await.unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&request).unwrap(),
            serde_json::json!({
                "type": "providerAccountLoginStart",
                "requestId": "request-login",
                "userLabel": "Third Codex",
            })
        );
        backend_write
            .write_all(br#"{"type":"providerAccountLoginStartResult","requestId":"request-login","loginId":"login-safe-handle","status":"awaitingUser","verificationUrl":"https://auth.openai.com/codex/device","userCode":"SAFE-CODE","expiresAt":2000,"failure":null,"account":null,"desiredStateRevision":1,"catalogRevision":1}
"#)
            .await
            .unwrap();
    });

    let (app_read, mut app_write) = tokio::io::split(&mut app_side);
    let mut app_read = BufReader::new(app_read);
    let result = request_provider_account_login_start(
        &mut app_write,
        &mut app_read,
        "request-login",
        "Third Codex",
    )
    .await
    .unwrap();

    assert_eq!(
        result,
        ProviderAccountLoginResult {
            login_id: "login-safe-handle".to_string(),
            status: "awaitingUser".to_string(),
            verification_url: Some("https://auth.openai.com/codex/device".to_string()),
            user_code: Some("SAFE-CODE".to_string()),
            expires_at: 2000,
            failure: None,
            account: None,
            desired_state_revision: 1,
            catalog_revision: 1,
        }
    );
    backend.await.unwrap();
}

#[tokio::test]
async fn model_route_create_is_correlated_and_secret_free() {
    let (mut app_side, backend_side) = tokio::io::duplex(4096);
    let (backend_read, mut backend_write) = tokio::io::split(backend_side);
    let mut backend_read = BufReader::new(backend_read);
    let backend = tokio::spawn(async move {
        let mut request = String::new();
        backend_read.read_line(&mut request).await.unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&request).unwrap(),
            serde_json::json!({
                "type": "modelRouteCreate",
                "requestId": "request-11",
                "expectedRevision": 4,
                "modelTag": "gpt-primary",
                "displayName": "GPT Primary",
                "semanticModel": "openai/gpt-primary",
                "providerId": "openai",
                "accountId": "account-local",
                "upstreamModelId": "gpt-primary-2026-08-13"
            })
        );
        backend_write
            .write_all(br#"{"type":"modelRouteCreateResult","requestId":"request-11","desiredStateRevision":5,"catalogRevision":5,"route":{"modelTag":"gpt-primary","displayName":"GPT Primary","retired":false,"semanticModel":"openai/gpt-primary","targets":[{"id":"target-1","providerId":"openai","accountId":"account-local","upstreamModelId":"gpt-primary-2026-08-13","priority":0,"status":"unverified"}]}}
"#)
            .await
            .unwrap();
    });

    let (app_read, mut app_write) = tokio::io::split(&mut app_side);
    let mut app_read = BufReader::new(app_read);
    let result = request_model_route_create(
        &mut app_write,
        &mut app_read,
        "request-11",
        &ModelRouteCreateRequest {
            expected_revision: 4,
            model_tag: "gpt-primary".to_string(),
            display_name: "GPT Primary".to_string(),
            semantic_model: "openai/gpt-primary".to_string(),
            provider_id: "openai".to_string(),
            account_id: "account-local".to_string(),
            upstream_model_id: "gpt-primary-2026-08-13".to_string(),
        },
    )
    .await
    .unwrap();

    assert_eq!(result.desired_state_revision, 5);
    assert_eq!(result.catalog_revision, 5);
    assert_eq!(result.route.model_tag, "gpt-primary");
    assert_eq!(result.route.targets[0].account_id, "account-local");
    backend.await.unwrap();
}

#[tokio::test]
async fn model_route_set_targets_preserves_order_and_optional_identity() {
    let (mut app_side, backend_side) = tokio::io::duplex(4096);
    let (backend_read, mut backend_write) = tokio::io::split(backend_side);
    let mut backend_read = BufReader::new(backend_read);
    let backend = tokio::spawn(async move {
        let mut request = String::new();
        backend_read.read_line(&mut request).await.unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&request).unwrap(),
            serde_json::json!({
                "type": "modelRouteSetTargets",
                "requestId": "request-targets",
                "expectedRevision": 5,
                "modelTag": "gpt-primary",
                "targets": [
                    {
                        "id": null,
                        "providerId": "openai",
                        "accountId": "account-backup",
                        "upstreamModelId": "gpt-backup"
                    },
                    {
                        "id": "target-1",
                        "providerId": "openai",
                        "accountId": "account-local",
                        "upstreamModelId": "gpt-primary"
                    }
                ]
            })
        );
        backend_write
            .write_all(br#"{"type":"modelRouteSetTargetsResult","requestId":"request-targets","desiredStateRevision":6,"catalogRevision":6,"route":{"modelTag":"gpt-primary","displayName":"GPT Primary","retired":false,"semanticModel":"gpt-5.4","targets":[{"id":"target-2","providerId":"openai","accountId":"account-backup","upstreamModelId":"gpt-backup","priority":0,"status":"unverified"},{"id":"target-1","providerId":"openai","accountId":"account-local","upstreamModelId":"gpt-primary","priority":1,"status":"unverified"}]}}
"#)
            .await
            .unwrap();
    });

    let (app_read, mut app_write) = tokio::io::split(&mut app_side);
    let mut app_read = BufReader::new(app_read);
    let result = request_model_route_set_targets(
        &mut app_write,
        &mut app_read,
        "request-targets",
        &ModelRouteSetTargetsRequest {
            expected_revision: 5,
            model_tag: "gpt-primary".to_string(),
            targets: vec![
                ModelRouteTargetRequest {
                    id: None,
                    provider_id: "openai".to_string(),
                    account_id: "account-backup".to_string(),
                    upstream_model_id: "gpt-backup".to_string(),
                },
                ModelRouteTargetRequest {
                    id: Some("target-1".to_string()),
                    provider_id: "openai".to_string(),
                    account_id: "account-local".to_string(),
                    upstream_model_id: "gpt-primary".to_string(),
                },
            ],
        },
    )
    .await
    .unwrap();

    assert_eq!(result.desired_state_revision, 6);
    assert_eq!(result.route.targets[0].id, "target-2");
    assert_eq!(result.route.targets[1].id, "target-1");
    backend.await.unwrap();
}

#[tokio::test]
async fn model_route_retire_maps_revision_conflict() {
    let input = br#"{"type":"operationError","requestId":"request-12","code":"revision_conflict","message":"must-not-be-reflected"}
"#;
    let mut reader = BufReader::new(&input[..]);
    let mut writer = tokio::io::sink();

    let error =
        request_model_route_retire(&mut writer, &mut reader, "request-12", 3, "gpt-primary")
            .await
            .unwrap_err();

    assert_eq!(error, RichCodexBackendClientError::RevisionConflict);
}

#[tokio::test]
async fn model_route_response_rejects_secret_fields_and_invalid_priorities() {
    for input in [
        br#"{"type":"modelRouteReadResult","requestId":"request-13","desiredStateRevision":1,"catalogRevision":1,"data":[{"modelTag":"gpt-primary","displayName":"GPT Primary","retired":false,"semanticModel":"openai/gpt-primary","targets":[{"id":"target-1","providerId":"openai","accountId":"account-local","upstreamModelId":"gpt-primary","priority":0,"status":"unverified","accessToken":"must-not-cross"}]}]}
"#
        .as_slice(),
        br#"{"type":"modelRouteReadResult","requestId":"request-13","desiredStateRevision":1,"catalogRevision":1,"data":[{"modelTag":"gpt-primary","displayName":"GPT Primary","retired":false,"semanticModel":"openai/gpt-primary","targets":[{"id":"target-1","providerId":"openai","accountId":"account-local","upstreamModelId":"gpt-primary","priority":1,"status":"unverified"}]}]}
"#
        .as_slice(),
    ] {
        let mut reader = BufReader::new(input);
        let mut writer = tokio::io::sink();

        let error = request_model_route_read(&mut writer, &mut reader, "request-13")
            .await
            .unwrap_err();

        assert_eq!(error, RichCodexBackendClientError::Unavailable);
    }
}

#[tokio::test]
async fn handshake_timeout_is_bounded() {
    let (_writer, reader) = tokio::io::duplex(16);
    let mut reader = BufReader::new(reader);

    let err = read_ready(&mut reader, Duration::from_millis(1))
        .await
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
}

#[test]
fn backend_resolution_requires_absolute_override_and_falls_back_to_sibling() {
    let absolute_backend = std::env::current_dir()
        .unwrap()
        .join("custom-richcodex-model-backend");
    let err = resolve_backend_executable(
        Some(OsString::from("relative/backend")),
        std::env::current_exe().unwrap().as_path(),
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

    assert_eq!(
        resolve_backend_executable(
            Some(absolute_backend.clone().into_os_string()),
            std::env::current_exe().unwrap().as_path(),
        )
        .unwrap(),
        Some(absolute_backend)
    );
}

#[tokio::test]
async fn eof_is_reported_without_waiting_for_timeout() {
    let input = &b""[..];
    let mut reader = BufReader::new(input);

    let err = read_ready(&mut reader, TEST_WAIT).await.unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}
