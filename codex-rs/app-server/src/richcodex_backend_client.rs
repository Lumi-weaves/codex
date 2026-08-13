use super::AppServerMessage;
use super::BackendMessage;
use super::MAX_SNAPSHOT_ITEMS;
use super::ModelSummary;
use super::ProviderSummary;
use super::REQUEST_TIMEOUT;
use super::SHUTDOWN_TIMEOUT;
use super::provider_login::ProviderAccountLoginResult;
use super::provider_login::request_provider_account_login_cancel;
use super::provider_login::request_provider_account_login_start;
use super::provider_login::request_provider_account_login_status;
use super::read_message;
use super::read_shutdown_complete;
use super::write_message;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashSet;
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
    pub credential_kind: String,
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

pub(crate) type ProviderAccountAddApiKeyResult = ProviderAccountImportResult;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModelRouteReadResult {
    pub desired_state_revision: u64,
    pub catalog_revision: u64,
    pub data: Vec<ModelSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModelRouteMutationResult {
    pub desired_state_revision: u64,
    pub catalog_revision: u64,
    pub route: ModelSummary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModelRouteCreateRequest {
    pub expected_revision: u64,
    pub model_tag: String,
    pub display_name: String,
    pub semantic_model: String,
    pub provider_id: String,
    pub account_id: String,
    pub upstream_model_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelRouteTargetRequest {
    pub id: Option<String>,
    pub provider_id: String,
    pub account_id: String,
    pub upstream_model_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModelRouteSetTargetsRequest {
    pub expected_revision: u64,
    pub model_tag: String,
    pub targets: Vec<ModelRouteTargetRequest>,
}

#[derive(Clone, Copy)]
enum ModelRouteMutationKind {
    Create,
    SetTargets,
    Retire,
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
    InvalidApiKey,
    LoginUnavailable,
    LoginLimitReached,
    LoginNotFound,
    StoreUnavailable,
    InvalidRequest,
    RevisionConflict,
    ModelTagExists,
    ModelTagNotFound,
    AccountUnavailable,
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
    InvalidApiKey,
    LoginUnavailable,
    LoginLimitReached,
    LoginNotFound,
    StoreUnavailable,
    InvalidRequest,
    RevisionConflict,
    ModelTagExists,
    ModelTagNotFound,
    AccountUnavailable,
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
            BackendOperationErrorCode::InvalidApiKey => Self::InvalidApiKey,
            BackendOperationErrorCode::LoginUnavailable => Self::LoginUnavailable,
            BackendOperationErrorCode::LoginLimitReached => Self::LoginLimitReached,
            BackendOperationErrorCode::LoginNotFound => Self::LoginNotFound,
            BackendOperationErrorCode::StoreUnavailable => Self::StoreUnavailable,
            BackendOperationErrorCode::InvalidRequest => Self::InvalidRequest,
            BackendOperationErrorCode::RevisionConflict => Self::RevisionConflict,
            BackendOperationErrorCode::ModelTagExists => Self::ModelTagExists,
            BackendOperationErrorCode::ModelTagNotFound => Self::ModelTagNotFound,
            BackendOperationErrorCode::AccountUnavailable => Self::AccountUnavailable,
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
    AddApiKey {
        request_id: String,
        api_key: String,
        user_label: String,
        response:
            oneshot::Sender<Result<ProviderAccountAddApiKeyResult, RichCodexBackendClientError>>,
    },
    StartLogin {
        request_id: String,
        user_label: String,
        response: oneshot::Sender<Result<ProviderAccountLoginResult, RichCodexBackendClientError>>,
    },
    ReadLogin {
        request_id: String,
        login_id: String,
        response: oneshot::Sender<Result<ProviderAccountLoginResult, RichCodexBackendClientError>>,
    },
    CancelLogin {
        request_id: String,
        login_id: String,
        response: oneshot::Sender<Result<ProviderAccountLoginResult, RichCodexBackendClientError>>,
    },
    ReadModelRoutes {
        request_id: String,
        response: oneshot::Sender<Result<ModelRouteReadResult, RichCodexBackendClientError>>,
    },
    CreateModelRoute {
        request_id: String,
        request: ModelRouteCreateRequest,
        response: oneshot::Sender<Result<ModelRouteMutationResult, RichCodexBackendClientError>>,
    },
    SetModelRouteTargets {
        request_id: String,
        request: ModelRouteSetTargetsRequest,
        response: oneshot::Sender<Result<ModelRouteMutationResult, RichCodexBackendClientError>>,
    },
    RetireModelRoute {
        request_id: String,
        expected_revision: u64,
        model_tag: String,
        response: oneshot::Sender<Result<ModelRouteMutationResult, RichCodexBackendClientError>>,
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

    pub(crate) async fn add_api_key_provider_account(
        &self,
        api_key: String,
        user_label: String,
    ) -> Result<ProviderAccountAddApiKeyResult, RichCodexBackendClientError> {
        let (response, received) = oneshot::channel();
        self.commands
            .send(BackendCommand::AddApiKey {
                request_id: self.request_id(),
                api_key,
                user_label,
                response,
            })
            .await
            .map_err(|_| RichCodexBackendClientError::Unavailable)?;
        received
            .await
            .unwrap_or(Err(RichCodexBackendClientError::Unavailable))
    }

    pub(crate) async fn start_provider_account_login(
        &self,
        user_label: String,
    ) -> Result<ProviderAccountLoginResult, RichCodexBackendClientError> {
        let (response, received) = oneshot::channel();
        self.commands
            .send(BackendCommand::StartLogin {
                request_id: self.request_id(),
                user_label,
                response,
            })
            .await
            .map_err(|_| RichCodexBackendClientError::Unavailable)?;
        received
            .await
            .unwrap_or(Err(RichCodexBackendClientError::Unavailable))
    }

    pub(crate) async fn provider_account_login_status(
        &self,
        login_id: String,
    ) -> Result<ProviderAccountLoginResult, RichCodexBackendClientError> {
        let (response, received) = oneshot::channel();
        self.commands
            .send(BackendCommand::ReadLogin {
                request_id: self.request_id(),
                login_id,
                response,
            })
            .await
            .map_err(|_| RichCodexBackendClientError::Unavailable)?;
        received
            .await
            .unwrap_or(Err(RichCodexBackendClientError::Unavailable))
    }

    pub(crate) async fn cancel_provider_account_login(
        &self,
        login_id: String,
    ) -> Result<ProviderAccountLoginResult, RichCodexBackendClientError> {
        let (response, received) = oneshot::channel();
        self.commands
            .send(BackendCommand::CancelLogin {
                request_id: self.request_id(),
                login_id,
                response,
            })
            .await
            .map_err(|_| RichCodexBackendClientError::Unavailable)?;
        received
            .await
            .unwrap_or(Err(RichCodexBackendClientError::Unavailable))
    }

    pub(crate) async fn read_model_routes(
        &self,
    ) -> Result<ModelRouteReadResult, RichCodexBackendClientError> {
        let (response, received) = oneshot::channel();
        self.commands
            .send(BackendCommand::ReadModelRoutes {
                request_id: self.request_id(),
                response,
            })
            .await
            .map_err(|_| RichCodexBackendClientError::Unavailable)?;
        received
            .await
            .unwrap_or(Err(RichCodexBackendClientError::Unavailable))
    }

    pub(crate) async fn create_model_route(
        &self,
        request: ModelRouteCreateRequest,
    ) -> Result<ModelRouteMutationResult, RichCodexBackendClientError> {
        let (response, received) = oneshot::channel();
        self.commands
            .send(BackendCommand::CreateModelRoute {
                request_id: self.request_id(),
                request,
                response,
            })
            .await
            .map_err(|_| RichCodexBackendClientError::Unavailable)?;
        received
            .await
            .unwrap_or(Err(RichCodexBackendClientError::Unavailable))
    }

    pub(crate) async fn retire_model_route(
        &self,
        expected_revision: u64,
        model_tag: String,
    ) -> Result<ModelRouteMutationResult, RichCodexBackendClientError> {
        let (response, received) = oneshot::channel();
        self.commands
            .send(BackendCommand::RetireModelRoute {
                request_id: self.request_id(),
                expected_revision,
                model_tag,
                response,
            })
            .await
            .map_err(|_| RichCodexBackendClientError::Unavailable)?;
        received
            .await
            .unwrap_or(Err(RichCodexBackendClientError::Unavailable))
    }

    pub(crate) async fn set_model_route_targets(
        &self,
        request: ModelRouteSetTargetsRequest,
    ) -> Result<ModelRouteMutationResult, RichCodexBackendClientError> {
        let (response, received) = oneshot::channel();
        self.commands
            .send(BackendCommand::SetModelRouteTargets {
                request_id: self.request_id(),
                request,
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
            BackendCommand::AddApiKey {
                request_id,
                api_key,
                user_label,
                response,
            } => {
                let result = request_provider_account_add_api_key(
                    &mut stdin,
                    &mut stdout,
                    &request_id,
                    &api_key,
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
            BackendCommand::StartLogin {
                request_id,
                user_label,
                response,
            } => {
                let result = request_provider_account_login_start(
                    &mut stdin,
                    &mut stdout,
                    &request_id,
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
            BackendCommand::ReadLogin {
                request_id,
                login_id,
                response,
            } => {
                let result = request_provider_account_login_status(
                    &mut stdin,
                    &mut stdout,
                    &request_id,
                    &login_id,
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
            BackendCommand::CancelLogin {
                request_id,
                login_id,
                response,
            } => {
                let result = request_provider_account_login_cancel(
                    &mut stdin,
                    &mut stdout,
                    &request_id,
                    &login_id,
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
            BackendCommand::ReadModelRoutes {
                request_id,
                response,
            } => {
                let result = request_model_route_read(&mut stdin, &mut stdout, &request_id).await;
                let is_fatal = matches!(&result, Err(RichCodexBackendClientError::Unavailable));
                let _ = response.send(result);
                if is_fatal {
                    stop_child(&mut child).await;
                    return Err(io::Error::other(
                        "RichCodex model backend became unavailable",
                    ));
                }
            }
            BackendCommand::CreateModelRoute {
                request_id,
                request,
                response,
            } => {
                let result =
                    request_model_route_create(&mut stdin, &mut stdout, &request_id, &request)
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
            BackendCommand::SetModelRouteTargets {
                request_id,
                request,
                response,
            } => {
                let result =
                    request_model_route_set_targets(&mut stdin, &mut stdout, &request_id, &request)
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
            BackendCommand::RetireModelRoute {
                request_id,
                expected_revision,
                model_tag,
                response,
            } => {
                let result = request_model_route_retire(
                    &mut stdin,
                    &mut stdout,
                    &request_id,
                    expected_revision,
                    &model_tag,
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

pub(super) async fn request_provider_account_add_api_key<W, R>(
    writer: &mut W,
    reader: &mut R,
    request_id: &str,
    api_key: &str,
    user_label: &str,
) -> Result<ProviderAccountAddApiKeyResult, RichCodexBackendClientError>
where
    W: AsyncWrite + Unpin,
    R: AsyncBufRead + Unpin,
{
    write_message(
        writer,
        &AppServerMessage::ProviderAccountAddApiKey {
            request_id,
            api_key,
            user_label,
        },
    )
    .await
    .map_err(|_| RichCodexBackendClientError::Unavailable)?;
    match read_message(reader, REQUEST_TIMEOUT)
        .await
        .map_err(|_| RichCodexBackendClientError::Unavailable)?
    {
        BackendMessage::ProviderAccountAddApiKeyResult {
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

pub(super) async fn request_model_route_read<W, R>(
    writer: &mut W,
    reader: &mut R,
    request_id: &str,
) -> Result<ModelRouteReadResult, RichCodexBackendClientError>
where
    W: AsyncWrite + Unpin,
    R: AsyncBufRead + Unpin,
{
    write_message(writer, &AppServerMessage::ModelRouteRead { request_id })
        .await
        .map_err(|_| RichCodexBackendClientError::Unavailable)?;
    match read_message(reader, REQUEST_TIMEOUT)
        .await
        .map_err(|_| RichCodexBackendClientError::Unavailable)?
    {
        BackendMessage::ModelRouteReadResult {
            request_id: returned_request_id,
            desired_state_revision,
            catalog_revision,
            data,
        } if returned_request_id == request_id => {
            validate_model_routes(&data).map_err(|_| RichCodexBackendClientError::Unavailable)?;
            Ok(ModelRouteReadResult {
                desired_state_revision,
                catalog_revision,
                data,
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

pub(super) async fn request_model_route_create<W, R>(
    writer: &mut W,
    reader: &mut R,
    request_id: &str,
    request: &ModelRouteCreateRequest,
) -> Result<ModelRouteMutationResult, RichCodexBackendClientError>
where
    W: AsyncWrite + Unpin,
    R: AsyncBufRead + Unpin,
{
    write_message(
        writer,
        &AppServerMessage::ModelRouteCreate {
            request_id,
            expected_revision: request.expected_revision,
            model_tag: &request.model_tag,
            display_name: &request.display_name,
            semantic_model: &request.semantic_model,
            provider_id: &request.provider_id,
            account_id: &request.account_id,
            upstream_model_id: &request.upstream_model_id,
        },
    )
    .await
    .map_err(|_| RichCodexBackendClientError::Unavailable)?;
    read_model_route_mutation(reader, request_id, ModelRouteMutationKind::Create).await
}

pub(super) async fn request_model_route_retire<W, R>(
    writer: &mut W,
    reader: &mut R,
    request_id: &str,
    expected_revision: u64,
    model_tag: &str,
) -> Result<ModelRouteMutationResult, RichCodexBackendClientError>
where
    W: AsyncWrite + Unpin,
    R: AsyncBufRead + Unpin,
{
    write_message(
        writer,
        &AppServerMessage::ModelRouteRetire {
            request_id,
            expected_revision,
            model_tag,
        },
    )
    .await
    .map_err(|_| RichCodexBackendClientError::Unavailable)?;
    read_model_route_mutation(reader, request_id, ModelRouteMutationKind::Retire).await
}

pub(super) async fn request_model_route_set_targets<W, R>(
    writer: &mut W,
    reader: &mut R,
    request_id: &str,
    request: &ModelRouteSetTargetsRequest,
) -> Result<ModelRouteMutationResult, RichCodexBackendClientError>
where
    W: AsyncWrite + Unpin,
    R: AsyncBufRead + Unpin,
{
    write_message(
        writer,
        &AppServerMessage::ModelRouteSetTargets {
            request_id,
            expected_revision: request.expected_revision,
            model_tag: &request.model_tag,
            targets: &request.targets,
        },
    )
    .await
    .map_err(|_| RichCodexBackendClientError::Unavailable)?;
    read_model_route_mutation(reader, request_id, ModelRouteMutationKind::SetTargets).await
}

async fn read_model_route_mutation<R>(
    reader: &mut R,
    request_id: &str,
    kind: ModelRouteMutationKind,
) -> Result<ModelRouteMutationResult, RichCodexBackendClientError>
where
    R: AsyncBufRead + Unpin,
{
    let message = read_message(reader, REQUEST_TIMEOUT)
        .await
        .map_err(|_| RichCodexBackendClientError::Unavailable)?;
    let result = match message {
        BackendMessage::ModelRouteCreateResult {
            request_id: returned_request_id,
            desired_state_revision,
            catalog_revision,
            route,
        } if matches!(kind, ModelRouteMutationKind::Create)
            && returned_request_id == request_id =>
        {
            Some((desired_state_revision, catalog_revision, route))
        }
        BackendMessage::ModelRouteRetireResult {
            request_id: returned_request_id,
            desired_state_revision,
            catalog_revision,
            route,
        } if matches!(kind, ModelRouteMutationKind::Retire)
            && returned_request_id == request_id =>
        {
            Some((desired_state_revision, catalog_revision, route))
        }
        BackendMessage::ModelRouteSetTargetsResult {
            request_id: returned_request_id,
            desired_state_revision,
            catalog_revision,
            route,
        } if matches!(kind, ModelRouteMutationKind::SetTargets)
            && returned_request_id == request_id =>
        {
            Some((desired_state_revision, catalog_revision, route))
        }
        BackendMessage::OperationError {
            request_id: returned_request_id,
            code,
            ..
        } if returned_request_id == request_id => return Err(code.into()),
        _ => None,
    };
    let Some((desired_state_revision, catalog_revision, route)) = result else {
        return Err(RichCodexBackendClientError::Unavailable);
    };
    validate_model_route(&route).map_err(|_| RichCodexBackendClientError::Unavailable)?;
    Ok(ModelRouteMutationResult {
        desired_state_revision,
        catalog_revision,
        route,
    })
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

pub(super) fn validate_provider(provider: &ProviderSummary) -> io::Result<()> {
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

fn validate_model_routes(routes: &[ModelSummary]) -> io::Result<()> {
    if routes.len() > MAX_SNAPSHOT_ITEMS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend model-route response exceeded its item limit",
        ));
    }
    let mut model_tags = HashSet::with_capacity(routes.len());
    let mut target_ids = HashSet::new();
    let mut target_count = 0usize;
    for route in routes {
        validate_model_route(route)?;
        if !model_tags.insert(route.model_tag.as_str()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "model backend sent a duplicate model tag",
            ));
        }
        target_count = target_count
            .checked_add(route.targets.len())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "model backend model-route response exceeded its target limit",
                )
            })?;
        if target_count > MAX_SNAPSHOT_ITEMS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "model backend model-route response exceeded its target limit",
            ));
        }
        for target in &route.targets {
            if !target_ids.insert(target.id.as_str()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "model backend sent a duplicate model-route target id",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_model_route(route: &ModelSummary) -> io::Result<()> {
    validate_model_tag(&route.model_tag)?;
    validate_trimmed_safe_text(&route.display_name, 80)?;
    validate_trimmed_safe_text(&route.semantic_model, 200)?;
    if route.targets.is_empty() || route.targets.len() > MAX_SNAPSHOT_ITEMS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid model-route target list",
        ));
    }
    let mut target_ids = HashSet::with_capacity(route.targets.len());
    for (priority, target) in route.targets.iter().enumerate() {
        validate_safe_text(&target.id, 80)?;
        if !target_ids.insert(target.id.as_str()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "model backend sent a duplicate model-route target id",
            ));
        }
        if target.provider_id != "openai" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "model backend sent an unsupported model-route provider",
            ));
        }
        validate_safe_text(&target.account_id, 80)?;
        validate_trimmed_safe_text(&target.upstream_model_id, 512)?;
        if usize::try_from(target.priority).ok() != Some(priority) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "model backend sent a non-contiguous model-route priority",
            ));
        }
        if !matches!(
            target.status.as_str(),
            "unverified" | "reauthenticationRequired"
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "model backend sent an invalid model-route target status",
            ));
        }
    }
    Ok(())
}

fn validate_model_tag(value: &str) -> io::Result<()> {
    validate_trimmed_safe_text(value, 80)?;
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid model tag",
        ));
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit()
        || bytes.any(|byte| {
            !byte.is_ascii_lowercase()
                && !byte.is_ascii_digit()
                && !matches!(byte, b'.' | b'_' | b'/' | b'-')
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid model tag",
        ));
    }
    Ok(())
}

fn validate_trimmed_safe_text(value: &str, max_bytes: usize) -> io::Result<()> {
    validate_safe_text(value, max_bytes)?;
    if value.trim() != value {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid safe-display value",
        ));
    }
    Ok(())
}

pub(super) fn validate_provider_account(account: &ProviderAccountSummary) -> io::Result<()> {
    validate_safe_text(&account.id, 80)?;
    validate_safe_text(&account.provider_id, 256)?;
    validate_safe_text(&account.user_label, 80)?;
    if !matches!(account.credential_kind.as_str(), "oauth" | "apiKey") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid provider-account credential kind",
        ));
    }
    if i64::try_from(account.added_at).is_err() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid provider-account timestamp",
        ));
    }
    if !matches!(
        account.status.as_str(),
        "ready" | "verificationRequired" | "reauthenticationRequired"
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid provider-account status",
        ));
    }
    Ok(())
}

pub(super) fn validate_safe_text(value: &str, max_bytes: usize) -> io::Result<()> {
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid safe-display value",
        ));
    }
    Ok(())
}
