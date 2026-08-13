use crate::JsonSchema;
use crate::TS;
use serde::Deserialize;
use serde::Serialize;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelRouteReadParams {}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum ModelRouteTargetStatus {
    Unverified,
    ReauthenticationRequired,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelRouteTarget {
    pub id: String,
    pub provider_id: String,
    /// Opaque RichCodex account handle; never an upstream provider identity.
    pub account_id: String,
    pub upstream_model_id: String,
    pub priority: u32,
    pub status: ModelRouteTargetStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelRoute {
    pub model_tag: String,
    pub display_name: String,
    pub retired: bool,
    pub semantic_model: String,
    pub targets: Vec<ModelRouteTarget>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelRouteReadResponse {
    pub data: Vec<ModelRoute>,
    pub desired_state_revision: String,
    pub catalog_revision: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelRouteCreateParams {
    /// Opaque revision returned by a previous model-plane read or mutation.
    pub expected_revision: String,
    pub model_tag: String,
    pub display_name: String,
    /// Stable semantic identity shared by equivalent provider targets.
    pub semantic_model: String,
    pub provider_id: String,
    pub account_id: String,
    pub upstream_model_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelRouteCreateResponse {
    pub route: ModelRoute,
    pub desired_state_revision: String,
    pub catalog_revision: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelRouteTargetInput {
    /// Preserve an existing target by its opaque RichCodex handle. Omit this
    /// field to allocate a new target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub id: Option<String>,
    pub provider_id: String,
    pub account_id: String,
    pub upstream_model_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelRouteSetTargetsParams {
    /// Opaque revision returned by a previous model-plane read or mutation.
    pub expected_revision: String,
    pub model_tag: String,
    /// Complete ordered target list. Its array order becomes target priority.
    pub targets: Vec<ModelRouteTargetInput>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelRouteSetTargetsResponse {
    pub route: ModelRoute,
    pub desired_state_revision: String,
    pub catalog_revision: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelRouteRetireParams {
    pub expected_revision: String,
    pub model_tag: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelRouteRetireResponse {
    pub route: ModelRoute,
    pub desired_state_revision: String,
    pub catalog_revision: String,
}
