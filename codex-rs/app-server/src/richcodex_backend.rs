use serde::Deserialize;
use serde::Serialize;
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
use tokio::process::Child;
use tokio::process::ChildStdin;
use tokio::process::ChildStdout;
use tokio::process::Command;
use tokio::time::timeout;

const BACKEND_PATH_ENV: &str = "RICHCX_MODEL_BACKEND_PATH";
const BACKEND_PROTOCOL_VERSION: u32 = 1;
const MAX_PROTOCOL_LINE_BYTES: usize = 64 * 1024;
const MAX_SNAPSHOT_ITEMS: usize = 512;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const KERNEL_LOCK_JSON: &str = include_str!("../richcodex-kernel.lock.json");

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
    pub tag: String,
    pub display_name: String,
    pub available: bool,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BackendSnapshot {
    pub instance_id: String,
    pub catalog_revision: u64,
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
        catalog_revision: u64,
        kernel: KernelProvenance,
        providers: Vec<ProviderSummary>,
        models: Vec<ModelSummary>,
    },
    ShutdownComplete {
        request_id: String,
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
    Shutdown { request_id: &'a str },
}

pub(crate) struct RichCodexBackend {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    snapshot: BackendSnapshot,
}

impl RichCodexBackend {
    pub(crate) async fn start_if_bundled(codex_home: &Path) -> io::Result<Option<Self>> {
        let Some(executable) = resolve_backend_executable(
            std::env::var_os(BACKEND_PATH_ENV),
            std::env::current_exe()?.as_path(),
        )?
        else {
            return Ok(None);
        };

        let state_root = codex_home.join("richcodex").join("model-backend");
        let mut command = backend_command(&executable, &state_root);
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

        Ok(Some(Self {
            child,
            stdin,
            stdout,
            snapshot,
        }))
    }

    pub(crate) fn snapshot(&self) -> &BackendSnapshot {
        &self.snapshot
    }

    pub(crate) async fn shutdown(mut self) -> io::Result<()> {
        let request_id = "app-server-shutdown";
        write_message(&mut self.stdin, &AppServerMessage::Shutdown { request_id }).await?;
        read_shutdown_complete(&mut self.stdout, request_id, SHUTDOWN_TIMEOUT).await?;
        drop(self.stdin);
        let status = timeout(SHUTDOWN_TIMEOUT, self.child.wait())
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "model backend did not exit"))??;
        if !status.success() {
            return Err(io::Error::other(format!(
                "model backend exited with {status}"
            )));
        }
        Ok(())
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

fn backend_command(executable: &Path, state_root: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("--state-root")
        .arg(state_root)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    copy_safe_process_environment(&mut command, "PATH");
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
        catalog_revision,
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
    validate_snapshot(&instance_id, &kernel, &providers, &models)?;
    Ok(BackendSnapshot {
        instance_id,
        catalog_revision,
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
        BackendMessage::ShutdownComplete { .. } | BackendMessage::Ready { .. } => {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "model backend sent an invalid shutdown acknowledgement",
            ))
        }
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
    for value in providers
        .iter()
        .flat_map(|provider| [&provider.id, &provider.display_name, &provider.status])
        .chain(
            models
                .iter()
                .flat_map(|model| [&model.tag, &model.display_name]),
        )
        .chain(models.iter().flat_map(|model| model.capabilities.iter()))
    {
        if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "model backend snapshot contained an invalid safe-display value",
            ));
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
        composition_version: lock.composition_version,
    })
}

#[cfg(test)]
#[path = "richcodex_backend_tests.rs"]
mod tests;
