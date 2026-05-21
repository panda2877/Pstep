use crate::client::{ChatRequest, ChatCompletionResponse, ClientError, StreamHandle};
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

impl Default for ModelStore {
    fn default() -> Self {
        Self::new()
    }
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

/// Health status of a model provider
#[derive(Debug, Clone, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Unhealthy(String),
    Unknown,
}

/// Trait for model providers that can make LLM calls
#[async_trait::async_trait]
pub trait ModelProvider: Send + Sync {
    /// Provider name (e.g., "openai", "anthropic")
    fn name(&self) -> &str;

    /// Non-streaming chat completion
    async fn chat(
        &self,
        config: &ModelConfig,
        request: &ChatRequest,
    ) -> Result<ChatCompletionResponse, ClientError>;

    /// Streaming chat completion
    async fn chat_stream(
        &self,
        config: &ModelConfig,
        request: &ChatRequest,
    ) -> Result<StreamHandle, ClientError>;

    /// Health check - lightweight probe to verify connectivity
    async fn health_check(&self, config: &ModelConfig) -> HealthStatus;
}

/// OpenAI-compatible provider (works with any OpenAI API-compatible endpoint)
pub struct OpenAIProvider {
    client: reqwest::Client,
}

impl Default for OpenAIProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenAIProvider {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to create HTTP client");
        Self { client }
    }
}

#[async_trait::async_trait]
impl ModelProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn chat(
        &self,
        config: &ModelConfig,
        request: &ChatRequest,
    ) -> Result<ChatCompletionResponse, ClientError> {
        let api_key = config.api_key.as_deref().unwrap_or("");
        let remote_model = config.remote_model.as_deref().unwrap_or(&request.model);

        let body = serde_json::json!({
            "model": remote_model,
            "messages": request.messages,
            "stream": false,
        });

        let resp = self
            .client
            .post(&config.url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .send()
            .await
            .map_err(ClientError::Network)?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::Upstream(status.as_u16(), text));
        }

        resp.json::<ChatCompletionResponse>()
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    async fn chat_stream(
        &self,
        config: &ModelConfig,
        request: &ChatRequest,
    ) -> Result<StreamHandle, ClientError> {
        let api_key = config.api_key.as_deref().unwrap_or("");
        let remote_model = config.remote_model.as_deref().unwrap_or(&request.model);

        let body = serde_json::json!({
            "model": remote_model,
            "messages": request.messages,
            "stream": true,
        });

        let resp = self
            .client
            .post(&config.url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(ClientError::Network)?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::Upstream(status.as_u16(), text));
        }

        let model_name = remote_model.to_string();
        Ok(StreamHandle::new(resp, model_name))
    }

    async fn health_check(&self, config: &ModelConfig) -> HealthStatus {
        let api_key = config.api_key.as_deref().unwrap_or("");

        // Try to get models list as a health check
        let base_url = config.url.trim_end_matches("/v1/chat/completions");
        let models_url = format!("{}/v1/models", base_url);

        match self
            .client
            .get(&models_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => HealthStatus::Healthy,
            Ok(resp) => HealthStatus::Unhealthy(format!("status {}", resp.status())),
            Err(e) => HealthStatus::Unhealthy(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_model_config(url: &str) -> ModelConfig {
        ModelConfig {
            url: url.to_string(),
            api_key_env: None,
            api_key: Some("test-key".to_string()),
            remote_model: Some("test-model".to_string()),
        }
    }

    fn test_entry(name: &str) -> ModelEntry {
        ModelEntry {
            name: name.to_string(),
            config: test_model_config(&format!("http://localhost:3000/{}", name)),
            fallback_chain: vec![],
        }
    }

    #[test]
    fn store_add_and_get() {
        let store = ModelStore::new();
        let entry = test_entry("model-a");

        store.add(entry.clone());
        let got = store.get("model-a").unwrap();
        assert_eq!(got.name, "model-a");
        assert_eq!(got.config.url, entry.config.url);
    }

    #[test]
    fn store_get_nonexistent_returns_none() {
        let store = ModelStore::new();
        assert!(store.get("nonexistent").is_none());
    }

    #[test]
    fn store_add_replaces_existing() {
        let store = ModelStore::new();

        let entry1 = ModelEntry {
            name: "model-a".to_string(),
            config: test_model_config("http://old-url"),
            fallback_chain: vec![],
        };
        let entry2 = ModelEntry {
            name: "model-a".to_string(),
            config: test_model_config("http://new-url"),
            fallback_chain: vec![],
        };

        store.add(entry1);
        let replaced = store.add(entry2);
        assert!(replaced.is_some());
        assert_eq!(replaced.unwrap().config.url, "http://old-url");

        let got = store.get("model-a").unwrap();
        assert_eq!(got.config.url, "http://new-url");
    }

    #[test]
    fn store_remove_existing() {
        let store = ModelStore::new();
        store.add(test_entry("model-a"));

        let removed = store.remove("model-a");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "model-a");
        assert!(store.get("model-a").is_none());
    }

    #[test]
    fn store_remove_nonexistent_returns_none() {
        let store = ModelStore::new();
        assert!(store.remove("nonexistent").is_none());
    }

    #[test]
    fn store_list_returns_all_names() {
        let store = ModelStore::new();
        store.add(test_entry("model-a"));
        store.add(test_entry("model-b"));
        store.add(test_entry("model-c"));

        let mut names = store.list();
        names.sort();
        assert_eq!(names, vec!["model-a", "model-b", "model-c"]);
    }

    #[test]
    fn store_list_empty() {
        let store = ModelStore::new();
        assert!(store.list().is_empty());
    }

    #[test]
    fn store_load_from_config() {
        let mut models = HashMap::new();
        models.insert("gpt-4".to_string(), test_model_config("http://api.openai.com"));
        models.insert("gpt-3.5-turbo".to_string(), test_model_config("http://api.openai.com"));

        let mut fallback_chains = HashMap::new();
        fallback_chains.insert("default".to_string(), vec!["gpt-4".to_string(), "gpt-3.5-turbo".to_string()]);

        let config = crate::config::GatewayConfig {
            models,
            fallback_chains,
            server: crate::config::ServerConfig {
                port: 3000,
                ws_port: 3400,
            },
        };

        let store = ModelStore::load_from_config(&config);
        let names = store.list();
        assert_eq!(names.len(), 2);
        assert!(store.get("gpt-4").is_some());
        assert!(store.get("gpt-3.5-turbo").is_some());

        // Check fallback chain is loaded
        let entry = store.get("gpt-4").unwrap();
        assert_eq!(entry.fallback_chain, vec!["gpt-4", "gpt-3.5-turbo"]);
    }
}
