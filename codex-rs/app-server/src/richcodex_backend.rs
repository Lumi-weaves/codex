use codex_core::RuntimeModelProviderRoutes;
use codex_model_provider::create_private_openai_loopback_model_provider;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashSet;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncBufRead;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::info;
use uuid::Uuid;

#[path = "richcodex_backend_client.rs"]
mod client;
#[path = "richcodex_backend_provider_login.rs"]
mod provider_login;
use client::BackendOperationErrorCode;
pub(crate) use client::ModelRouteCreateRequest;
pub(crate) use client::ModelRouteMutationResult;
pub(crate) use client::ModelRouteReadResult;
pub(crate) use client::ModelRouteSetTargetsRequest;
pub(crate) use client::ModelRouteTargetRequest;
pub(crate) use client::ProviderAccountAddApiKeyRequest;
pub(crate) use client::ProviderAccountAddApiKeyResult;
pub(crate) use client::ProviderAccountImportResult;
pub(crate) use client::ProviderAccountListResult;
pub(crate) use client::ProviderAccountMutationResult;
pub(crate) use client::ProviderAccountRemovalPreviewResult;
pub(crate) use client::ProviderAccountRemovalTargetSummary;
pub(crate) use client::ProviderAccountSummary;
pub(crate) use client::RichCodexBackendClient;
pub(crate) use client::RichCodexBackendClientError;
#[cfg(test)]
use client::request_model_route_create;
#[cfg(test)]
use client::request_model_route_read;
#[cfg(test)]
use client::request_model_route_retire;
#[cfg(test)]
use client::request_model_route_set_targets;
#[cfg(test)]
use client::request_provider_account_import;
#[cfg(test)]
use client::request_provider_account_list;
pub(crate) use provider_login::ProviderAccountLoginResult;
#[cfg(test)]
use provider_login::request_provider_account_login_start;

const BACKEND_PATH_ENV: &str = "RICHCX_MODEL_BACKEND_PATH";
const BACKEND_DATA_PLANE_TOKEN_ENV: &str = "RICHCODEX_BACKEND_DATA_PLANE_TOKEN";
const BACKEND_PROTOCOL_VERSION: u32 = 9;
const MAX_PROTOCOL_LINE_BYTES: usize = 64 * 1024;
const MAX_SNAPSHOT_ITEMS: usize = 512;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const KERNEL_LOCK_JSON: &str = include_str!("../richcodex-kernel.lock.json");

/// Owns one bundled backend process and the projections shared by every app-server transport.
pub(crate) struct RichCodexBackendRuntime {
    backend: Option<RichCodexBackend>,
    client: Option<RichCodexBackendClient>,
    initial_model_routes: Vec<ModelSummary>,
    runtime_model_provider_routes: Option<RuntimeModelProviderRoutes>,
}

impl RichCodexBackendRuntime {
    pub(crate) async fn start_if_bundled(codex_home: &Path) -> io::Result<Self> {
        let app_server_executable = std::env::current_exe()?;
        Self::start(
            codex_home,
            std::env::var_os(BACKEND_PATH_ENV),
            &app_server_executable,
        )
        .await
    }

    pub(crate) async fn start(
        codex_home: &Path,
        configured_path: Option<OsString>,
        app_server_executable: &Path,
    ) -> io::Result<Self> {
        let backend =
            RichCodexBackend::start(codex_home, configured_path, app_server_executable).await?;
        if let Some(backend) = &backend {
            let snapshot = backend.snapshot();
            info!(
                backend_instance_id = %snapshot.instance_id,
                kernel_source_commit = %snapshot.kernel.source_commit,
                catalog_revision = snapshot.catalog_revision,
                provider_count = snapshot.providers.len(),
                model_count = snapshot.models.len(),
                "bundled RichCodex model backend is ready"
            );
        }
        let client = backend.as_ref().map(RichCodexBackend::client);
        let initial_model_routes = backend
            .as_ref()
            .map(|backend| backend.snapshot().models.clone())
            .unwrap_or_default();
        let runtime_model_provider_routes = backend.as_ref().map(|backend| {
            let (capability, port) = backend.data_plane();
            RuntimeModelProviderRoutes::new(
                "richcodex",
                create_private_openai_loopback_model_provider(port, capability.to_owned()),
                std::iter::empty(),
            )
        });
        Ok(Self {
            backend,
            client,
            initial_model_routes,
            runtime_model_provider_routes,
        })
    }

    pub(crate) fn client(&self) -> Option<RichCodexBackendClient> {
        self.client.clone()
    }

    pub(crate) fn initial_model_routes(&self) -> Vec<ModelSummary> {
        self.initial_model_routes.clone()
    }

    pub(crate) fn runtime_model_provider_routes(&self) -> Option<RuntimeModelProviderRoutes> {
        self.runtime_model_provider_routes.clone()
    }

    pub(crate) async fn shutdown(self) -> io::Result<()> {
        match self.backend {
            Some(backend) => backend.shutdown().await,
            None => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderSummary {
    pub id: String,
    pub display_name: String,
    pub account_count: u32,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ModelSummary {
    pub model_tag: String,
    pub display_name: String,
    pub retired: bool,
    pub semantic_model: String,
    pub targets: Vec<ModelTargetSummary>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ModelTargetSummary {
    pub id: String,
    pub provider_id: String,
    pub account_id: String,
    pub upstream_model_id: String,
    pub priority: u32,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BackendSnapshot {
    pub instance_id: String,
    pub desired_state_revision: u64,
    pub catalog_revision: u64,
    pub data_plane_port: u16,
    pub kernel: KernelProvenance,
    pub providers: Vec<ProviderSummary>,
    pub models: Vec<ModelSummary>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct KernelProvenance {
    pub source_repository: String,
    pub source_commit: String,
    pub content_digest: String,
    pub selection_digest: String,
    pub composition_version: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct KernelLock {
    schema_version: u32,
    source_repository: String,
    source_commit: String,
    archive_digest: String,
    archive_digest_recipe: String,
    license: String,
    selection_manifest: String,
    selection_digest: String,
    selection_digest_recipe: String,
    composition_version: u32,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum BackendMessage {
    Ready {
        protocol_version: u32,
        instance_id: String,
        desired_state_revision: u64,
        catalog_revision: u64,
        data_plane_port: u16,
        kernel: KernelProvenance,
        providers: Vec<ProviderSummary>,
        models: Vec<ModelSummary>,
    },
    ShutdownComplete {
        request_id: String,
    },
    ProviderAccountListResult {
        request_id: String,
        desired_state_revision: u64,
        catalog_revision: u64,
        providers: Vec<ProviderSummary>,
        data: Vec<ProviderAccountSummary>,
        next_cursor: Option<String>,
    },
    ProviderAccountImportResult {
        request_id: String,
        desired_state_revision: u64,
        catalog_revision: u64,
        account: ProviderAccountSummary,
    },
    ProviderAccountAddApiKeyResult {
        request_id: String,
        desired_state_revision: u64,
        catalog_revision: u64,
        account: ProviderAccountSummary,
    },
    ProviderAccountReplaceApiKeyResult {
        request_id: String,
        desired_state_revision: u64,
        catalog_revision: u64,
        account: ProviderAccountSummary,
    },
    ProviderAccountRemovalPreviewResult {
        request_id: String,
        desired_state_revision: u64,
        catalog_revision: u64,
        account: ProviderAccountSummary,
        affected_targets: Vec<ProviderAccountRemovalTargetSummary>,
        can_remove: bool,
    },
    ProviderAccountRemoveResult {
        request_id: String,
        desired_state_revision: u64,
        catalog_revision: u64,
        account: ProviderAccountSummary,
    },
    ProviderAccountLoginStartResult {
        request_id: String,
        login_id: String,
        status: String,
        verification_url: Option<String>,
        user_code: Option<String>,
        expires_at: u64,
        failure: Option<String>,
        account: Option<ProviderAccountSummary>,
        desired_state_revision: u64,
        catalog_revision: u64,
    },
    ProviderAccountLoginStatusResult {
        request_id: String,
        login_id: String,
        status: String,
        verification_url: Option<String>,
        user_code: Option<String>,
        expires_at: u64,
        failure: Option<String>,
        account: Option<ProviderAccountSummary>,
        desired_state_revision: u64,
        catalog_revision: u64,
    },
    ProviderAccountLoginCancelResult {
        request_id: String,
        login_id: String,
        status: String,
        verification_url: Option<String>,
        user_code: Option<String>,
        expires_at: u64,
        failure: Option<String>,
        account: Option<ProviderAccountSummary>,
        desired_state_revision: u64,
        catalog_revision: u64,
    },
    ModelRouteReadResult {
        request_id: String,
        desired_state_revision: u64,
        catalog_revision: u64,
        data: Vec<ModelSummary>,
    },
    ModelRouteCreateResult {
        request_id: String,
        desired_state_revision: u64,
        catalog_revision: u64,
        route: ModelSummary,
    },
    ModelRouteSetTargetsResult {
        request_id: String,
        desired_state_revision: u64,
        catalog_revision: u64,
        route: ModelSummary,
    },
    ModelRouteRetireResult {
        request_id: String,
        desired_state_revision: u64,
        catalog_revision: u64,
        route: ModelSummary,
    },
    OperationError {
        request_id: String,
        code: BackendOperationErrorCode,
        #[serde(rename = "message")]
        _message: String,
    },
    ProtocolError {
        #[serde(rename = "code")]
        _code: String,
    },
}

#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum AppServerMessage<'a> {
    Shutdown {
        request_id: &'a str,
    },
    ProviderAccountList {
        request_id: &'a str,
        cursor: Option<&'a str>,
        limit: Option<u32>,
    },
    ProviderAccountImport {
        request_id: &'a str,
        auth_json_path: &'a str,
        user_label: &'a str,
    },
    ProviderAccountAddApiKey {
        request_id: &'a str,
        provider_id: &'a str,
        provider_display_name: &'a str,
        api_base_url: &'a str,
        api_key: &'a str,
        user_label: &'a str,
    },
    ProviderAccountLoginStart {
        request_id: &'a str,
        user_label: &'a str,
        account_id: Option<&'a str>,
    },
    ProviderAccountReplaceApiKey {
        request_id: &'a str,
        expected_revision: u64,
        account_id: &'a str,
        api_key: &'a str,
    },
    ProviderAccountRemovalPreview {
        request_id: &'a str,
        account_id: &'a str,
    },
    ProviderAccountRemove {
        request_id: &'a str,
        expected_revision: u64,
        account_id: &'a str,
    },
    ProviderAccountLoginStatus {
        request_id: &'a str,
        login_id: &'a str,
    },
    ProviderAccountLoginCancel {
        request_id: &'a str,
        login_id: &'a str,
    },
    ModelRouteRead {
        request_id: &'a str,
    },
    ModelRouteCreate {
        request_id: &'a str,
        expected_revision: u64,
        model_tag: &'a str,
        display_name: &'a str,
        semantic_model: &'a str,
        provider_id: &'a str,
        account_id: &'a str,
        upstream_model_id: &'a str,
    },
    ModelRouteSetTargets {
        request_id: &'a str,
        expected_revision: u64,
        model_tag: &'a str,
        targets: &'a [ModelRouteTargetRequest],
    },
    ModelRouteRetire {
        request_id: &'a str,
        expected_revision: u64,
        model_tag: &'a str,
    },
}

pub(crate) struct RichCodexBackend {
    snapshot: BackendSnapshot,
    data_plane_capability: String,
    client: RichCodexBackendClient,
    actor: JoinHandle<io::Result<()>>,
}

impl RichCodexBackend {
    async fn start(
        codex_home: &Path,
        configured_path: Option<OsString>,
        app_server_executable: &Path,
    ) -> io::Result<Option<Self>> {
        let Some(executable) = resolve_backend_executable(configured_path, app_server_executable)?
        else {
            return Ok(None);
        };

        let state_root = codex_home.join("richcodex").join("model-backend");
        let data_plane_capability = format!("{}.{}", Uuid::now_v7(), Uuid::now_v7());
        let mut command = backend_command(&executable, &state_root, &data_plane_capability);
        let mut child = command.spawn().map_err(|err| {
            io::Error::new(
                err.kind(),
                format!(
                    "failed to start bundled RichCodex model backend at {}: {err}",
                    executable.display()
                ),
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("RichCodex model backend stdin was not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("RichCodex model backend stdout was not piped"))?;
        let mut stdout = BufReader::new(stdout);
        let snapshot = match read_ready(&mut stdout, STARTUP_TIMEOUT).await {
            Ok(snapshot) => snapshot,
            Err(err) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(err);
            }
        };

        let (client, actor) = RichCodexBackendClient::spawn(child, stdin, stdout);

        Ok(Some(Self {
            snapshot,
            data_plane_capability,
            client,
            actor,
        }))
    }

    pub(crate) fn snapshot(&self) -> &BackendSnapshot {
        &self.snapshot
    }

    pub(crate) fn client(&self) -> RichCodexBackendClient {
        self.client.clone()
    }

    pub(crate) fn data_plane(&self) -> (&str, u16) {
        (&self.data_plane_capability, self.snapshot.data_plane_port)
    }

    pub(crate) async fn shutdown(self) -> io::Result<()> {
        let shutdown_result = self.client.shutdown().await;
        drop(self.client);
        let actor_result = self
            .actor
            .await
            .map_err(|_| io::Error::other("RichCodex model backend actor failed"))?;
        shutdown_result.and(actor_result)
    }
}

fn resolve_backend_executable(
    configured_path: Option<OsString>,
    app_server_executable: &Path,
) -> io::Result<Option<PathBuf>> {
    if let Some(configured_path) = configured_path.filter(|value| !value.is_empty()) {
        let configured_path = PathBuf::from(configured_path);
        if !configured_path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{BACKEND_PATH_ENV} must name an absolute executable path"),
            ));
        }
        return Ok(Some(configured_path));
    }

    let Some(parent) = app_server_executable.parent() else {
        return Ok(None);
    };
    let sibling = parent.join(if cfg!(windows) {
        "richcodex-model-backend.exe"
    } else {
        "richcodex-model-backend"
    });
    Ok(sibling.is_file().then_some(sibling))
}

fn backend_command(executable: &Path, state_root: &Path, data_plane_capability: &str) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("--state-root")
        .arg(state_root)
        .env_clear()
        .env(BACKEND_DATA_PLANE_TOKEN_ENV, data_plane_capability)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    copy_safe_process_environment(&mut command, "PATH");
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ] {
        copy_safe_process_environment(&mut command, name);
    }
    #[cfg(windows)]
    {
        copy_safe_process_environment(&mut command, "SYSTEMROOT");
        copy_safe_process_environment(&mut command, "WINDIR");
    }
    command
}

fn copy_safe_process_environment(command: &mut Command, name: &str) {
    if let Some(value) = std::env::var_os(name) {
        command.env(name, value);
    }
}

async fn read_ready<R>(reader: &mut R, wait: Duration) -> io::Result<BackendSnapshot>
where
    R: AsyncBufRead + Unpin,
{
    let message = read_message(reader, wait).await?;
    let BackendMessage::Ready {
        protocol_version,
        instance_id,
        desired_state_revision,
        catalog_revision,
        data_plane_port,
        kernel,
        providers,
        models,
    } = message
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend did not send a ready handshake",
        ));
    };
    if protocol_version != BACKEND_PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported model backend protocol version {protocol_version}; expected {BACKEND_PROTOCOL_VERSION}"
            ),
        ));
    }
    if data_plane_port == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid data-plane port",
        ));
    }
    validate_snapshot(&instance_id, &kernel, &providers, &models)?;
    Ok(BackendSnapshot {
        instance_id,
        desired_state_revision,
        catalog_revision,
        data_plane_port,
        kernel,
        providers,
        models,
    })
}

async fn read_shutdown_complete<R>(
    reader: &mut R,
    expected_request_id: &str,
    wait: Duration,
) -> io::Result<()>
where
    R: AsyncBufRead + Unpin,
{
    match read_message(reader, wait).await? {
        BackendMessage::ShutdownComplete { request_id } if request_id == expected_request_id => {
            Ok(())
        }
        BackendMessage::ProtocolError { .. } => {
            Err(io::Error::other("model backend rejected shutdown"))
        }
        BackendMessage::ShutdownComplete { .. }
        | BackendMessage::Ready { .. }
        | BackendMessage::ProviderAccountListResult { .. }
        | BackendMessage::ProviderAccountImportResult { .. }
        | BackendMessage::ProviderAccountAddApiKeyResult { .. }
        | BackendMessage::ProviderAccountReplaceApiKeyResult { .. }
        | BackendMessage::ProviderAccountRemovalPreviewResult { .. }
        | BackendMessage::ProviderAccountRemoveResult { .. }
        | BackendMessage::ProviderAccountLoginStartResult { .. }
        | BackendMessage::ProviderAccountLoginStatusResult { .. }
        | BackendMessage::ProviderAccountLoginCancelResult { .. }
        | BackendMessage::ModelRouteReadResult { .. }
        | BackendMessage::ModelRouteCreateResult { .. }
        | BackendMessage::ModelRouteSetTargetsResult { .. }
        | BackendMessage::ModelRouteRetireResult { .. }
        | BackendMessage::OperationError { .. } => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid shutdown acknowledgement",
        )),
    }
}

async fn read_message<R>(reader: &mut R, wait: Duration) -> io::Result<BackendMessage>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
    let bytes_read = timeout(
        wait,
        reader
            .take((MAX_PROTOCOL_LINE_BYTES + 1) as u64)
            .read_until(b'\n', &mut line),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "model backend handshake timed out"))??;
    if bytes_read == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "model backend closed its protocol stream",
        ));
    }
    if line.len() > MAX_PROTOCOL_LINE_BYTES || line.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an oversized or unterminated protocol message",
        ));
    }
    serde_json::from_slice(&line).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent a malformed protocol message",
        )
    })
}

async fn write_message<W, T>(writer: &mut W, message: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut encoded = serde_json::to_vec(message).map_err(io::Error::other)?;
    encoded.push(b'\n');
    writer.write_all(&encoded).await?;
    writer.flush().await
}

fn validate_snapshot(
    instance_id: &str,
    kernel: &KernelProvenance,
    providers: &[ProviderSummary],
    models: &[ModelSummary],
) -> io::Result<()> {
    if instance_id.is_empty()
        || instance_id.len() > 128
        || !instance_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid instance id",
        ));
    }
    let expected_kernel = expected_kernel_provenance()?;
    if kernel != &expected_kernel {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend kernel provenance does not match this RichCodex build",
        ));
    }
    if providers.len() > MAX_SNAPSHOT_ITEMS || models.len() > MAX_SNAPSHOT_ITEMS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend snapshot exceeded its item limit",
        ));
    }
    let mut provider_ids = HashSet::with_capacity(providers.len());
    for provider in providers {
        client::validate_provider(provider)?;
        if !provider_ids.insert(provider.id.as_str()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "model backend snapshot contained a duplicate provider id",
            ));
        }
    }
    let mut model_tags = HashSet::with_capacity(models.len());
    let mut target_ids = HashSet::new();
    let mut target_count = 0usize;
    for model in models {
        client::validate_model_route(model)?;
        if !model_tags.insert(model.model_tag.as_str()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "model backend snapshot contained a duplicate model tag",
            ));
        }
        target_count = target_count
            .checked_add(model.targets.len())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "model backend snapshot exceeded its target limit",
                )
            })?;
        if target_count > MAX_SNAPSHOT_ITEMS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "model backend snapshot exceeded its target limit",
            ));
        }
        for target in &model.targets {
            if !provider_ids.contains(target.provider_id.as_str()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "model backend snapshot referenced an unknown provider",
                ));
            }
            if !target_ids.insert(target.id.as_str()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "model backend snapshot contained a duplicate target id",
                ));
            }
        }
    }
    Ok(())
}

fn expected_kernel_provenance() -> io::Result<KernelProvenance> {
    let lock: KernelLock = serde_json::from_str(KERNEL_LOCK_JSON).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "this RichCodex build contains an invalid kernel lock",
        )
    })?;
    if lock.schema_version != 1
        || lock.archive_digest_recipe != "git archive --format=tar <sourceCommit> | sha256sum"
        || lock.license != "MIT"
        || lock.selection_manifest != "richcodex-model-backend/kernel-selection.json"
        || lock.selection_digest_recipe != "sha256sum richcodex-model-backend/kernel-selection.json"
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "this RichCodex build contains an unsupported kernel lock",
        ));
    }
    Ok(KernelProvenance {
        source_repository: lock.source_repository,
        source_commit: lock.source_commit,
        content_digest: lock.archive_digest,
        selection_digest: lock.selection_digest,
        composition_version: lock.composition_version,
    })
}

#[cfg(test)]
#[path = "richcodex_backend_tests.rs"]
mod tests;
