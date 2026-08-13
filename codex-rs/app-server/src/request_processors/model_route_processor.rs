use crate::error_code::internal_error;
use crate::error_code::invalid_params;
use crate::model_list_catalog::ModelListCatalog;
use crate::richcodex_backend::ModelRouteCreateRequest;
use crate::richcodex_backend::ModelRouteMutationResult;
use crate::richcodex_backend::ModelRouteReadResult;
use crate::richcodex_backend::ModelRouteSetTargetsRequest;
use crate::richcodex_backend::ModelRouteTargetRequest;
use crate::richcodex_backend::ModelSummary;
use crate::richcodex_backend::ModelTargetSummary;
use crate::richcodex_backend::RichCodexBackendClient;
use crate::richcodex_backend::RichCodexBackendClientError;
use codex_app_server_protocol::ClientResponsePayload;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::ModelRoute;
use codex_app_server_protocol::ModelRouteCreateParams;
use codex_app_server_protocol::ModelRouteCreateResponse;
use codex_app_server_protocol::ModelRouteReadParams;
use codex_app_server_protocol::ModelRouteReadResponse;
use codex_app_server_protocol::ModelRouteRetireParams;
use codex_app_server_protocol::ModelRouteRetireResponse;
use codex_app_server_protocol::ModelRouteSetTargetsParams;
use codex_app_server_protocol::ModelRouteSetTargetsResponse;
use codex_app_server_protocol::ModelRouteTarget;
use codex_app_server_protocol::ModelRouteTargetStatus;
use std::sync::Arc;

const MAX_BACKEND_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone)]
pub(crate) struct ModelRouteRequestProcessor {
    backend: Option<RichCodexBackendClient>,
    model_list_catalog: Arc<ModelListCatalog>,
}

impl ModelRouteRequestProcessor {
    pub(crate) fn new(
        backend: Option<RichCodexBackendClient>,
        model_list_catalog: Arc<ModelListCatalog>,
    ) -> Self {
        Self {
            backend,
            model_list_catalog,
        }
    }

    pub(crate) async fn read(
        &self,
        _params: ModelRouteReadParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let backend = self.backend.as_ref().ok_or_else(backend_unavailable)?;
        backend
            .read_model_routes()
            .await
            .map(model_route_read_response)
            .map(|response| Some(response.into()))
            .map_err(model_route_error)
    }

    pub(crate) async fn create(
        &self,
        params: ModelRouteCreateParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let expected_revision = parse_revision(&params.expected_revision)?;
        validate_create_params(&params)?;
        self.model_list_catalog
            .validate_route_semantic_model(&params.semantic_model)
            .await
            .map_err(|_| invalid_params("semanticModel is not in the active model catalog"))?;
        let backend = self.backend.as_ref().ok_or_else(backend_unavailable)?;
        let existing_routes = backend
            .read_model_routes()
            .await
            .map_err(model_route_error)?;
        if params.semantic_model != params.model_tag
            && existing_routes
                .data
                .iter()
                .any(|route| route.model_tag == params.semantic_model)
        {
            return Err(invalid_params(
                "semanticModel must identify a native model, not another RichCodex route",
            ));
        }
        let result = backend
            .create_model_route(ModelRouteCreateRequest {
                expected_revision,
                model_tag: params.model_tag,
                display_name: params.display_name,
                semantic_model: params.semantic_model,
                provider_id: params.provider_id,
                account_id: params.account_id,
                upstream_model_id: params.upstream_model_id,
            })
            .await
            .map_err(model_route_error)?;
        self.publish_backend_routes(backend).await?;
        Ok(Some(model_route_create_response(result).into()))
    }

    pub(crate) async fn retire(
        &self,
        params: ModelRouteRetireParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let expected_revision = parse_revision(&params.expected_revision)?;
        validate_model_tag(&params.model_tag)?;
        let backend = self.backend.as_ref().ok_or_else(backend_unavailable)?;
        let result = backend
            .retire_model_route(expected_revision, params.model_tag)
            .await
            .map_err(model_route_error)?;
        self.publish_backend_routes(backend).await?;
        Ok(Some(model_route_retire_response(result).into()))
    }

    pub(crate) async fn set_targets(
        &self,
        params: ModelRouteSetTargetsParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let expected_revision = parse_revision(&params.expected_revision)?;
        validate_model_tag(&params.model_tag)?;
        if params.targets.is_empty() || params.targets.len() > 64 {
            return Err(invalid_params(
                "targets must contain between 1 and 64 entries",
            ));
        }
        let mut target_ids = std::collections::HashSet::new();
        let mut bindings = std::collections::HashSet::new();
        let mut targets = Vec::with_capacity(params.targets.len());
        for target in params.targets {
            if target.provider_id != "openai" {
                return Err(invalid_params("providerId is not supported by this build"));
            }
            if let Some(id) = target.id.as_deref() {
                validate_text(id, 80, "target id")?;
                if !target_ids.insert(id.to_string()) {
                    return Err(invalid_params("target id is duplicated"));
                }
            }
            validate_text(&target.account_id, 80, "accountId")?;
            validate_trimmed_text(&target.upstream_model_id, 512, "upstreamModelId")?;
            if !bindings.insert((
                target.provider_id.clone(),
                target.account_id.clone(),
                target.upstream_model_id.clone(),
            )) {
                return Err(invalid_params("target binding is duplicated"));
            }
            targets.push(ModelRouteTargetRequest {
                id: target.id,
                provider_id: target.provider_id,
                account_id: target.account_id,
                upstream_model_id: target.upstream_model_id,
            });
        }
        let backend = self.backend.as_ref().ok_or_else(backend_unavailable)?;
        let result = backend
            .set_model_route_targets(ModelRouteSetTargetsRequest {
                expected_revision,
                model_tag: params.model_tag,
                targets,
            })
            .await
            .map_err(model_route_error)?;
        self.publish_backend_routes(backend).await?;
        Ok(Some(model_route_set_targets_response(result).into()))
    }

    async fn publish_backend_routes(
        &self,
        backend: &RichCodexBackendClient,
    ) -> Result<(), JSONRPCErrorError> {
        let routes = backend
            .read_model_routes()
            .await
            .map_err(model_route_error)?;
        self.model_list_catalog
            .publish_runtime_routes(&routes.data)
            .await
            .map(|_| ())
            .map_err(|_| {
                internal_error(
                    "RichCodex route was saved, but its model catalog could not be published safely",
                )
            })
    }
}

fn parse_revision(value: &str) -> Result<u64, JSONRPCErrorError> {
    if value.is_empty()
        || value.len() > 16
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(invalid_params(
            "expectedRevision must be an opaque decimal revision returned by RichCodex",
        ));
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|revision| *revision <= MAX_BACKEND_SAFE_INTEGER)
        .ok_or_else(|| {
            invalid_params(
                "expectedRevision must be an opaque decimal revision returned by RichCodex",
            )
        })
}

fn validate_create_params(params: &ModelRouteCreateParams) -> Result<(), JSONRPCErrorError> {
    validate_model_tag(&params.model_tag)?;
    validate_trimmed_text(&params.display_name, 80, "displayName")?;
    validate_trimmed_text(&params.semantic_model, 200, "semanticModel")?;
    if params.provider_id != "openai" {
        return Err(invalid_params("providerId is not supported by this build"));
    }
    validate_text(&params.account_id, 80, "accountId")?;
    validate_trimmed_text(&params.upstream_model_id, 512, "upstreamModelId")
}

fn validate_model_tag(value: &str) -> Result<(), JSONRPCErrorError> {
    validate_trimmed_text(value, 80, "modelTag")?;
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return Err(invalid_params("modelTag is invalid"));
    };
    if (!first.is_ascii_lowercase() && !first.is_ascii_digit())
        || bytes.any(|byte| {
            !byte.is_ascii_lowercase()
                && !byte.is_ascii_digit()
                && !matches!(byte, b'.' | b'_' | b'/' | b'-')
        })
    {
        return Err(invalid_params("modelTag is invalid"));
    }
    Ok(())
}

fn validate_trimmed_text(
    value: &str,
    max_bytes: usize,
    field: &str,
) -> Result<(), JSONRPCErrorError> {
    validate_text(value, max_bytes, field)?;
    if value.trim() != value {
        return Err(invalid_params(format!("{field} is invalid")));
    }
    Ok(())
}

fn validate_text(value: &str, max_bytes: usize, field: &str) -> Result<(), JSONRPCErrorError> {
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(invalid_params(format!("{field} is invalid")));
    }
    Ok(())
}

fn model_route_read_response(result: ModelRouteReadResult) -> ModelRouteReadResponse {
    ModelRouteReadResponse {
        data: result.data.into_iter().map(model_route).collect(),
        desired_state_revision: result.desired_state_revision.to_string(),
        catalog_revision: result.catalog_revision.to_string(),
    }
}

fn model_route_create_response(result: ModelRouteMutationResult) -> ModelRouteCreateResponse {
    ModelRouteCreateResponse {
        route: model_route(result.route),
        desired_state_revision: result.desired_state_revision.to_string(),
        catalog_revision: result.catalog_revision.to_string(),
    }
}

fn model_route_retire_response(result: ModelRouteMutationResult) -> ModelRouteRetireResponse {
    ModelRouteRetireResponse {
        route: model_route(result.route),
        desired_state_revision: result.desired_state_revision.to_string(),
        catalog_revision: result.catalog_revision.to_string(),
    }
}

fn model_route_set_targets_response(
    result: ModelRouteMutationResult,
) -> ModelRouteSetTargetsResponse {
    ModelRouteSetTargetsResponse {
        route: model_route(result.route),
        desired_state_revision: result.desired_state_revision.to_string(),
        catalog_revision: result.catalog_revision.to_string(),
    }
}

fn model_route(route: ModelSummary) -> ModelRoute {
    ModelRoute {
        model_tag: route.model_tag,
        display_name: route.display_name,
        retired: route.retired,
        semantic_model: route.semantic_model,
        targets: route.targets.into_iter().map(model_route_target).collect(),
    }
}

fn model_route_target(target: ModelTargetSummary) -> ModelRouteTarget {
    ModelRouteTarget {
        id: target.id,
        provider_id: target.provider_id,
        account_id: target.account_id,
        upstream_model_id: target.upstream_model_id,
        priority: target.priority,
        status: match target.status.as_str() {
            "unverified" => ModelRouteTargetStatus::Unverified,
            "reauthenticationRequired" => ModelRouteTargetStatus::ReauthenticationRequired,
            _ => unreachable!("backend response status is validated by the client actor"),
        },
    }
}

fn backend_unavailable() -> JSONRPCErrorError {
    internal_error("RichCodex model-route backend is unavailable")
}

fn model_route_error(error: RichCodexBackendClientError) -> JSONRPCErrorError {
    match error {
        RichCodexBackendClientError::InvalidRequest => {
            invalid_params("model-route request is invalid")
        }
        RichCodexBackendClientError::RevisionConflict => {
            invalid_params("model plane revision does not match")
        }
        RichCodexBackendClientError::ModelTagExists => invalid_params("model tag already exists"),
        RichCodexBackendClientError::ModelTagNotFound => invalid_params("model tag does not exist"),
        RichCodexBackendClientError::AccountUnavailable => {
            invalid_params("selected provider account is unavailable")
        }
        RichCodexBackendClientError::StoreUnavailable => {
            internal_error("RichCodex model-plane store is unavailable")
        }
        RichCodexBackendClientError::Unavailable => backend_unavailable(),
        RichCodexBackendClientError::SourceUnavailable
        | RichCodexBackendClientError::SourceTooLarge
        | RichCodexBackendClientError::InvalidAuthDocument
        | RichCodexBackendClientError::CredentialExpired
        | RichCodexBackendClientError::AccountAlreadyExists
        | RichCodexBackendClientError::AccountLimitReached
        | RichCodexBackendClientError::InvalidApiKey => {
            internal_error("RichCodex model-route backend returned an invalid operation error")
        }
    }
}
