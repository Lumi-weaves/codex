use crate::JsonSchema;
use crate::TS;
use serde::Deserialize;
use serde::Serialize;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelWorkbenchReadParams {}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelWorkbenchEntry {
    pub model_tag: String,
    pub display_name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelWorkbenchStoredEntry {
    pub model_tag: String,
    pub display_name: String,
    pub retired: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelWorkbenchReadResponse {
    pub revision: u64,
    pub entries: Vec<ModelWorkbenchEntry>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelWorkbenchUpsertParams {
    pub model_tag: String,
    pub display_name: String,
    #[ts(optional = nullable)]
    pub expected_revision: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelWorkbenchRetireParams {
    pub model_tag: String,
    #[ts(optional = nullable)]
    pub expected_revision: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum ModelWorkbenchPublicationStatus {
    Synchronized,
    Pending,
    Failed,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelWorkbenchPublication {
    pub registry_revision: u64,
    pub catalog_revision: Option<u64>,
    pub models_cache_revision: Option<u64>,
    pub synchronized: bool,
    pub status: ModelWorkbenchPublicationStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelWorkbenchUpsertResponse {
    pub revision: u64,
    pub changed: bool,
    pub entry: ModelWorkbenchStoredEntry,
    pub publication: ModelWorkbenchPublication,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ModelWorkbenchRetireResponse {
    pub revision: u64,
    pub changed: bool,
    pub entry: ModelWorkbenchStoredEntry,
    pub publication: ModelWorkbenchPublication,
}
