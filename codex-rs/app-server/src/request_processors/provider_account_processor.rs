use crate::error_code::internal_error;
use crate::error_code::invalid_params;
use crate::richcodex_backend::ProviderAccountImportResult;
use crate::richcodex_backend::ProviderAccountListResult;
use crate::richcodex_backend::ProviderAccountSummary;
use crate::richcodex_backend::ProviderSummary;
use crate::richcodex_backend::RichCodexBackendClient;
use crate::richcodex_backend::RichCodexBackendClientError;
use codex_app_server_protocol::ClientResponsePayload;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::ProviderAccount;
use codex_app_server_protocol::ProviderAccountImportParams;
use codex_app_server_protocol::ProviderAccountImportResponse;
use codex_app_server_protocol::ProviderAccountListParams;
use codex_app_server_protocol::ProviderAccountListResponse;
use codex_app_server_protocol::ProviderAccountProvider;
use codex_app_server_protocol::ProviderAccountProviderStatus;
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

fn provider_account(account: ProviderAccountSummary) -> ProviderAccount {
    ProviderAccount {
        id: account.id,
        provider_id: account.provider_id,
        user_label: account.user_label,
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
