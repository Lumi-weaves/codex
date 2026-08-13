//! Private device-login protocol codec for the bundled RichCodex backend.

use super::AppServerMessage;
use super::BackendMessage;
use super::REQUEST_TIMEOUT;
use super::client::ProviderAccountSummary;
use super::client::RichCodexBackendClientError;
use super::client::validate_provider_account;
use super::client::validate_safe_text;
use super::read_message;
use super::write_message;
use serde::Deserialize;
use std::io;
use std::time::Duration;
use tokio::io::AsyncBufRead;
use tokio::io::AsyncWrite;

const PROVIDER_LOGIN_START_TIMEOUT: Duration = Duration::from_secs(/*secs*/ 35);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderAccountLoginResult {
    pub login_id: String,
    pub status: String,
    pub verification_url: Option<String>,
    pub user_code: Option<String>,
    pub expires_at: u64,
    pub failure: Option<String>,
    pub account: Option<ProviderAccountSummary>,
    pub desired_state_revision: u64,
    pub catalog_revision: u64,
}

pub(super) async fn request_provider_account_login_start<W, R>(
    writer: &mut W,
    reader: &mut R,
    request_id: &str,
    user_label: &str,
    account_id: Option<&str>,
) -> Result<ProviderAccountLoginResult, RichCodexBackendClientError>
where
    W: AsyncWrite + Unpin,
    R: AsyncBufRead + Unpin,
{
    write_message(
        writer,
        &AppServerMessage::ProviderAccountLoginStart {
            request_id,
            user_label,
            account_id,
        },
    )
    .await
    .map_err(|_| RichCodexBackendClientError::Unavailable)?;
    read_provider_account_login_result(
        reader,
        request_id,
        ProviderAccountLoginResultKind::Start,
        PROVIDER_LOGIN_START_TIMEOUT,
    )
    .await
}

pub(super) async fn request_provider_account_login_status<W, R>(
    writer: &mut W,
    reader: &mut R,
    request_id: &str,
    login_id: &str,
) -> Result<ProviderAccountLoginResult, RichCodexBackendClientError>
where
    W: AsyncWrite + Unpin,
    R: AsyncBufRead + Unpin,
{
    write_message(
        writer,
        &AppServerMessage::ProviderAccountLoginStatus {
            request_id,
            login_id,
        },
    )
    .await
    .map_err(|_| RichCodexBackendClientError::Unavailable)?;
    read_provider_account_login_result(
        reader,
        request_id,
        ProviderAccountLoginResultKind::Status,
        REQUEST_TIMEOUT,
    )
    .await
}

pub(super) async fn request_provider_account_login_cancel<W, R>(
    writer: &mut W,
    reader: &mut R,
    request_id: &str,
    login_id: &str,
) -> Result<ProviderAccountLoginResult, RichCodexBackendClientError>
where
    W: AsyncWrite + Unpin,
    R: AsyncBufRead + Unpin,
{
    write_message(
        writer,
        &AppServerMessage::ProviderAccountLoginCancel {
            request_id,
            login_id,
        },
    )
    .await
    .map_err(|_| RichCodexBackendClientError::Unavailable)?;
    read_provider_account_login_result(
        reader,
        request_id,
        ProviderAccountLoginResultKind::Cancel,
        REQUEST_TIMEOUT,
    )
    .await
}

#[derive(Clone, Copy)]
enum ProviderAccountLoginResultKind {
    Start,
    Status,
    Cancel,
}

async fn read_provider_account_login_result<R>(
    reader: &mut R,
    request_id: &str,
    kind: ProviderAccountLoginResultKind,
    wait: Duration,
) -> Result<ProviderAccountLoginResult, RichCodexBackendClientError>
where
    R: AsyncBufRead + Unpin,
{
    let message = read_message(reader, wait)
        .await
        .map_err(|_| RichCodexBackendClientError::Unavailable)?;
    let result = match message {
        BackendMessage::ProviderAccountLoginStartResult {
            request_id: returned_request_id,
            login_id,
            status,
            verification_url,
            user_code,
            expires_at,
            failure,
            account,
            desired_state_revision,
            catalog_revision,
        } if matches!(kind, ProviderAccountLoginResultKind::Start)
            && returned_request_id == request_id =>
        {
            ProviderAccountLoginResult {
                login_id,
                status,
                verification_url,
                user_code,
                expires_at,
                failure,
                account,
                desired_state_revision,
                catalog_revision,
            }
        }
        BackendMessage::ProviderAccountLoginStatusResult {
            request_id: returned_request_id,
            login_id,
            status,
            verification_url,
            user_code,
            expires_at,
            failure,
            account,
            desired_state_revision,
            catalog_revision,
        } if matches!(kind, ProviderAccountLoginResultKind::Status)
            && returned_request_id == request_id =>
        {
            ProviderAccountLoginResult {
                login_id,
                status,
                verification_url,
                user_code,
                expires_at,
                failure,
                account,
                desired_state_revision,
                catalog_revision,
            }
        }
        BackendMessage::ProviderAccountLoginCancelResult {
            request_id: returned_request_id,
            login_id,
            status,
            verification_url,
            user_code,
            expires_at,
            failure,
            account,
            desired_state_revision,
            catalog_revision,
        } if matches!(kind, ProviderAccountLoginResultKind::Cancel)
            && returned_request_id == request_id =>
        {
            ProviderAccountLoginResult {
                login_id,
                status,
                verification_url,
                user_code,
                expires_at,
                failure,
                account,
                desired_state_revision,
                catalog_revision,
            }
        }
        BackendMessage::OperationError {
            request_id: returned_request_id,
            code,
            ..
        } if returned_request_id == request_id => return Err(code.into()),
        _ => return Err(RichCodexBackendClientError::Unavailable),
    };
    validate_provider_account_login(&result)
        .map_err(|_| RichCodexBackendClientError::Unavailable)?;
    Ok(result)
}

fn validate_provider_account_login(login: &ProviderAccountLoginResult) -> io::Result<()> {
    validate_safe_text(&login.login_id, 80)?;
    if i64::try_from(login.expires_at).is_err() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid provider-login timestamp",
        ));
    }
    if let Some(url) = login.verification_url.as_deref()
        && url != "https://auth.openai.com/codex/device"
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid provider-login URL",
        ));
    }
    if let Some(user_code) = login.user_code.as_deref() {
        validate_safe_text(user_code, 128)?;
    }
    if let Some(failure) = login.failure.as_deref()
        && !matches!(
            failure,
            "expired"
                | "unavailable"
                | "invalidCredential"
                | "accountAlreadyExists"
                | "accountLimitReached"
                | "accountNotFound"
                | "credentialKindMismatch"
                | "accountIdentityMismatch"
                | "storeUnavailable"
        )
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid provider-login failure",
        ));
    }
    if let Some(account) = login.account.as_ref() {
        validate_provider_account(account)?;
    }
    let shape_is_valid = match login.status.as_str() {
        "awaitingUser" => {
            login.verification_url.is_some()
                && login.user_code.is_some()
                && login.failure.is_none()
                && login.account.is_none()
        }
        "exchanging" => {
            login.verification_url.is_none()
                && login.user_code.is_none()
                && login.failure.is_none()
                && login.account.is_none()
        }
        "completed" => {
            login.verification_url.is_none()
                && login.user_code.is_none()
                && login.failure.is_none()
                && login.account.is_some()
        }
        "failed" => {
            login.verification_url.is_none()
                && login.user_code.is_none()
                && login.failure.is_some()
                && login.account.is_none()
        }
        "cancelled" => {
            login.verification_url.is_none()
                && login.user_code.is_none()
                && login.failure.is_none()
                && login.account.is_none()
        }
        _ => false,
    };
    if !shape_is_valid {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model backend sent an invalid provider-login state",
        ));
    }
    Ok(())
}
