use crate::JsonSchema;
use crate::TS;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use serde::Serialize;

/// Read a page of accounts owned by the RichCodex provider plane.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ProviderAccountListParams {
    /// Opaque pagination cursor returned by a previous call.
    #[ts(optional = nullable)]
    pub cursor: Option<String>,
    /// Optional page size. The backend currently accepts values from 1 to 100.
    #[ts(optional = nullable)]
    pub limit: Option<u32>,
}

/// Import one explicitly selected Codex login into the RichCodex provider plane.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ProviderAccountImportParams {
    /// Absolute path selected by the user. App-server forwards but never reads this file.
    pub auth_json_path: AbsolutePathBuf,
    /// User-controlled display label; provider credentials and account identity stay private.
    pub user_label: String,
}

/// Add one API-key credential to the RichCodex provider plane.
///
/// The key is write-only request material: it is never returned by any public
/// model-plane response.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ProviderAccountAddApiKeyParams {
    pub api_key: String,
    pub user_label: String,
}

impl std::fmt::Debug for ProviderAccountAddApiKeyParams {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderAccountAddApiKeyParams")
            .field("api_key", &"[REDACTED]")
            .field("user_label", &self.user_label)
            .finish()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum ProviderAccountStatus {
    Ready,
    VerificationRequired,
    ReauthenticationRequired,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum ProviderAccountCredentialKind {
    #[serde(rename = "oauth")]
    #[ts(rename = "oauth")]
    OAuth,
    ApiKey,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ProviderAccount {
    /// Opaque local identifier. This is not the upstream provider account id.
    pub id: String,
    pub provider_id: String,
    pub user_label: String,
    pub credential_kind: ProviderAccountCredentialKind,
    pub status: ProviderAccountStatus,
    /// Unix timestamp in seconds.
    #[ts(type = "number")]
    pub added_at: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum ProviderAccountProviderStatus {
    Ready,
    NeedsAccount,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ProviderAccountProvider {
    pub id: String,
    pub display_name: String,
    pub account_count: u32,
    pub status: ProviderAccountProviderStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ProviderAccountListResponse {
    pub data: Vec<ProviderAccount>,
    pub providers: Vec<ProviderAccountProvider>,
    /// Opaque desired-state revision for cache comparison.
    pub desired_state_revision: String,
    /// Opaque catalog revision produced by the same backend transaction.
    pub catalog_revision: String,
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ProviderAccountImportResponse {
    pub account: ProviderAccount,
    pub desired_state_revision: String,
    pub catalog_revision: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ProviderAccountAddApiKeyResponse {
    pub account: ProviderAccount,
    pub desired_state_revision: String,
    pub catalog_revision: String,
}
