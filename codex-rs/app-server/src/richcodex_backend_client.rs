use super::AppServerMessage;
use super::BackendMessage;
use super::MAX_SNAPSHOT_ITEMS;
use super::ProviderSummary;
use super::REQUEST_TIMEOUT;
use super::SHUTDOWN_TIMEOUT;
use super::read_message;
use super::read_shutdown_complete;
use super::write_message;
use serde::Deserialize;
use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use tokio::io::AsyncBufRead;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::process::Child;
use tokio::process::ChildStdin;
use tokio::process::ChildStdout;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const COMMAND_CHANNEL_CAPACITY: usize = 16;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderAccountSummary {
    pub id: String,
    pub provider_id: String,
    pub user_label: String,
    pub status: String,
    pub added_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderAccountListResult {
    pub desired_state_revision: u64,
    pub catalog_revision: u64,
    pub providers: Vec<ProviderSummary>,
    pub data: Vec<ProviderAccountSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderAccountImportResult {
    pub desired_state_revision: u64,
    pub catalog_revision: u64,
    pub account: ProviderAccountSummary,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum BackendOperationErrorCode {
    SourceUnavailable,
    SourceTooLarge,
    InvalidAuthDocument,
    CredentialExpired,
    AccountAlreadyExists,
    AccountLimitReached,
    StoreUnavailable,
    InvalidRequest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RichCodexBackendClientError {
    Unavailable,
    SourceUnavailable,
    SourceTooLarge,
    InvalidAuthDocument,
    CredentialExpired,
    AccountAlreadyExists,
    AccountLimitReached,
    StoreUnavailable,
    InvalidRequest,
}

impl From<BackendOperationErrorCode> for RichCodexBackendClientError {
    fn from(value: BackendOperationErrorCode) -> Self {
        match value {
            BackendOperationErrorCode::SourceUnavailable => Self::SourceUnavailable,
            BackendOperationErrorCode::SourceTooLarge => Self::SourceTooLarge,
            BackendOperationErrorCode::InvalidAuthDocument => Self::InvalidAuthDocument,
            BackendOperationErrorCode::CredentialExpired => Self::CredentialExpired,
            BackendOperationErrorCode::AccountAlreadyExists => Self::AccountAlreadyExists,
            BackendOperationErrorCode::AccountLimitReached => Self::AccountLimitReached,
            BackendOperationErrorCode::StoreUnavailable => Self::StoreUnavailable,
            BackendOperationErrorCode::InvalidRequest => Self::InvalidRequest,
        }
    }
}

enum BackendCommand {
    List {
        request_id: String,
        cursor: Option<String>,
        limit: Option<u32>,
        response: oneshot::Sender<Result<ProviderAccountListResult, RichCodexBackendClientError>>,
    },
    Import {
        request_id: String,
        auth_json_path: String,
        user_label: String,
        response: oneshot::Sender<Result<ProviderAccountImportResult, RichCodexBackendClientError>>,
    },
    Shutdown {
        request_id: String,
        response: oneshot::Sender<io::Result<()>>,
    },
}

#[derive(Clone)]
pub(crate) struct RichCodexBackendClient {
    commands: mpsc::Sender<BackendCommand>,
    next_request_id: Arc<AtomicU64>,
}

impl RichCodexBackendClient {
    pub(super) fn spawn(
        child: Child,
        stdin: ChildStdin,
        stdout: BufReader<ChildStdout>,
    ) -> (Self, JoinHandle<io::Result<()>>) {
        let (commands, command_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let client = Self {
            commands,
            next_request_id: Arc::new(AtomicU64::new(1)),
        };
        let actor = tokio::spawn(run_backend_actor(child, stdin, stdout, command_rx));
        (client, actor)
    }

    fn request_id(&self) -> String {
        format!(
            "app-server-{}",
            self.next_request_id.fetch_add(1, Ordering::Relaxed)
        )
    }

    pub(crate) async fn list_provider_accounts(
        &self,
        cursor: Option<String>,
        limit: Option<u32>,
    ) -> Result<ProviderAccountListResult, RichCodexBackendClientError> {
        let (response, received) = oneshot::channel();
        self.commands
            .send(BackendCommand::List {
                request_id: self.request_id(),
                cursor,
                limit,
                response,
            })
            .await
            .map_err(|_| RichCodexBackendClientError::Unavailable)?;
        received
            .await
            .unwrap_or(Err(RichCodexBackendClientError::Unavailable))
    }

    pub(crate) async fn import_provider_account(
        &self,
        auth_json_path: String,
        user_label: String,
    ) -> Result<ProviderAccountImportResult, RichCodexBackendClientError> {
        let (response, received) = oneshot::channel();
        self.commands
            .send(BackendCommand::Import {
                request_id: self.request_id(),
                auth_json_path,
                user_label,
                response,
            })
            .await
            .map_err(|_| RichCodexBackendClientError::Unavailable)?;
        received
            .await
            .unwrap_or(Err(RichCodexBackendClientError::Unavailable))
    }

    pub(super) async fn shutdown(&self) -> io::Result<()> {
        let (response, received) = oneshot::channel();
        self.commands
            .send(BackendCommand::Shutdown {
                request_id: self.request_id(),
                response,
            })
            .await
            .map_err(|_| io::Error::other("RichCodex model backend is unavailable for shutdown"))?;
        received.await.unwrap_or_else(|_| {
            Err(io::Error::other(
                "RichCodex model backend shutdown channel closed",
            ))
        })
    }
}

async fn run_backend_actor(
    mut child: Child,
    mut stdin: ChildStdin,
    mut stdout: BufReader<ChildStdout>,
    mut commands: mpsc::Receiver<BackendCommand>,
) -> io::Result<()> {
    while let Some(command) = commands.recv().await {
        match command {
            BackendCommand::List {
                request_id,
                cursor,
                limit,
                response,
            } => {
                let result = request_provider_account_list(
                    &mut stdin,
                    &mut stdout,
                    &request_id,
                    cursor.as_deref(),
                    limit,
                )
                .await;
                let is_fatal = matches!(&result, Err(RichCodexBackendClientError::Unavailable));
                let _ = response.send(result);
                if is_fatal {
                    stop_child(&mut child).await;
                    return Err(io::Error::other(
                        "RichCodex model backend became unavailable",
                    ));
                }
            }
            BackendCommand::Import {
                request_id,
                auth_json_path,
                user_label,
                response,
            } => {
                let result = request_provider_account_import(
                    &mut stdin,
                    &mut stdout,
                    &request_id,
                    &auth_json_path,
                    &user_label,
                )
                .await;
                let is_fatal = matches!(&result, Err(RichCodexBackendClientError::Unavailable));
                let _ = response.send(result);
                if is_fatal {
                    stop_child(&mut child).await;
                    return Err(io::Error::other(
                        "RichCodex model backend became unavailable",
                    ));
                }
            }
            BackendCommand::Shutdown {
                request_id,
                response,
            } => {
                let result = shutdown_child(&mut child, &mut stdin, &mut stdout, &request_id).await;
                let returned = result
                    .as_ref()
                    .map(|_| ())
                    .map_err(|err| io::Error::new(err.kind(), err.to_string()));
                let _ = response.send(returned);
                return result;
            }
        }
    }

    stop_child(&mut child).await;
    Ok(())
}

async fn stop_child(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn shutdown_child(
    child: &mut Child,
    stdin: &mut ChildStdin,
    stdout: &mut BufReader<ChildStdout>,
    request_id: &str,
) -> io::Result<()> {
    write_message(stdin, &AppServerMessage::Shutdown { request_id }).await?;
    read_shutdown_complete(stdout, request_id, SHUTDOWN_TIMEOUT).await?;
    stdin.shutdown().await?;
    let status = match timeout(SHUTDOWN_TIMEOUT, child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            stop_child(child).await;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "model backend did not exit",
            ));
        }
    };
    if !status.success() {
        return Err(io::Error::other(format!(
            "model backend exited with {status}"
        )));
    }
    Ok(())
}

pub(super) async fn request_provider_account_list<W, R>(
    writer: &mut W,
    reader: &mut R,
    request_id: &str,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> Result<ProviderAccountListResult, RichCodexBackendClientError>
where
    W: AsyncWrite + Unpin,
    R: AsyncBufRead + Unpin,
{
    write_message(
        writer,
        &AppServerMessage::ProviderAccountList {
            request_id,
            cursor,
            limit,
        },
    )
    .await
    .map_err(|_| RichCodexBackendClientError::Unavailable)?;
    match read_message(reader, REQUEST_TIMEOUT)
        .await
        .map_err(|_| RichCodexBackendClientError::Unavailable)?
    {
        BackendMessage::ProviderAccountListResult {
            request_id: returned_request_id,
            desired_state_revision,
            catalog_revision,
            providers,
            data,
            next_cursor,
        } if returned_request_id == request_id => {
            validate_provider_accounts(&providers, &data, next_cursor.as_deref())
                .map_err(|_| RichCodexBackendClientError::Unavailable)?;
            Ok(ProviderAccountListResult {
                desired_state_revision,
                catalog_revision,
                providers,
                data,
                next_cursor,
            })
        }
        BackendMessage::OperationError {
            request_id: returned_request_id,
            code,
            ..
        } if returned_request_id == request_id => Err(code.into()),
        _ => Err(RichCodexBackendClientError::Unavailable),
    }
}

pub(super) async fn request_provider_account_import<W, R>(
    writer: &mut W,
    reader: &mut R,
    request_id: &str,
    auth_json_path: &str,
    user_label: &str,
) -> Result<ProviderAccountImportResult, RichCodexBackendClientError>
where
    W: AsyncWrite + Unpin,
    R: AsyncBufRead + Unpin,
{
    write_message(
        writer,
        &AppServerMessage::ProviderAccountImport {
            request_id,
            auth_json_path,
            user_label,
        },
    )
    .await
    .map_err(|_| RichCodexBackendClientError::Unavailable)?;
    match read_message(reader, REQUEST_TIMEOUT)
        .await
        .map_err(|_| RichCodexBackendClientError::Unavailable)?
    {
        BackendMessage::ProviderAccountImportResult {
            request_id: returned_request_id,
            desired_state_revision,
            catalog_revision,
            account,
        } if returned_request_id == request_id => {
            validate_provider_account(&account)
                .map_err(|_| RichCodexBackendClientError::Unavailable)?;
            Ok(ProviderAccountImportResult {
                desired_state_revision,
                catalog_revision,
                account,
            })
        }
        BackendMessage::OperationError {
            request_id: returned_request_id,
            code,
            ..
        } if returned_request_id == request_id => Err(code.into()),
        _ => Err(RichCodexBackendClientError::Unavailable),
    }
}

fn validate_provider_accounts(
    providers: &[ProviderSummary],
    accounts: &[ProviderAccountSummary],
    next_cursor: Option<&str>,
) -> io::Result<()> {
    if providers.len() > MAX_SNAPSHOT_ITEMS || accounts.len() > 100 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend provider-account response exceeded its item limit",
        ));
    }
    for provider in providers {
        validate_provider(provider)?;
    }
    for account in accounts {
        validate_provider_account(account)?;
    }
    if let Some(cursor) = next_cursor {
        validate_safe_text(cursor, 64)?;
    }
    Ok(())
}

fn validate_provider(provider: &ProviderSummary) -> io::Result<()> {
    validate_safe_text(&provider.id, 256)?;
    validate_safe_text(&provider.display_name, 256)?;
    if !matches!(provider.status.as_str(), "ready" | "needsAccount") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid provider status",
        ));
    }
    Ok(())
}

fn validate_provider_account(account: &ProviderAccountSummary) -> io::Result<()> {
    validate_safe_text(&account.id, 80)?;
    validate_safe_text(&account.provider_id, 256)?;
    validate_safe_text(&account.user_label, 80)?;
    if i64::try_from(account.added_at / 1000).is_err() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid provider-account timestamp",
        ));
    }
    if !matches!(
        account.status.as_str(),
        "verificationRequired" | "reauthenticationRequired"
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid provider-account status",
        ));
    }
    Ok(())
}

fn validate_safe_text(value: &str, max_bytes: usize) -> io::Result<()> {
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid safe-display value",
        ));
    }
    Ok(())
}
