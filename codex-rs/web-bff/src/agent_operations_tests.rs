use super::*;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering as AtomicOrdering;

#[derive(Default)]
struct FakeSource {
    loaded: Mutex<Option<Result<LoadedThreadIds, ()>>>,
    threads: HashMap<String, Result<ThreadSummary, ()>>,
    delays_ms: HashMap<String, u64>,
    in_flight: Option<Arc<AtomicUsize>>,
    max_in_flight: Option<Arc<AtomicUsize>>,
}

impl AgentOperationsSource for FakeSource {
    async fn loaded_thread_ids(&self) -> Result<LoadedThreadIds, AgentOperationsError> {
        self.loaded
            .lock()
            .expect("loaded lock")
            .take()
            .unwrap_or_else(|| {
                Ok(LoadedThreadIds {
                    ids: Vec::new(),
                    is_truncated: false,
                })
            })
            .map_err(|()| AgentOperationsError::Upstream)
    }

    async fn read_thread(&self, thread_id: String) -> Result<ThreadSummary, AgentOperationsError> {
        if let Some(delay_ms) = self.delays_ms.get(&thread_id) {
            let _guard = self.in_flight.as_ref().map(|in_flight| {
                let active = in_flight.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                if let Some(max_in_flight) = self.max_in_flight.as_ref() {
                    max_in_flight.fetch_max(active, AtomicOrdering::SeqCst);
                }
                InFlightGuard(Arc::clone(in_flight))
            });
            tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
        }
        self.threads
            .get(&thread_id)
            .cloned()
            .unwrap_or(Err(()))
            .map_err(|()| AgentOperationsError::Upstream)
    }
}

struct InFlightGuard(Arc<AtomicUsize>);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, AtomicOrdering::SeqCst);
    }
}

fn thread(id: &str, parent_id: Option<&str>, status: RuntimeStatus) -> ThreadSummary {
    ThreadSummary {
        id: id.to_string(),
        parent_id: parent_id.map(str::to_string),
        worker: parent_id.is_some(),
        nickname: None,
        status,
        created_at: 1_700_000_000,
        updated_at: 1_700_000_020,
    }
}

#[test]
fn status_mapping_is_exhaustive_and_thread_state_wins() {
    let cases = [
        (
            RuntimeStatus::Active(Some(ThreadActiveFlag::WaitingOnApproval)),
            AgentOperationStatus::Waiting,
            "Awaiting approval",
        ),
        (
            RuntimeStatus::Active(Some(ThreadActiveFlag::WaitingOnUserInput)),
            AgentOperationStatus::Waiting,
            "Awaiting user input",
        ),
        (
            RuntimeStatus::Active(Some(ThreadActiveFlag::WaitingOnBackgroundTerminal)),
            AgentOperationStatus::Waiting,
            "Awaiting background terminal",
        ),
        (
            RuntimeStatus::Active(None),
            AgentOperationStatus::Running,
            "Turn in progress",
        ),
        (
            RuntimeStatus::SystemError,
            AgentOperationStatus::Failed,
            "Thread system error",
        ),
        (
            RuntimeStatus::Idle,
            AgentOperationStatus::Idle,
            "Thread idle",
        ),
    ];

    for (index, (runtime, expected_status, expected_activity)) in cases.into_iter().enumerate() {
        let node = project_thread(
            thread(&format!("thread-{index}"), None, runtime),
            "2026-08-12T00:00:00.000Z",
        )
        .expect("projected node");
        assert_eq!(node.status, expected_status);
        assert_eq!(node.activity, expected_activity);
    }

    assert!(
        project_thread(
            thread("unloaded", None, RuntimeStatus::NotLoaded),
            "2026-08-12T00:00:00.000Z"
        )
        .is_none()
    );
}

#[tokio::test]
async fn snapshot_is_deterministic_repairs_the_forest_and_marks_gaps() {
    let ids = [
        "root", "child", "orphan", "cycle-a", "cycle-b", "unloaded", "missing",
    ];
    let source = FakeSource {
        loaded: Mutex::new(Some(Ok(LoadedThreadIds {
            ids: ids.into_iter().map(str::to_string).collect(),
            is_truncated: true,
        }))),
        threads: HashMap::from([
            (
                "root".to_string(),
                Ok(thread("root", None, RuntimeStatus::Active(None))),
            ),
            (
                "child".to_string(),
                Ok(thread("child", Some("root"), RuntimeStatus::Idle)),
            ),
            (
                "orphan".to_string(),
                Ok(thread(
                    "orphan",
                    Some("missing-parent"),
                    RuntimeStatus::Idle,
                )),
            ),
            (
                "cycle-a".to_string(),
                Ok(thread("cycle-a", Some("cycle-b"), RuntimeStatus::Idle)),
            ),
            (
                "cycle-b".to_string(),
                Ok(thread("cycle-b", Some("cycle-a"), RuntimeStatus::Idle)),
            ),
            (
                "unloaded".to_string(),
                Ok(thread("unloaded", None, RuntimeStatus::NotLoaded)),
            ),
            ("missing".to_string(), Err(())),
        ]),
        delays_ms: HashMap::from([("root".to_string(), 10)]),
        in_flight: None,
        max_in_flight: None,
    };

    let snapshot = snapshot_from_source(&source).await.expect("snapshot");
    assert_eq!(snapshot.schema_version, 1);
    assert!(snapshot.is_partial);
    assert!(snapshot.is_truncated);
    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>(),
        vec!["root", "child", "orphan", "cycle-a", "cycle-b"]
    );
    let parents = snapshot
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.parent_id.as_deref()))
        .collect::<HashMap<_, _>>();
    assert_eq!(parents["child"], Some("root"));
    assert_eq!(parents["orphan"], None);
    assert_eq!(parents["cycle-a"], None);
    assert_eq!(parents["cycle-b"], Some("cycle-a"));

    let json = serde_json::to_value(&snapshot).expect("serialize snapshot");
    assert_eq!(
        json.as_object()
            .expect("snapshot object")
            .keys()
            .cloned()
            .collect::<HashSet<_>>(),
        HashSet::from([
            "schemaVersion".to_string(),
            "capturedAt".to_string(),
            "isPartial".to_string(),
            "isTruncated".to_string(),
            "nodes".to_string(),
        ])
    );
    assert_eq!(
        json["nodes"][0]
            .as_object()
            .expect("node object")
            .keys()
            .cloned()
            .collect::<HashSet<_>>(),
        HashSet::from([
            "id".to_string(),
            "parentId".to_string(),
            "role".to_string(),
            "label".to_string(),
            "status".to_string(),
            "activity".to_string(),
            "model".to_string(),
            "startedAt".to_string(),
            "updatedAt".to_string(),
        ])
    );
}

#[tokio::test]
async fn snapshot_fails_closed_when_inventory_or_all_metadata_is_unavailable() {
    let source = FakeSource {
        loaded: Mutex::new(Some(Err(()))),
        ..Default::default()
    };
    assert!(matches!(
        snapshot_from_source(&source).await,
        Err(AgentOperationsError::Upstream)
    ));

    let source = FakeSource {
        loaded: Mutex::new(Some(Ok(LoadedThreadIds {
            ids: vec!["first".to_string(), "second".to_string()],
            is_truncated: false,
        }))),
        threads: HashMap::from([
            ("first".to_string(), Err(())),
            ("second".to_string(), Err(())),
        ]),
        delays_ms: HashMap::new(),
        in_flight: None,
        max_in_flight: None,
    };
    assert!(matches!(
        snapshot_from_source(&source).await,
        Err(AgentOperationsError::ThreadMetadataUnavailable)
    ));
}

#[tokio::test(start_paused = true)]
async fn snapshot_enforces_read_concurrency_and_deadlines() {
    let in_flight = Arc::new(AtomicUsize::new(0));
    let max_in_flight = Arc::new(AtomicUsize::new(0));
    let ids = (0..MAX_THREADS)
        .map(|index| format!("thread-{index}"))
        .collect::<Vec<_>>();
    let source = Arc::new(FakeSource {
        loaded: Mutex::new(Some(Ok(LoadedThreadIds {
            ids: ids.clone(),
            is_truncated: true,
        }))),
        threads: ids
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    Ok(thread(id, None, RuntimeStatus::Active(None))),
                )
            })
            .collect(),
        delays_ms: ids.iter().map(|id| (id.clone(), 100)).collect(),
        in_flight: Some(Arc::clone(&in_flight)),
        max_in_flight: Some(Arc::clone(&max_in_flight)),
    });
    let snapshot_task = tokio::spawn({
        let source = Arc::clone(&source);
        async move { snapshot_from_source(source.as_ref()).await }
    });
    tokio::task::yield_now().await;
    assert_eq!(max_in_flight.load(AtomicOrdering::SeqCst), MAX_THREAD_READS);
    tokio::time::advance(Duration::from_secs(2)).await;
    let snapshot = snapshot_task
        .await
        .expect("snapshot task")
        .expect("snapshot");
    assert_eq!(snapshot.nodes.len(), MAX_THREADS);
    assert!(snapshot.is_truncated);
    assert!(max_in_flight.load(AtomicOrdering::SeqCst) <= MAX_THREAD_READS);

    let timeout_source = FakeSource {
        loaded: Mutex::new(Some(Ok(LoadedThreadIds {
            ids: vec!["slow".to_string()],
            is_truncated: false,
        }))),
        threads: HashMap::from([(
            "slow".to_string(),
            Ok(thread("slow", None, RuntimeStatus::Active(None))),
        )]),
        delays_ms: HashMap::from([("slow".to_string(), 10_000)]),
        in_flight: None,
        max_in_flight: None,
    };
    let result = snapshot_from_source(&timeout_source).await;
    assert!(matches!(
        result,
        Err(AgentOperationsError::ThreadMetadataUnavailable)
    ));

    let deadline_ids = (0..MAX_THREADS)
        .map(|index| format!("deadline-{index}"))
        .collect::<Vec<_>>();
    let deadline_source = FakeSource {
        loaded: Mutex::new(Some(Ok(LoadedThreadIds {
            ids: deadline_ids.clone(),
            is_truncated: false,
        }))),
        threads: deadline_ids
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    Ok(thread(id, None, RuntimeStatus::Active(None))),
                )
            })
            .collect(),
        delays_ms: deadline_ids.iter().map(|id| (id.clone(), 10_000)).collect(),
        in_flight: None,
        max_in_flight: None,
    };
    let deadline_task = tokio::spawn(async move { snapshot_with_deadline(&deadline_source).await });
    tokio::task::yield_now().await;
    tokio::time::advance(SNAPSHOT_TIMEOUT + Duration::from_millis(1)).await;
    assert!(matches!(
        deadline_task.await.expect("deadline task"),
        Err(AgentOperationsError::Deadline)
    ));
}
