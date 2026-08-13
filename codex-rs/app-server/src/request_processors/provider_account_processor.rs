use crate::error_code::internal_error;
use crate::error_code::invalid_params;
use crate::richcodex_backend::ProviderAccountAddApiKeyRequest;
use crate::richcodex_backend::ProviderAccountAddApiKeyResult;
use crate::richcodex_backend::ProviderAccountImportResult;
use crate::richcodex_backend::ProviderAccountListResult;
use crate::richcodex_backend::ProviderAccountLoginResult;
use crate::richcodex_backend::ProviderAccountMutationResult;
use crate::richcodex_backend::ProviderAccountRemovalPreviewResult;
use crate::richcodex_backend::ProviderAccountSummary;
use crate::richcodex_backend::ProviderSummary;
use crate::richcodex_backend::RichCodexBackendClient;
use crate::richcodex_backend::RichCodexBackendClientError;
use codex_app_server_protocol::ClientResponsePayload;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::ProviderAccount;
use codex_app_server_protocol::ProviderAccountAddApiKeyParams;
use codex_app_server_protocol::ProviderAccountAddApiKeyResponse;
use codex_app_server_protocol::ProviderAccountCredentialKind;
use codex_app_server_protocol::ProviderAccountImportParams;
use codex_app_server_protocol::ProviderAccountImportResponse;
use codex_app_server_protocol::ProviderAccountListParams;
use codex_app_server_protocol::ProviderAccountListResponse;
use codex_app_server_protocol::ProviderAccountLogin;
use codex_app_server_protocol::ProviderAccountLoginCancelParams;
use codex_app_server_protocol::ProviderAccountLoginCancelResponse;
use codex_app_server_protocol::ProviderAccountLoginFailure;
use codex_app_server_protocol::ProviderAccountLoginStartParams;
use codex_app_server_protocol::ProviderAccountLoginStartResponse;
use codex_app_server_protocol::ProviderAccountLoginStatus;
use codex_app_server_protocol::ProviderAccountLoginStatusParams;
use codex_app_server_protocol::ProviderAccountLoginStatusResponse;
use codex_app_server_protocol::ProviderAccountProvider;
use codex_app_server_protocol::ProviderAccountProviderStatus;
use codex_app_server_protocol::ProviderAccountRemovalPreviewParams;
use codex_app_server_protocol::ProviderAccountRemovalPreviewResponse;
use codex_app_server_protocol::ProviderAccountRemovalTarget;
use codex_app_server_protocol::ProviderAccountRemoveParams;
use codex_app_server_protocol::ProviderAccountRemoveResponse;
use codex_app_server_protocol::ProviderAccountReplaceApiKeyParams;
use codex_app_server_protocol::ProviderAccountReplaceApiKeyResponse;
use codex_app_server_protocol::ProviderAccountStatus;

#[derive(Clone)]
pub(crate) struct ProviderAccountRequestProcessor {
    backend: Option<RichCodexBackendClient>,
}

impl ProviderAccountRequestProcessor {
    pub(crate) fn new(backend: Option<RichCodexBackendClient>) -> Self {
        Self { backend }
    }

    pub(crate) async fn list(
        &self,
        params: ProviderAccountListParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        if params
            .limit
            .is_some_and(|limit| !(1..=100).contains(&limit))
        {
            return Err(invalid_params(
                "provider account list limit must be between 1 and 100",
            ));
        }
        let backend = self.backend.as_ref().ok_or_else(backend_unavailable)?;
        backend
            .list_provider_accounts(params.cursor, params.limit)
            .await
            .map(provider_account_list_response)
            .map(|response| Some(response.into()))
            .map_err(provider_account_error)
    }

    pub(crate) async fn import(
        &self,
        params: ProviderAccountImportParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let auth_json_path = params
            .auth_json_path
            .as_path()
            .to_str()
            .ok_or_else(|| invalid_params("authJsonPath must be valid UTF-8"))?
            .to_string();
        let backend = self.backend.as_ref().ok_or_else(backend_unavailable)?;
        backend
            .import_provider_account(auth_json_path, params.user_label)
            .await
            .map(provider_account_import_response)
            .map(|response| Some(response.into()))
            .map_err(provider_account_error)
    }

    pub(crate) async fn add_api_key(
        &self,
        params: ProviderAccountAddApiKeyParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        if !valid_provider_id(&params.provider_id)
            || !valid_safe_text(&params.provider_display_name, 80)
            || params.provider_display_name.trim() != params.provider_display_name
            || !valid_api_base_url(&params.api_base_url)
            || params.api_key.is_empty()
            || params.api_key.len() > 64 * 1024
            || params.api_key.trim() != params.api_key
            || params.api_key.chars().any(char::is_control)
            || params.user_label.is_empty()
            || params.user_label.len() > 80
            || params.user_label.trim() != params.user_label
            || params.user_label.chars().any(char::is_control)
        {
            return Err(invalid_params("API-key provider account input is invalid"));
        }
        let backend = self.backend.as_ref().ok_or_else(backend_unavailable)?;
        backend
            .add_api_key_provider_account(ProviderAccountAddApiKeyRequest {
                provider_id: params.provider_id,
                provider_display_name: params.provider_display_name,
                api_base_url: params.api_base_url,
                api_key: params.api_key,
                user_label: params.user_label,
            })
            .await
            .map(provider_account_add_api_key_response)
            .map(|response| Some(response.into()))
            .map_err(provider_account_error)
    }

    pub(crate) async fn login_start(
        &self,
        params: ProviderAccountLoginStartParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        if !valid_safe_text(&params.user_label, 80) || params.user_label.trim() != params.user_label
        {
            return Err(invalid_params("provider account login label is invalid"));
        }
        let backend = self.backend.as_ref().ok_or_else(backend_unavailable)?;
        backend
            .start_provider_account_login(params.user_label, params.account_id)
            .await
            .map(|result| ProviderAccountLoginStartResponse {
                login: provider_account_login(result),
            })
            .map(|response| Some(response.into()))
            .map_err(provider_account_error)
    }

    pub(crate) async fn login_status(
        &self,
        params: ProviderAccountLoginStatusParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        if !valid_safe_text(&params.login_id, 80) {
            return Err(invalid_params("provider account login ID is invalid"));
        }
        let backend = self.backend.as_ref().ok_or_else(backend_unavailable)?;
        backend
            .provider_account_login_status(params.login_id)
            .await
            .map(|result| ProviderAccountLoginStatusResponse {
                login: provider_account_login(result),
            })
            .map(|response| Some(response.into()))
            .map_err(provider_account_error)
    }

    pub(crate) async fn login_cancel(
        &self,
        params: ProviderAccountLoginCancelParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        if !valid_safe_text(&params.login_id, 80) {
            return Err(invalid_params("provider account login ID is invalid"));
        }
        let backend = self.backend.as_ref().ok_or_else(backend_unavailable)?;
        backend
            .cancel_provider_account_login(params.login_id)
            .await
            .map(|result| ProviderAccountLoginCancelResponse {
                login: provider_account_login(result),
            })
            .map(|response| Some(response.into()))
            .map_err(provider_account_error)
    }

    pub(crate) async fn replace_api_key(
        &self,
        params: ProviderAccountReplaceApiKeyParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let expected_revision = parse_revision(&params.expected_revision)?;
        if !valid_safe_text(&params.account_id, 80)
            || !valid_safe_text(&params.api_key, 64 * 1024)
            || params.api_key.trim() != params.api_key
        {
            return Err(invalid_params("API-key replacement input is invalid"));
        }
        let backend = self.backend.as_ref().ok_or_else(backend_unavailable)?;
        backend
            .replace_api_key_provider_account(expected_revision, params.account_id, params.api_key)
            .await
            .map(|result| ProviderAccountReplaceApiKeyResponse {
                account: provider_account(result.account),
                desired_state_revision: result.desired_state_revision.to_string(),
                catalog_revision: result.catalog_revision.to_string(),
            })
            .map(|response| Some(response.into()))
            .map_err(provider_account_error)
    }

    pub(crate) async fn removal_preview(
        &self,
        params: ProviderAccountRemovalPreviewParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        if !valid_safe_text(&params.account_id, 80) {
            return Err(invalid_params("provider account ID is invalid"));
        }
        let backend = self.backend.as_ref().ok_or_else(backend_unavailable)?;
        backend
            .preview_provider_account_removal(params.account_id)
            .await
            .map(provider_account_removal_preview_response)
            .map(|response| Some(response.into()))
            .map_err(provider_account_error)
    }

    pub(crate) async fn remove(
        &self,
        params: ProviderAccountRemoveParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let expected_revision = parse_revision(&params.expected_revision)?;
        if !valid_safe_text(&params.account_id, 80) {
            return Err(invalid_params("provider account ID is invalid"));
        }
        let backend = self.backend.as_ref().ok_or_else(backend_unavailable)?;
        backend
            .remove_provider_account(expected_revision, params.account_id)
            .await
            .map(provider_account_remove_response)
            .map(|response| Some(response.into()))
            .map_err(provider_account_error)
    }
}

fn parse_revision(value: &str) -> Result<u64, JSONRPCErrorError> {
    value
        .parse::<u64>()
        .map_err(|_| invalid_params("expectedRevision is invalid"))
}

fn valid_safe_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

fn provider_account_list_response(
    result: ProviderAccountListResult,
) -> ProviderAccountListResponse {
    ProviderAccountListResponse {
        data: result.data.into_iter().map(provider_account).collect(),
        providers: result.providers.into_iter().map(provider).collect(),
        desired_state_revision: result.desired_state_revision.to_string(),
        catalog_revision: result.catalog_revision.to_string(),
        next_cursor: result.next_cursor,
    }
}

fn provider_account_import_response(
    result: ProviderAccountImportResult,
) -> ProviderAccountImportResponse {
    ProviderAccountImportResponse {
        account: provider_account(result.account),
        desired_state_revision: result.desired_state_revision.to_string(),
        catalog_revision: result.catalog_revision.to_string(),
    }
}

fn provider_account_add_api_key_response(
    result: ProviderAccountAddApiKeyResult,
) -> ProviderAccountAddApiKeyResponse {
    ProviderAccountAddApiKeyResponse {
        account: provider_account(result.account),
        desired_state_revision: result.desired_state_revision.to_string(),
        catalog_revision: result.catalog_revision.to_string(),
    }
}

fn provider_account_removal_preview_response(
    result: ProviderAccountRemovalPreviewResult,
) -> ProviderAccountRemovalPreviewResponse {
    ProviderAccountRemovalPreviewResponse {
        account: provider_account(result.account),
        affected_targets: result
            .affected_targets
            .into_iter()
            .map(|target| ProviderAccountRemovalTarget {
                model_tag: target.model_tag,
                display_name: target.display_name,
                retired: target.retired,
                target_id: target.target_id,
                upstream_model_id: target.upstream_model_id,
                priority: target.priority,
            })
            .collect(),
        can_remove: result.can_remove,
        desired_state_revision: result.desired_state_revision.to_string(),
        catalog_revision: result.catalog_revision.to_string(),
    }
}

fn provider_account_remove_response(
    result: ProviderAccountMutationResult,
) -> ProviderAccountRemoveResponse {
    ProviderAccountRemoveResponse {
        account: provider_account(result.account),
        desired_state_revision: result.desired_state_revision.to_string(),
        catalog_revision: result.catalog_revision.to_string(),
    }
}

fn provider_account_login(result: ProviderAccountLoginResult) -> ProviderAccountLogin {
    ProviderAccountLogin {
        login_id: result.login_id,
        status: match result.status.as_str() {
            "awaitingUser" => ProviderAccountLoginStatus::AwaitingUser,
            "exchanging" => ProviderAccountLoginStatus::Exchanging,
            "completed" => ProviderAccountLoginStatus::Completed,
            "failed" => ProviderAccountLoginStatus::Failed,
            "cancelled" => ProviderAccountLoginStatus::Cancelled,
            _ => unreachable!("backend response state is validated by the client actor"),
        },
        verification_url: result.verification_url,
        user_code: result.user_code,
        expires_at: result.expires_at as i64,
        failure: result.failure.map(|failure| match failure.as_str() {
            "expired" => ProviderAccountLoginFailure::Expired,
            "unavailable" => ProviderAccountLoginFailure::Unavailable,
            "invalidCredential" => ProviderAccountLoginFailure::InvalidCredential,
            "accountAlreadyExists" => ProviderAccountLoginFailure::AccountAlreadyExists,
            "accountLimitReached" => ProviderAccountLoginFailure::AccountLimitReached,
            "accountNotFound" => ProviderAccountLoginFailure::AccountNotFound,
            "credentialKindMismatch" => ProviderAccountLoginFailure::CredentialKindMismatch,
            "accountIdentityMismatch" => ProviderAccountLoginFailure::AccountIdentityMismatch,
            "storeUnavailable" => ProviderAccountLoginFailure::StoreUnavailable,
            _ => unreachable!("backend response failure is validated by the client actor"),
        }),
        account: result.account.map(provider_account),
        desired_state_revision: result.desired_state_revision.to_string(),
        catalog_revision: result.catalog_revision.to_string(),
    }
}

fn provider_account(account: ProviderAccountSummary) -> ProviderAccount {
    ProviderAccount {
        id: account.id,
        provider_id: account.provider_id,
        user_label: account.user_label,
        credential_kind: match account.credential_kind.as_str() {
            "oauth" => ProviderAccountCredentialKind::OAuth,
            "apiKey" => ProviderAccountCredentialKind::ApiKey,
            _ => unreachable!("backend response kind is validated by the client actor"),
        },
        status: match account.status.as_str() {
            "ready" => ProviderAccountStatus::Ready,
            "verificationRequired" => ProviderAccountStatus::VerificationRequired,
            "reauthenticationRequired" => ProviderAccountStatus::ReauthenticationRequired,
            _ => unreachable!("backend response status is validated by the client actor"),
        },
        added_at: account.added_at as i64,
    }
}

fn provider(provider: ProviderSummary) -> ProviderAccountProvider {
    ProviderAccountProvider {
        id: provider.id,
        display_name: provider.display_name,
        account_count: provider.account_count,
        status: match provider.status.as_str() {
            "ready" => ProviderAccountProviderStatus::Ready,
            "needsAccount" => ProviderAccountProviderStatus::NeedsAccount,
            _ => unreachable!("backend response status is validated by the client actor"),
        },
    }
}

fn backend_unavailable() -> JSONRPCErrorError {
    internal_error("RichCodex provider account backend is unavailable")
}

fn provider_account_error(error: RichCodexBackendClientError) -> JSONRPCErrorError {
    match error {
        RichCodexBackendClientError::InvalidRequest => {
            invalid_params("provider account request is invalid")
        }
        RichCodexBackendClientError::SourceUnavailable => {
            invalid_params("selected credential source is unavailable")
        }
        RichCodexBackendClientError::SourceTooLarge => {
            invalid_params("selected credential source exceeds its limit")
        }
        RichCodexBackendClientError::InvalidAuthDocument => {
            invalid_params("selected credential source is not a supported Codex login")
        }
        RichCodexBackendClientError::CredentialExpired => {
            invalid_params("selected Codex login has expired")
        }
        RichCodexBackendClientError::AccountAlreadyExists => {
            invalid_params("this provider account is already configured")
        }
        RichCodexBackendClientError::AccountLimitReached => {
            invalid_params("provider account limit reached")
        }
        RichCodexBackendClientError::AccountNotFound => {
            invalid_params("provider account does not exist")
        }
        RichCodexBackendClientError::AccountInUse => {
            invalid_params("provider account is still referenced by model targets")
        }
        RichCodexBackendClientError::CredentialKindMismatch => {
            invalid_params("provider account credential kind does not match")
        }
        RichCodexBackendClientError::AccountIdentityMismatch => {
            invalid_params("reauthentication returned a different upstream account")
        }
        RichCodexBackendClientError::InvalidProvider => {
            invalid_params("provider configuration is invalid")
        }
        RichCodexBackendClientError::ProviderConflict => {
            invalid_params("providerId is already configured differently")
        }
        RichCodexBackendClientError::InvalidApiKey => invalid_params("API key is invalid"),
        RichCodexBackendClientError::LoginUnavailable => {
            internal_error("OpenAI provider login is unavailable")
        }
        RichCodexBackendClientError::LoginLimitReached => {
            invalid_params("too many provider logins are active")
        }
        RichCodexBackendClientError::LoginNotFound => {
            invalid_params("provider login does not exist")
        }
        RichCodexBackendClientError::StoreUnavailable => {
            internal_error("RichCodex provider account store is unavailable")
        }
        RichCodexBackendClientError::Unavailable => backend_unavailable(),
        RichCodexBackendClientError::RevisionConflict
        | RichCodexBackendClientError::ModelTagExists
        | RichCodexBackendClientError::ModelTagNotFound
        | RichCodexBackendClientError::AccountUnavailable => {
            internal_error("RichCodex provider account backend returned an invalid operation error")
        }
    }
}

fn valid_provider_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    value.len() <= 64
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn valid_api_base_url(value: &str) -> bool {
    if value.len() > 2048 || value.trim() != value || value.ends_with('/') {
        return false;
    }
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    (url.scheme() == "https" || url.scheme() == "http" && loopback)
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path() != "/"
}
