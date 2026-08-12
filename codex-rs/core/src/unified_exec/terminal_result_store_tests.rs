use pretty_assertions::assert_eq;

use super::*;

fn input(process_id: i32, output: &str) -> TerminalResultInput {
    TerminalResultInput {
        process_id,
        item_id: format!("call-{process_id}"),
        command: "printf output".to_string(),
        cwd: "/tmp".to_string(),
        exit_code: Some(0),
        failure_message: None,
        duration_ms: 25,
        output_bytes_total: output.len(),
        output_bytes_retained: output.len(),
        output_bytes_omitted: 0,
        retained_output: output.to_string(),
    }
}

#[test]
fn retained_result_reads_progressive_utf8_pages() {
    let mut store = TerminalResultStore::with_limits(2, 2);
    let metadata = store.retain(input(1000, "ab界cd"));

    let first = store.read(&metadata.result_ref, 0, 1).expect("first page");
    assert_eq!(first.state, TerminalResultState::Available);
    assert_eq!(first.output.as_deref(), Some("a"));
    assert_eq!(first.next_offset, Some(1));

    let second = store.read(&metadata.result_ref, 1, 2).expect("second page");
    assert_eq!(second.output.as_deref(), Some("b"));
    assert_eq!(second.next_offset, Some(2));

    let third = store
        .read(&metadata.result_ref, 2, 1)
        .expect("a scalar may exceed the byte budget to ensure progress");
    assert_eq!(third.output.as_deref(), Some("界"));
    assert_eq!(third.next_offset, Some(5));
}

#[test]
fn least_recently_used_available_result_becomes_evicted_tombstone() {
    let mut store = TerminalResultStore::with_limits(2, 4);
    let first = store.retain(input(1000, "one"));
    let second = store.retain(input(1001, "two"));
    store.read(&first.result_ref, 0, 10).expect("touch first");
    let third = store.retain(input(1002, "three"));

    assert_eq!(
        store.read(&second.result_ref, 0, 10).expect("evicted read"),
        TerminalResultRead {
            state: TerminalResultState::Evicted,
            result_ref: second.result_ref.clone(),
            metadata: Some(second),
            output_offset: None,
            output: None,
            next_offset: None,
        }
    );
    assert_eq!(
        store
            .read(&third.result_ref, 0, 10)
            .expect("available read")
            .output
            .as_deref(),
        Some("three")
    );
    assert_eq!(
        store.read("missing", 0, 10).expect("missing read"),
        TerminalResultRead {
            state: TerminalResultState::Unavailable,
            result_ref: "missing".to_string(),
            metadata: None,
            output_offset: None,
            output: None,
            next_offset: None,
        }
    );
}
