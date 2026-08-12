use codex_protocol::openai_models::ModelPreset;
use std::sync::RwLock;
use std::sync::TryLockError;

#[derive(Debug)]
pub(crate) struct ModelCatalog {
    models: RwLock<Vec<ModelPreset>>,
}

impl ModelCatalog {
    pub(crate) fn new(models: Vec<ModelPreset>) -> Self {
        Self {
            models: RwLock::new(models),
        }
    }

    pub(crate) fn try_list_models(&self) -> Result<Vec<ModelPreset>, ModelCatalogUnavailable> {
        match self.models.try_read() {
            Ok(models) => Ok(models.clone()),
            Err(TryLockError::Poisoned(poisoned)) => Ok(poisoned.into_inner().clone()),
            Err(TryLockError::WouldBlock) => Err(ModelCatalogUnavailable),
        }
    }

    pub(crate) fn replace_models(&self, models: Vec<ModelPreset>) {
        match self.models.write() {
            Ok(mut current) => *current = models,
            Err(poisoned) => *poisoned.into_inner() = models,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ModelCatalogUnavailable;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TEST_MODEL_PRESETS;

    #[test]
    fn replace_models_updates_shared_catalog_in_place() {
        let catalog = ModelCatalog::new(vec![TEST_MODEL_PRESETS[0].clone()]);
        let replacement = TEST_MODEL_PRESETS[1].clone();

        catalog.replace_models(vec![replacement.clone()]);

        assert_eq!(catalog.try_list_models().unwrap(), vec![replacement]);
    }
}
