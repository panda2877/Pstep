use crate::config::ModelConfig;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Clone)]
pub struct ModelEntry {
    pub name: String,
    pub config: ModelConfig,
    pub fallback_chain: Vec<String>,
}

pub struct ModelStore {
    models: RwLock<HashMap<String, ModelEntry>>,
}

impl ModelStore {
    pub fn new() -> Self {
        Self {
            models: RwLock::new(HashMap::new()),
        }
    }

    pub fn add(&self, entry: ModelEntry) -> Option<ModelEntry> {
        let mut models = self.models.write().unwrap();
        models.insert(entry.name.clone(), entry)
    }

    pub fn remove(&self, name: &str) -> Option<ModelEntry> {
        let mut models = self.models.write().unwrap();
        models.remove(name)
    }

    pub fn get(&self, name: &str) -> Option<ModelEntry> {
        let models = self.models.read().unwrap();
        models.get(name).cloned()
    }

    pub fn list(&self) -> Vec<String> {
        let models = self.models.read().unwrap();
        models.keys().cloned().collect()
    }

    pub fn load_from_config(config: &crate::config::GatewayConfig) -> Self {
        let store = Self::new();
        for (name, model_config) in &config.models {
            let fallback_chain = config.get_fallback_chain(name)
                .into_iter().map(String::from).collect();
            store.add(ModelEntry {
                name: name.clone(),
                config: model_config.clone(),
                fallback_chain,
            });
        }
        store
    }
}
