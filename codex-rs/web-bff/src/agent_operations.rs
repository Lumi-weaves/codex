use chrono::SecondsFormat;
use chrono::Utc;
use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::SessionSource;
use codex_app_server_protocol::ThreadActiveFlag;
use codex_app_server_protocol::ThreadLoadedListParams;
use codex_app_server_protocol::ThreadLoadedListResponse;
use codex_app_server_protocol::ThreadReadParams;
use codex_app_server_protocol::ThreadReadResponse;
use codex_app_server_protocol::ThreadStatus;
use futures::StreamExt;
use futures::stream;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::HashSet;
use std::future::Future;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::time::timeout;

const MAX_THREADS: usize = 200;
const MAX_THREAD_READS: usize = 16;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentOperationRole {
    Root,
    Worker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentOperationStatus {
    Running,
    Waiting,
    Idle,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOperationNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub role: AgentOperationRole,
    pub label: String,
    pub status: AgentOperationStatus,
    pub activity: String,
    pub model: Option<String>,
    pub started_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOperationsSnapshot {
    pub schema_version: u32,
    pub captured_at: String,
    pub is_partial: bool,
    pub is_truncated: bool,
    pub nodes: Vec<AgentOperationNode>,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentOperationsError {
    #[error("app-server request failed")]
    Upstream,
    #[error("thread metadata was unavailable")]
    ThreadMetadataUnavailable,
    #[error("snapshot deadline elapsed")]
    Deadline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeStatus {
    NotLoaded,
    Idle,
    SystemError,
    Active(Option<ThreadActiveFlag>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ThreadSummary {
    id: String,
    parent_id: Option<String>,
    worker: bool,
    nickname: Option<String>,
    status: RuntimeStatus,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug)]
struct LoadedThreadIds {
    ids: Vec<String>,
    is_truncated: bool,
}

/// Supplies the bounded loaded-thread inventory and metadata used by the Web
/// projection. Implementations must not request or return turn content.
trait AgentOperationsSource: Sync {
    fn loaded_thread_ids(
        &self,
    ) -> impl Future<Output = Result<LoadedThreadIds, AgentOperationsError>> + Send;

    fn read_thread(
        &self,
        thread_id: String,
    ) -> impl Future<Output = Result<ThreadSummary, AgentOperationsError>> + Send;
}

#[derive(Clone)]
struct AppServerAgentOperationsSource {
    handle: AppServerRequestHandle,
}

impl AppServerAgentOperationsSource {
    fn new(handle: AppServerRequestHandle) -> Self {
        Self { handle }
    }

    fn request_id(&self) -> RequestId {
        RequestId::String(format!(
            "lumi-web/{}",
            NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }
}

impl AgentOperationsSource for AppServerAgentOperationsSource {
    async fn loaded_thread_ids(&self) -> Result<LoadedThreadIds, AgentOperationsError> {
        let response = self
            .handle
            .request_typed::<ThreadLoadedListResponse>(ClientRequest::ThreadLoadedList {
                request_id: self.request_id(),
                params: ThreadLoadedListParams {
                    cursor: None,
                    limit: Some((MAX_THREADS + 1) as u32),
                },
            })
            .await
            .map_err(|_| AgentOperationsError::Upstream)?;
        let is_truncated = response.next_cursor.is_some() || response.data.len() > MAX_THREADS;
        Ok(LoadedThreadIds {
            ids: response.data.into_iter().take(MAX_THREADS).collect(),
            is_truncated,
        })
    }

    async fn read_thread(&self, thread_id: String) -> Result<ThreadSummary, AgentOperationsError> {
        let response = self
            .handle
            .request_typed::<ThreadReadResponse>(ClientRequest::ThreadRead {
                request_id: self.request_id(),
                params: ThreadReadParams {
                    thread_id,
                    include_turns: false,
                },
            })
            .await
            .map_err(|_| AgentOperationsError::Upstream)?;
        Ok(ThreadSummary::from(response.thread))
    }
}

impl From<codex_app_server_protocol::Thread> for ThreadSummary {
    fn from(thread: codex_app_server_protocol::Thread) -> Self {
        let worker = thread.parent_thread_id.is_some()
            || matches!(thread.source, SessionSource::SubAgent(_));
        let status = match thread.status {
            ThreadStatus::NotLoaded => RuntimeStatus::NotLoaded,
            ThreadStatus::Idle => RuntimeStatus::Idle,
            ThreadStatus::SystemError => RuntimeStatus::SystemError,
            ThreadStatus::Active { active_flags } => RuntimeStatus::Active(
                [
                    ThreadActiveFlag::WaitingOnApproval,
                    ThreadActiveFlag::WaitingOnUserInput,
                    ThreadActiveFlag::WaitingOnBackgroundTerminal,
                ]
                .into_iter()
                .find(|flag| active_flags.contains(flag)),
            ),
        };
        Self {
            id: thread.id,
            parent_id: thread.parent_thread_id,
            worker,
            nickname: thread.agent_nickname,
            status,
            created_at: thread.created_at,
            updated_at: thread.updated_at,
        }
    }
}

pub struct AgentOperationsService {
    source: AppServerAgentOperationsSource,
}

impl AgentOperationsService {
    pub fn new(handle: AppServerRequestHandle) -> Self {
        Self {
            source: AppServerAgentOperationsSource::new(handle),
        }
    }

    pub async fn snapshot(&self) -> Result<AgentOperationsSnapshot, AgentOperationsError> {
        snapshot_with_deadline(&self.source).await
    }
}

async fn snapshot_with_deadline<S>(
    source: &S,
) -> Result<AgentOperationsSnapshot, AgentOperationsError>
where
    S: AgentOperationsSource,
{
    timeout(SNAPSHOT_TIMEOUT, snapshot_from_source(source))
        .await
        .map_err(|_| AgentOperationsError::Deadline)?
}

async fn snapshot_from_source<S>(
    source: &S,
) -> Result<AgentOperationsSnapshot, AgentOperationsError>
where
    S: AgentOperationsSource,
{
    let loaded = timeout(REQUEST_TIMEOUT, source.loaded_thread_ids())
        .await
        .map_err(|_| AgentOperationsError::Upstream)??;
    let mut observations = stream::iter(loaded.ids.into_iter().enumerate().map(
        |(position, thread_id)| async move {
            let thread = timeout(REQUEST_TIMEOUT, source.read_thread(thread_id))
                .await
                .map_err(|_| AgentOperationsError::Upstream)
                .and_then(std::convert::identity);
            (position, thread)
        },
    ))
    .buffer_unordered(MAX_THREAD_READS)
    .collect::<Vec<_>>()
    .await;

    if !observations.is_empty() && observations.iter().all(|(_, thread)| thread.is_err()) {
        return Err(AgentOperationsError::ThreadMetadataUnavailable);
    }

    observations.sort_by_key(|(position, _)| *position);
    let captured_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let is_partial = observations.iter().any(|(_, thread)| match thread {
        Ok(thread) => matches!(thread.status, RuntimeStatus::NotLoaded),
        Err(_) => true,
    });
    let mut nodes = observations
        .into_iter()
        .filter_map(|(_, thread)| thread.ok())
        .filter_map(|thread| project_thread(thread, &captured_at))
        .collect::<Vec<_>>();
    normalize_parents(&mut nodes);
    Ok(AgentOperationsSnapshot {
        schema_version: 1,
        captured_at,
        is_partial,
        is_truncated: loaded.is_truncated,
        nodes,
    })
}

fn project_thread(thread: ThreadSummary, captured_at: &str) -> Option<AgentOperationNode> {
    let (status, activity) = match thread.status {
        RuntimeStatus::NotLoaded => return None,
        RuntimeStatus::Idle => (AgentOperationStatus::Idle, "Thread idle"),
        RuntimeStatus::SystemError => (AgentOperationStatus::Failed, "Thread system error"),
        RuntimeStatus::Active(Some(ThreadActiveFlag::WaitingOnApproval)) => {
            (AgentOperationStatus::Waiting, "Awaiting approval")
        }
        RuntimeStatus::Active(Some(ThreadActiveFlag::WaitingOnUserInput)) => {
            (AgentOperationStatus::Waiting, "Awaiting user input")
        }
        RuntimeStatus::Active(Some(ThreadActiveFlag::WaitingOnBackgroundTerminal)) => (
            AgentOperationStatus::Waiting,
            "Awaiting background terminal",
        ),
        RuntimeStatus::Active(None) => (AgentOperationStatus::Running, "Turn in progress"),
    };
    let role = if thread.worker {
        AgentOperationRole::Worker
    } else {
        AgentOperationRole::Root
    };
    let short_id = thread.id.chars().take(8).collect::<String>();
    let nickname = thread.nickname.and_then(sanitize_nickname);
    let label = match (role, nickname) {
        (AgentOperationRole::Worker, Some(nickname)) => format!("Agent {nickname}"),
        (AgentOperationRole::Worker, None) => format!("Worker {short_id}"),
        (AgentOperationRole::Root, _) => format!("Root {short_id}"),
    };
    Some(AgentOperationNode {
        id: thread.id,
        parent_id: thread.parent_id,
        role,
        label,
        status,
        activity: activity.to_string(),
        model: None,
        started_at: iso_time(thread.created_at),
        updated_at: iso_time(thread.updated_at).unwrap_or_else(|| captured_at.to_string()),
    })
}

fn iso_time(seconds: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|time| time.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn sanitize_nickname(value: String) -> Option<String> {
    let value = value
        .chars()
        .filter(|character| !character.is_control())
        .take(48)
        .collect::<String>();
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn normalize_parents(nodes: &mut [AgentOperationNode]) {
    let ids = nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    let mut parents = nodes
        .iter()
        .map(|node| {
            let parent = node
                .parent_id
                .clone()
                .filter(|parent| parent != &node.id && ids.contains(parent));
            (node.id.clone(), parent)
        })
        .collect::<HashMap<_, _>>();
    for start in ids {
        let mut path = Vec::new();
        let mut positions = HashMap::new();
        let mut current = Some(start);
        while let Some(id) = current {
            if let Some(position) = positions.insert(id.clone(), path.len()) {
                if let Some(cut) = path[position..].iter().min().cloned() {
                    parents.insert(cut, None);
                }
                break;
            }
            path.push(id.clone());
            current = parents.get(&id).cloned().flatten();
        }
    }
    for node in nodes {
        node.parent_id = parents.remove(&node.id).flatten();
    }
}

#[cfg(test)]
#[path = "agent_operations_tests.rs"]
mod tests;
