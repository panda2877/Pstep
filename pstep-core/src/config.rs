use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct GatewayConfig {
    pub models: HashMap<String, ModelConfig>,
    #[serde(rename = "fallbackChains")]
    pub fallback_chains: HashMap<String, Vec<String>>,
    pub server: ServerConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModelConfig {
    pub url: String,
    #[serde(rename = "apiKeyEnv")]
    pub api_key_env: Option<String>,
    #[serde(default, rename = "apiKey")]
    pub api_key: Option<String>,
    #[serde(rename = "remoteModel")]
    pub remote_model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_ws_port", rename = "wsPort")]
    pub ws_port: u16,
}

fn default_port() -> u16 {
    3000
}

fn default_ws_port() -> u16 {
    3400
}

impl GatewayConfig {
    pub fn load(config_dir: &Path) -> Result<Self, ConfigError> {
        let config_path = config_dir.join("models.json");
        let raw = fs::read_to_string(&config_path).map_err(|e| ConfigError::Io(e, config_path))?;
        let mut config: GatewayConfig =
            serde_json::from_str(&raw).map_err(ConfigError::Parse)?;

        // Resolve apiKeyEnv → actual env var value
        for model in config.models.values_mut() {
            if let Some(ref env_name) = model.api_key_env {
                model.api_key = std::env::var(env_name).ok();
            }
        }

        Ok(config)
    }

    pub fn get_model(&self, name: &str) -> Option<&ModelConfig> {
        self.models.get(name)
    }

    pub fn get_fallback_chain(&self, model: &str) -> Vec<&str> {
        self.fallback_chains
            .get(model)
            .or_else(|| self.fallback_chains.get("default"))
            .map(|chain| chain.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file {1}: {0}")]
    Io(std::io::Error, std::path::PathBuf),
    #[error("failed to parse config: {0}")]
    Parse(serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn sample_config_json() -> &'static str {
        r#"{
            "models": {
                "deepseek": {
                    "url": "https://api.deepseek.com/v1/chat/completions",
                    "apiKeyEnv": "DEEPSEEK_API_KEY",
                    "remoteModel": "deepseek-chat"
                },
                "openai": {
                    "url": "https://api.openai.com/v1/chat/completions",
                    "apiKeyEnv": "OPENAI_API_KEY"
                }
            },
            "fallbackChains": {
                "default": ["deepseek", "openai"],
                "fast": ["deepseek"]
            },
            "server": {
                "port": 8080,
                "wsPort": 8400
            }
        }"#
    }

    fn minimal_config_json() -> &'static str {
        r#"{
            "models": {
                "mimo": {
                    "url": "https://example.com/v1",
                    "apiKeyEnv": "MIMO_KEY",
                    "remoteModel": "mimo-v2.5"
                }
            },
            "fallbackChains": {
                "default": ["mimo"]
            },
            "server": {}
        }"#
    }

    fn write_config(dir: &Path, content: &str) {
        fs::write(dir.join("models.json"), content).unwrap();
    }

    #[test]
    fn deserialize_full_config() {
        let config: GatewayConfig = serde_json::from_str(sample_config_json()).unwrap();

        assert_eq!(config.models.len(), 2);
        assert!(config.models.contains_key("deepseek"));
        assert!(config.models.contains_key("openai"));

        assert_eq!(config.fallback_chains.len(), 2);
        assert_eq!(config.fallback_chains["default"], vec!["deepseek", "openai"]);
        assert_eq!(config.fallback_chains["fast"], vec!["deepseek"]);

        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.ws_port, 8400);
    }

    #[test]
    fn deserialize_model_config_fields() {
        let config: GatewayConfig = serde_json::from_str(sample_config_json()).unwrap();

        let ds = config.models.get("deepseek").unwrap();
        assert_eq!(ds.url, "https://api.deepseek.com/v1/chat/completions");
        assert_eq!(ds.api_key_env.as_deref(), Some("DEEPSEEK_API_KEY"));
        assert_eq!(ds.remote_model.as_deref(), Some("deepseek-chat"));
        assert_eq!(ds.api_key, None); // not resolved yet

        let oai = config.models.get("openai").unwrap();
        assert_eq!(oai.remote_model, None); // optional field absent
    }

    #[test]
    fn default_port_values() {
        let config: GatewayConfig = serde_json::from_str(minimal_config_json()).unwrap();
        assert_eq!(config.server.port, 3000);
        assert_eq!(config.server.ws_port, 3400);
    }

    #[test]
    fn get_model_existing() {
        let config: GatewayConfig = serde_json::from_str(sample_config_json()).unwrap();
        let model = config.get_model("deepseek").unwrap();
        assert_eq!(model.url, "https://api.deepseek.com/v1/chat/completions");
    }

    #[test]
    fn get_model_missing() {
        let config: GatewayConfig = serde_json::from_str(sample_config_json()).unwrap();
        assert!(config.get_model("nonexistent").is_none());
    }

    #[test]
    fn get_fallback_chain_specific() {
        let config: GatewayConfig = serde_json::from_str(sample_config_json()).unwrap();
        let chain = config.get_fallback_chain("fast");
        assert_eq!(chain, vec!["deepseek"]);
    }

    #[test]
    fn get_fallback_chain_unknown_falls_back_to_default() {
        let config: GatewayConfig = serde_json::from_str(sample_config_json()).unwrap();
        // "unknown" has no chain, should fall back to "default"
        let chain = config.get_fallback_chain("unknown");
        assert_eq!(chain, vec!["deepseek", "openai"]);
    }

    #[test]
    fn get_fallback_chain_empty_when_no_default() {
        let json = r#"{
            "models": {},
            "fallbackChains": {},
            "server": {}
        }"#;
        let config: GatewayConfig = serde_json::from_str(json).unwrap();
        let chain = config.get_fallback_chain("anything");
        assert!(chain.is_empty());
    }

    #[test]
    fn load_from_file() {
        let tmp = TempDir::new().unwrap();
        write_config(tmp.path(), sample_config_json());

        let config = GatewayConfig::load(tmp.path()).unwrap();
        assert_eq!(config.models.len(), 2);
        assert_eq!(config.server.port, 8080);
    }

    #[test]
    fn load_resolves_api_key_env() {
        let tmp = TempDir::new().unwrap();
        write_config(tmp.path(), sample_config_json());

        unsafe { std::env::set_var("TEST_CFG_DEEPSEEK_KEY", "sk-test-123"); }

        // Patch config to use our test env var
        let patched = sample_config_json().replace("DEEPSEEK_API_KEY", "TEST_CFG_DEEPSEEK_KEY");
        write_config(tmp.path(), &patched);

        let config = GatewayConfig::load(tmp.path()).unwrap();
        let ds = config.models.get("deepseek").unwrap();
        assert_eq!(ds.api_key.as_deref(), Some("sk-test-123"));

        unsafe { std::env::remove_var("TEST_CFG_DEEPSEEK_KEY"); }
    }

    #[test]
    fn load_api_key_missing_env_is_none() {
        let tmp = TempDir::new().unwrap();
        write_config(tmp.path(), sample_config_json());

        // Ensure the env var does NOT exist
        unsafe { std::env::remove_var("DEEPSEEK_API_KEY"); }

        let config = GatewayConfig::load(tmp.path()).unwrap();
        let ds = config.models.get("deepseek").unwrap();
        assert_eq!(ds.api_key, None);
    }

    #[test]
    fn load_file_not_found() {
        let tmp = TempDir::new().unwrap();
        let result = GatewayConfig::load(tmp.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::Io(_, path) => {
                assert!(path.ends_with("models.json"));
            }
            other => panic!("expected Io error, got: {}", other),
        }
    }

    #[test]
    fn load_invalid_json() {
        let tmp = TempDir::new().unwrap();
        write_config(tmp.path(), "{ not valid json!!!");

        let result = GatewayConfig::load(tmp.path());
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ConfigError::Parse(_)));
    }
}
