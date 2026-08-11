use super::merge_catalog;
use super::with_catalog_overlay;
use crate::bundled_models_response;
use crate::manager::RefreshStrategy;
use crate::manager::StaticModelsManager;
use codex_http_client::HttpClientFactory;
use codex_http_client::OutboundProxyPolicy;
use codex_protocol::openai_models::ModelsResponse;
use std::sync::Arc;

const HTTP_CLIENT_FACTORY: HttpClientFactory =
    HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault);

#[test]
fn overlay_preserves_unrelated_models_and_replaces_matching_slugs() {
    let bundled = bundled_models_response().expect("bundled model catalog should parse");
    let first = bundled
        .models
        .first()
        .expect("bundled catalog should not be empty");
    let second = bundled
        .models
        .get(1)
        .expect("bundled catalog should contain at least two models");

    let mut replacement = first.clone();
    replacement.description = Some("overlay replacement".to_string());
    let mut addition = first.clone();
    addition.slug = "provider/custom-model".to_string();
    addition.display_name = "Custom Model".to_string();

    let merged = merge_catalog(
        vec![first.clone(), second.clone()],
        &[replacement.clone(), addition.clone()],
    );

    assert_eq!(merged, vec![replacement, second.clone(), addition]);
}

#[tokio::test]
async fn overlay_model_can_be_selected_without_replacing_the_inner_default() {
    let bundled = bundled_models_response().expect("bundled model catalog should parse");
    let inner_default = bundled.models[0].slug.clone();
    let mut addition = bundled.models[0].clone();
    addition.slug = "provider/custom-model".to_string();
    addition.priority = 1000;
    let manager = with_catalog_overlay(
        Arc::new(StaticModelsManager::new(None, bundled)),
        Some(ModelsResponse {
            models: vec![addition.clone()],
        }),
    );

    let selected = manager
        .get_default_model(
            &Some(addition.slug.clone()),
            /* allow_provider_model_fallback */ true,
            RefreshStrategy::Offline,
            HTTP_CLIENT_FACTORY,
        )
        .await;
    let default = manager
        .get_default_model(
            &None,
            /* allow_provider_model_fallback */ true,
            RefreshStrategy::Offline,
            HTTP_CLIENT_FACTORY,
        )
        .await;

    assert_eq!(selected, addition.slug);
    assert_eq!(default, inner_default);
}

#[tokio::test]
async fn overlay_priority_drives_both_picker_and_implicit_default() {
    let bundled = bundled_models_response().expect("bundled model catalog should parse");
    let mut addition = bundled.models[0].clone();
    addition.slug = "provider/high-priority-model".to_string();
    addition.priority = 0;
    let manager = with_catalog_overlay(
        Arc::new(StaticModelsManager::new(None, bundled)),
        Some(ModelsResponse {
            models: vec![addition.clone()],
        }),
    );

    let models = manager
        .list_models(RefreshStrategy::Offline, HTTP_CLIENT_FACTORY)
        .await;
    let default = manager
        .get_default_model(
            &None,
            /* allow_provider_model_fallback */ true,
            RefreshStrategy::Offline,
            HTTP_CLIENT_FACTORY,
        )
        .await;

    assert_eq!(
        models
            .iter()
            .find(|model| model.is_default)
            .map(|model| model.model.as_str()),
        Some(addition.slug.as_str())
    );
    assert_eq!(default, addition.slug);
}
