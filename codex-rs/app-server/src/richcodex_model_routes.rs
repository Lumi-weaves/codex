use crate::richcodex_backend::ModelSummary;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ModelsResponse;
use std::collections::HashSet;

/// Build a secret-free model-catalog layer from RichCodex's stable tag projection.
///
/// A route borrows capability metadata from its semantic model. Retired routes remain as hidden
/// tombstones so a same-slug model from a lower catalog layer cannot reappear in the picker.
pub(crate) fn project_model_routes(
    candidates: &[ModelInfo],
    routes: &[ModelSummary],
) -> Result<ModelsResponse, ModelRouteProjectionError> {
    let route_tags = routes
        .iter()
        .map(|route| route.model_tag.as_str())
        .collect::<HashSet<_>>();
    let mut projected = Vec::with_capacity(routes.len());
    for route in routes {
        if route.semantic_model != route.model_tag
            && route_tags.contains(route.semantic_model.as_str())
        {
            return Err(ModelRouteProjectionError);
        }
        let template = candidates
            .iter()
            .find(|candidate| candidate.slug == route.semantic_model)
            .or_else(|| {
                route
                    .retired
                    .then(|| {
                        candidates
                            .iter()
                            .find(|candidate| candidate.slug == route.model_tag)
                    })
                    .flatten()
            })
            .ok_or(ModelRouteProjectionError)?;
        let mut model = template.clone();
        model.slug = route.model_tag.clone();
        model.display_name = route.display_name.clone();
        model.visibility = if route.retired {
            ModelVisibility::Hide
        } else {
            ModelVisibility::List
        };
        model.supported_in_api = true;
        model.upgrade = None;
        model.availability_nux = None;
        model.used_fallback_model_metadata = false;
        projected.push(model);
    }
    Ok(ModelsResponse { models: projected })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ModelRouteProjectionError;

#[cfg(test)]
#[path = "richcodex_model_routes_tests.rs"]
mod tests;
