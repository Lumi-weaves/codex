use super::*;
use pretty_assertions::assert_eq;
use std::ffi::OsString;
use tokio::io::BufReader;

const TEST_WAIT: Duration = Duration::from_millis(100);

#[tokio::test]
async fn reads_bounded_ready_snapshot() {
    let input = br#"{"type":"ready","protocolVersion":1,"instanceId":"backend-1","catalogRevision":7,"kernel":{"sourceRepository":"https://github.com/lidge-jun/opencodex","sourceCommit":"cbbfdd8773e68a5dc2391ddeb32f33a225373c1a","contentDigest":"sha256:65672062788957661574aafd6d32d571d0a33afb0575f6a12e19801d72874b78","compositionVersion":1},"providers":[{"id":"openai","displayName":"OpenAI","accountCount":2,"status":"healthy"}],"models":[{"tag":"gpt-5.6-luna","displayName":"Luna","available":true,"capabilities":["tools"]}]}
"#;
    let mut reader = BufReader::new(&input[..]);

    let snapshot = read_ready(&mut reader, TEST_WAIT).await.unwrap();

    assert_eq!(
        snapshot,
        BackendSnapshot {
            instance_id: "backend-1".to_string(),
            catalog_revision: 7,
            kernel: expected_kernel_provenance().unwrap(),
            providers: vec![ProviderSummary {
                id: "openai".to_string(),
                display_name: "OpenAI".to_string(),
                account_count: 2,
                status: "healthy".to_string(),
            }],
            models: vec![ModelSummary {
                tag: "gpt-5.6-luna".to_string(),
                display_name: "Luna".to_string(),
                available: true,
                capabilities: vec!["tools".to_string()],
            }],
        }
    );
}

#[tokio::test]
async fn rejects_a_kernel_provenance_mismatch() {
    let input = br#"{"type":"ready","protocolVersion":1,"instanceId":"backend-1","catalogRevision":0,"kernel":{"sourceRepository":"https://github.com/lidge-jun/opencodex","sourceCommit":"floating-main","contentDigest":"sha256:untrusted","compositionVersion":1},"providers":[],"models":[]}
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
