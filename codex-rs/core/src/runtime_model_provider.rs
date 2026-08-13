use codex_model_provider::SharedModelProvider;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::RwLock;

/// Process-owned model routes published by a supervised runtime provider.
///
/// The provider is immutable for one app-server lifetime. Its active stable
/// tags are replaced atomically after backend desired state has been validated
/// and published into the live model catalog.
#[derive(Clone, Debug)]
pub struct RuntimeModelProviderRoutes {
    provider_id: String,
    provider: SharedModelProvider,
    active_model_tags: Arc<RwLock<HashSet<String>>>,
}

impl RuntimeModelProviderRoutes {
    pub fn new(
        provider_id: impl Into<String>,
        provider: SharedModelProvider,
        active_model_tags: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            provider,
            active_model_tags: Arc::new(RwLock::new(active_model_tags.into_iter().collect())),
        }
    }

    /// Resolves one active stable tag without exposing provider credentials.
    pub fn resolve(&self, model_tag: &str) -> Option<(String, SharedModelProvider)> {
        self.active_model_tags
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(model_tag)
            .then(|| (self.provider_id.clone(), Arc::clone(&self.provider)))
    }

    /// Returns whether a session is currently bound to this runtime provider.
    pub fn owns_provider_id(&self, provider_id: &str) -> bool {
        self.provider_id == provider_id
    }

    /// Replaces the complete active-tag snapshot after successful publication.
    pub fn replace_active_model_tags(&self, model_tags: impl IntoIterator<Item = String>) {
        *self
            .active_model_tags
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = model_tags.into_iter().collect();
    }
}

#[cfg(test)]
mod tests {
    use codex_model_provider::create_ephemeral_openai_bearer_model_provider;

    use super::*;

    #[test]
    fn active_tags_are_replaced_as_one_snapshot() {
        let routes = RuntimeModelProviderRoutes::new(
            "richcodex",
            create_ephemeral_openai_bearer_model_provider(
                48767,
                "private-loopback-capability".to_string(),
            ),
            ["fast".to_string()],
        );

        let (provider_id, provider) = routes.resolve("fast").expect("route should resolve");
        assert_eq!(provider_id, "richcodex");
        assert!(routes.owns_provider_id("richcodex"));
        assert!(!routes.owns_provider_id("openai"));
        assert_eq!(
            provider.info().base_url.as_deref(),
            Some("http://127.0.0.1:48767/v1")
        );
        assert!(routes.resolve("review").is_none());

        routes.replace_active_model_tags(["review".to_string()]);
        assert!(routes.resolve("fast").is_none());
        assert!(routes.resolve("review").is_some());
    }
}
