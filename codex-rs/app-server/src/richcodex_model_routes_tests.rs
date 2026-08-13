use super::*;
use crate::richcodex_backend::ModelTargetSummary;
use pretty_assertions::assert_eq;

fn route(model_tag: &str, semantic_model: &str, retired: bool) -> ModelSummary {
    ModelSummary {
        model_tag: model_tag.to_string(),
        display_name: "Display Name".to_string(),
        retired,
        semantic_model: semantic_model.to_string(),
        targets: vec![ModelTargetSummary {
            id: "target-local".to_string(),
            provider_id: "openai".to_string(),
            account_id: "account-local".to_string(),
            upstream_model_id: "upstream-model".to_string(),
            priority: 0,
            status: "unverified".to_string(),
        }],
    }
}

#[test]
fn active_route_borrows_semantic_metadata_without_provider_details() {
    let template = codex_models_manager::bundled_models_response()
        .expect("bundled model catalog")
        .models
        .into_iter()
        .next()
        .expect("bundled model");
    let projected = project_model_routes(
        std::slice::from_ref(&template),
        &[route("stable-tag", &template.slug, false)],
    )
    .expect("route projection");
    let mut expected = template;
    expected.slug = "stable-tag".to_string();
    expected.display_name = "Display Name".to_string();
    expected.visibility = ModelVisibility::List;
    expected.supported_in_api = true;
    expected.upgrade = None;
    expected.availability_nux = None;
    expected.used_fallback_model_metadata = false;

    assert_eq!(
        projected,
        ModelsResponse {
            models: vec![expected]
        }
    );
}

#[test]
fn retired_route_projects_a_hidden_tombstone() {
    let template = codex_models_manager::bundled_models_response()
        .expect("bundled model catalog")
        .models
        .into_iter()
        .next()
        .expect("bundled model");
    let projected = project_model_routes(
        std::slice::from_ref(&template),
        &[route(&template.slug, &template.slug, true)],
    )
    .expect("route projection");

    assert_eq!(projected.models[0].visibility, ModelVisibility::Hide);
    assert_eq!(projected.models[0].slug, template.slug);
}

#[test]
fn unknown_or_chained_semantic_models_fail_without_partial_projection() {
    let template = codex_models_manager::bundled_models_response()
        .expect("bundled model catalog")
        .models
        .into_iter()
        .next()
        .expect("bundled model");

    assert_eq!(
        project_model_routes(&[template], &[route("stable-tag", "unknown", false)]),
        Err(ModelRouteProjectionError)
    );
    assert_eq!(
        project_model_routes(
            &[],
            &[
                route("stable-a", "stable-b", false),
                route("stable-b", "stable-b", false),
            ],
        ),
        Err(ModelRouteProjectionError)
    );
}
