use super::*;
use crate::request_processors::model_workbench_transport::create_capability;
use crate::request_processors::model_workbench_transport::direct_client;
use crate::request_processors::model_workbench_transport::is_base64url_256;
use crate::request_processors::model_workbench_transport::loopback_host;
use pretty_assertions::assert_eq;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

#[test]
fn capability_matches_the_opencodex_contract() {
    let capability = create_capability(
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB",
        &Method::PUT,
        ENTRIES_PATH,
        1234,
        10100,
        2_000_000_000_000,
        "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC",
    )
    .expect("valid capability inputs");

    assert_eq!(capability, "UYY-lUiT-jJy-uHwx6EZJ52zfcfFyIgOsGjL6wKlsFk");
}

#[test]
fn only_explicit_loopback_runtime_hosts_are_admitted() {
    assert_eq!(loopback_host(None).expect("default loopback"), "127.0.0.1");
    assert_eq!(
        loopback_host(Some("0.0.0.0")).expect("wildcard reaches local listener"),
        "127.0.0.1"
    );
    assert_eq!(loopback_host(Some("::1")).expect("IPv6 loopback"), "[::1]");
    assert!(loopback_host(Some("192.0.2.8")).is_err());
}

#[test]
fn capability_inputs_are_strict_base64url_sha256_values() {
    assert!(is_base64url_256(
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    ));
    assert!(!is_base64url_256(
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
    ));
    assert!(
        create_capability(
            "not-a-signing-root",
            "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB",
            &Method::GET,
            ENTRIES_PATH,
            1,
            10100,
            2_000_000_000_000,
            "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC",
        )
        .is_err()
    );
}

#[test]
fn request_fields_match_opencodex_utf16_and_control_limits() {
    assert!(validate_model_tag("provider/model").is_ok());
    assert!(validate_display_name("Work Model").is_ok());
    assert!(validate_model_tag(" model").is_err());
    assert!(validate_display_name("bad\nname").is_err());
    assert!(validate_display_name(&"😀".repeat(60)).is_ok());
    assert!(validate_display_name(&"😀".repeat(61)).is_err());
}

#[test]
fn terminal_publication_skips_are_failed_not_pending() {
    let publication = ApiPublication {
        registry_revision: 4,
        catalog_revision: Some(3),
        models_cache_revision: Some(3),
        synchronized: false,
        catalog_refresh: serde_json::json!({
            "status": "skipped",
            "reason": "refused",
            "retryable": false,
        }),
    };

    assert_eq!(
        publication_status(&publication),
        ModelWorkbenchPublicationStatus::Failed
    );
}

#[tokio::test]
async fn backend_conflict_exposes_only_the_current_revision() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/conflict"))
        .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
            "error": "internal backend wording is not part of the RPC contract",
            "currentRevision": 19,
        })))
        .mount(&server)
        .await;

    let response = direct_client()
        .expect("direct client")
        .get(format!("{}/conflict", server.uri()))
        .send()
        .await
        .expect("mock response");
    let error = require_status(response, &[StatusCode::OK])
        .await
        .expect_err("conflict must fail");

    assert_eq!(error.code, REVISION_CONFLICT_ERROR_CODE);
    assert_eq!(error.message, "Model Workbench revision conflict");
    assert_eq!(
        error.data,
        Some(serde_json::json!({ "currentRevision": 19 }))
    );
}

#[tokio::test]
async fn backend_busy_maps_to_the_existing_overload_code() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/busy"))
        .respond_with(ResponseTemplate::new(503).set_body_string("private backend detail"))
        .mount(&server)
        .await;

    let response = direct_client()
        .expect("direct client")
        .get(format!("{}/busy", server.uri()))
        .send()
        .await
        .expect("mock response");
    let error = require_status(response, &[StatusCode::OK])
        .await
        .expect_err("busy must fail");

    assert_eq!(error.code, crate::error_code::OVERLOADED_ERROR_CODE);
    assert_eq!(error.message, "Model Workbench is busy");
    assert_eq!(error.data, None);
}
