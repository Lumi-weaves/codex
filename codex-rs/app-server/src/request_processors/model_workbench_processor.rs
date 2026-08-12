use super::*;
use crate::error_code::internal_error;
use crate::error_code::invalid_params;
use crate::error_code::overloaded;
use crate::model_list_catalog::ModelListCatalog;
use crate::request_processors::model_workbench_transport::request_workbench;
use codex_app_server_protocol::ModelWorkbenchEntry;
use codex_app_server_protocol::ModelWorkbenchPublication;
use codex_app_server_protocol::ModelWorkbenchPublicationStatus;
use codex_app_server_protocol::ModelWorkbenchReadParams;
use codex_app_server_protocol::ModelWorkbenchReadResponse;
use codex_app_server_protocol::ModelWorkbenchRetireParams;
use codex_app_server_protocol::ModelWorkbenchRetireResponse;
use codex_app_server_protocol::ModelWorkbenchStoredEntry;
use codex_app_server_protocol::ModelWorkbenchUpsertParams;
use codex_app_server_protocol::ModelWorkbenchUpsertResponse;
use codex_models_manager::manager::RefreshStrategy;
use futures::StreamExt;
use reqwest::Method;
use reqwest::StatusCode;
use serde::Deserialize;
use serde::Serialize;

#[cfg(test)]
#[path = "model_workbench_processor_tests.rs"]
mod tests;

const ENTRIES_PATH: &str = "/api/model-workbench/entries";
const RETIRE_PATH: &str = "/api/model-workbench/entries/retire";
const BODY_LIMIT: usize = 512 * 1024;
const MAX_ACTIVE_ENTRIES: usize = 256;
const REVISION_CONFLICT_ERROR_CODE: i64 = -32072;

#[derive(Clone)]
pub(crate) struct ModelWorkbenchRequestProcessor {
    model_list_catalog: Arc<ModelListCatalog>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiReadResponse {
    revision: u64,
    entries: Vec<ModelWorkbenchEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiPublication {
    registry_revision: u64,
    catalog_revision: Option<u64>,
    models_cache_revision: Option<u64>,
    synchronized: bool,
    catalog_refresh: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiMutationResponse {
    revision: u64,
    changed: bool,
    entry: ModelWorkbenchStoredEntry,
    publication: ApiPublication,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiErrorResponse {
    current_revision: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpsertBody<'a> {
    display_name: &'a str,
    model_tag: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RetireBody<'a> {
    model_tag: &'a str,
}

impl ModelWorkbenchRequestProcessor {
    pub(crate) fn new(model_list_catalog: Arc<ModelListCatalog>) -> Self {
        Self { model_list_catalog }
    }

    pub(crate) async fn read(
        &self,
        _params: ModelWorkbenchReadParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let response = request_workbench(Method::GET, ENTRIES_PATH, Vec::new(), None).await?;
        let response = require_status(response, &[StatusCode::OK]).await?;
        let bytes = read_bounded(response).await?;
        let state: ApiReadResponse = serde_json::from_slice(&bytes).map_err(|_| {
            internal_error("OpenCodex returned an invalid Model Workbench response")
        })?;
        if state.entries.len() > MAX_ACTIVE_ENTRIES
            || state.entries.iter().any(|entry| {
                !workbench_string_is_valid(&entry.model_tag, 256)
                    || !workbench_string_is_valid(&entry.display_name, 120)
            })
        {
            return Err(internal_error(
                "OpenCodex returned an invalid Model Workbench response",
            ));
        }
        Ok(Some(
            ModelWorkbenchReadResponse {
                revision: state.revision,
                entries: state.entries,
            }
            .into(),
        ))
    }

    pub(crate) async fn upsert(
        &self,
        params: ModelWorkbenchUpsertParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        validate_model_tag(&params.model_tag)?;
        validate_display_name(&params.display_name)?;
        let body = serde_json::to_vec(&UpsertBody {
            display_name: &params.display_name,
            model_tag: &params.model_tag,
        })
        .map_err(|_| invalid_params("Model Workbench parameters are invalid"))?;
        let response =
            request_workbench(Method::PUT, ENTRIES_PATH, body, params.expected_revision).await?;
        let response = mutation_response(response).await?;
        self.model_list_catalog
            .refresh(RefreshStrategy::Offline)
            .await;
        Ok(Some(
            ModelWorkbenchUpsertResponse {
                revision: response.revision,
                changed: response.changed,
                entry: response.entry,
                publication: response.publication,
            }
            .into(),
        ))
    }

    pub(crate) async fn retire(
        &self,
        params: ModelWorkbenchRetireParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        validate_model_tag(&params.model_tag)?;
        let body = serde_json::to_vec(&RetireBody {
            model_tag: &params.model_tag,
        })
        .map_err(|_| invalid_params("Model Workbench parameters are invalid"))?;
        let response =
            request_workbench(Method::POST, RETIRE_PATH, body, params.expected_revision).await?;
        let response = mutation_response(response).await?;
        self.model_list_catalog
            .refresh(RefreshStrategy::Offline)
            .await;
        Ok(Some(
            ModelWorkbenchRetireResponse {
                revision: response.revision,
                changed: response.changed,
                entry: response.entry,
                publication: response.publication,
            }
            .into(),
        ))
    }
}

async fn mutation_response(
    response: reqwest::Response,
) -> Result<ApiMutationReceipt, JSONRPCErrorError> {
    let response = require_status(response, &[StatusCode::OK, StatusCode::ACCEPTED]).await?;
    let bytes = read_bounded(response).await?;
    let receipt: ApiMutationResponse = serde_json::from_slice(&bytes)
        .map_err(|_| internal_error("OpenCodex returned an invalid Model Workbench receipt"))?;
    if !workbench_string_is_valid(&receipt.entry.model_tag, 256)
        || !workbench_string_is_valid(&receipt.entry.display_name, 120)
    {
        return Err(internal_error(
            "OpenCodex returned an invalid Model Workbench receipt",
        ));
    }
    let publication_status = publication_status(&receipt.publication);
    Ok(ApiMutationReceipt {
        revision: receipt.revision,
        changed: receipt.changed,
        entry: receipt.entry,
        publication: ModelWorkbenchPublication {
            registry_revision: receipt.publication.registry_revision,
            catalog_revision: receipt.publication.catalog_revision,
            models_cache_revision: receipt.publication.models_cache_revision,
            synchronized: receipt.publication.synchronized,
            status: publication_status,
        },
    })
}

fn publication_status(publication: &ApiPublication) -> ModelWorkbenchPublicationStatus {
    if publication.synchronized {
        ModelWorkbenchPublicationStatus::Synchronized
    } else if publication
        .catalog_refresh
        .get("status")
        .and_then(|value| value.as_str())
        == Some("failed")
        || (publication
            .catalog_refresh
            .get("status")
            .and_then(|value| value.as_str())
            == Some("skipped")
            && publication
                .catalog_refresh
                .get("retryable")
                .and_then(serde_json::Value::as_bool)
                == Some(false))
    {
        ModelWorkbenchPublicationStatus::Failed
    } else {
        ModelWorkbenchPublicationStatus::Pending
    }
}

struct ApiMutationReceipt {
    revision: u64,
    changed: bool,
    entry: ModelWorkbenchStoredEntry,
    publication: ModelWorkbenchPublication,
}

fn validate_model_tag(value: &str) -> Result<(), JSONRPCErrorError> {
    validate_workbench_string(value, 256, "modelTag")
}

fn validate_display_name(value: &str) -> Result<(), JSONRPCErrorError> {
    validate_workbench_string(value, 120, "displayName")
}

fn validate_workbench_string(
    value: &str,
    max_utf16_units: usize,
    name: &str,
) -> Result<(), JSONRPCErrorError> {
    if !workbench_string_is_valid(value, max_utf16_units) {
        return Err(invalid_params(format!("Model Workbench {name} is invalid")));
    }
    Ok(())
}

fn workbench_string_is_valid(value: &str, max_utf16_units: usize) -> bool {
    let has_ascii_control = value
        .chars()
        .any(|character| character <= '\u{001f}' || character == '\u{007f}');
    !value.is_empty()
        && value.trim() == value
        && value.encode_utf16().count() <= max_utf16_units
        && !has_ascii_control
}

pub(super) async fn read_bounded(
    response: reqwest::Response,
) -> Result<Vec<u8>, JSONRPCErrorError> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| internal_error("OpenCodex response was interrupted"))?;
        if body.len().saturating_add(chunk.len()) > BODY_LIMIT {
            return Err(internal_error("OpenCodex response is too large"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn require_status(
    response: reqwest::Response,
    accepted: &[StatusCode],
) -> Result<reqwest::Response, JSONRPCErrorError> {
    if accepted.contains(&response.status()) {
        return Ok(response);
    }
    let status = response.status();
    let body = read_bounded(response).await.unwrap_or_default();
    match status {
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND => {
            Err(invalid_params("Model Workbench request was rejected"))
        }
        StatusCode::CONFLICT => {
            let current_revision = serde_json::from_slice::<ApiErrorResponse>(&body)
                .ok()
                .and_then(|error| error.current_revision);
            Err(JSONRPCErrorError {
                code: REVISION_CONFLICT_ERROR_CODE,
                message: "Model Workbench revision conflict".to_string(),
                data: current_revision
                    .map(|revision| serde_json::json!({ "currentRevision": revision })),
            })
        }
        StatusCode::SERVICE_UNAVAILABLE => Err(overloaded("Model Workbench is busy")),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            Err(internal_error("Model Workbench capability was rejected"))
        }
        _ => Err(internal_error("Model Workbench request failed")),
    }
}
