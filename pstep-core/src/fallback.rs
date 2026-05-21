use crate::client::{ChatCompletionResponse, ChatRequest, ModelClient, StreamHandle};
use crate::config::GatewayConfig;

#[derive(Debug)]
pub struct FallbackResult {
    pub data: ChatCompletionResponse,
    pub actual_model: String,
    pub requested_model: String,
}

pub struct StreamFallbackResult {
    pub handle: StreamHandle,
    pub actual_model: String,
    pub requested_model: String,
}

pub async fn handle_with_fallback(
    config: &GatewayConfig,
    client: &ModelClient,
    request: &ChatRequest,
) -> Result<FallbackResult, FallbackError> {
    let requested_model = request.model.clone();
    let chain = config.get_fallback_chain(&requested_model);

    if chain.is_empty() {
        return Err(FallbackError::NoModelsInChain);
    }

    let mut last_error = None;

    for model_name in &chain {
        let model_config = match config.get_model(model_name) {
            Some(c) => c,
            None => {
                last_error = Some(format!("model '{}' not configured", model_name));
                continue;
            }
        };

        match client.call_model(model_config, request).await {
            Ok(mut resp) => {
                resp.model = model_name.to_string();
                return Ok(FallbackResult {
                    data: resp,
                    actual_model: model_name.to_string(),
                    requested_model,
                });
            }
            Err(e) => {
                last_error = Some(e.to_string());
                tracing::warn!(model = model_name, error = %e, "model call failed, trying next");
            }
        }
    }

    Err(FallbackError::AllModelsFailed(
        last_error.unwrap_or_else(|| "unknown error".to_string()),
    ))
}

pub async fn handle_stream_with_fallback(
    config: &GatewayConfig,
    client: &ModelClient,
    request: &ChatRequest,
) -> Result<StreamFallbackResult, FallbackError> {
    let requested_model = request.model.clone();
    let chain = config.get_fallback_chain(&requested_model);

    if chain.is_empty() {
        return Err(FallbackError::NoModelsInChain);
    }

    let mut last_error = None;

    for model_name in &chain {
        let model_config = match config.get_model(model_name) {
            Some(c) => c,
            None => {
                last_error = Some(format!("model '{}' not configured", model_name));
                continue;
            }
        };

        match client.call_model_stream(model_config, request).await {
            Ok(handle) => {
                return Ok(StreamFallbackResult {
                    handle,
                    actual_model: model_name.to_string(),
                    requested_model,
                });
            }
            Err(e) => {
                last_error = Some(e.to_string());
                tracing::warn!(model = model_name, error = %e, "stream call failed, trying next");
            }
        }
    }

    Err(FallbackError::AllModelsFailed(
        last_error.unwrap_or_else(|| "unknown error".to_string()),
    ))
}

#[derive(Debug, thiserror::Error)]
pub enum FallbackError {
    #[error("no models in fallback chain")]
    NoModelsInChain,
    #[error("all models failed: {0}")]
    AllModelsFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GatewayConfig, ServerConfig};
    use mockito::{Server, ServerGuard};
    use std::collections::HashMap;

    fn test_config_with_chain(
        server_url: &str,
        models: Vec<(&str, &str)>,
        chain: Vec<&str>,
    ) -> GatewayConfig {
        let mut model_map = HashMap::new();
        for (name, model_name) in &models {
            model_map.insert(
                name.to_string(),
                crate::config::ModelConfig {
                    url: format!("{}/v1/chat/completions", server_url),
                    api_key_env: None,
                    api_key: Some("test-key".to_string()),
                    remote_model: Some(model_name.to_string()),
                },
            );
        }

        let mut fallback_chains = HashMap::new();
        fallback_chains.insert(
            "default".to_string(),
            chain.iter().map(|s| s.to_string()).collect(),
        );

        GatewayConfig {
            models: model_map,
            fallback_chains,
            server: ServerConfig {
                port: 3000,
                ws_port: 3400,
            },
        }
    }

    fn test_request(model: &str) -> ChatRequest {
        ChatRequest {
            model: model.to_string(),
            messages: vec![crate::client::Message {
                role: Some("user".to_string()),
                content: Some("hello".to_string()),
                tool_calls: None,
                tool_call_id: None,
            }],
            stream: None,
            tools: None,
            max_tokens: None,
        }
    }

    fn success_response(model: &str) -> String {
        format!(
            r#"{{"id":"chatcmpl-1","model":"{}","choices":[{{"message":{{"role":"assistant","content":"ok"}}}}],"usage":{{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8}}}}"#,
            model
        )
    }

    async fn start_mock_server() -> ServerGuard {
        Server::new_async().await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fallback_first_model_success() {
        let mut server = start_mock_server().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_body(success_response("model-a"))
            .create();

        let config = test_config_with_chain(
            &server.url(),
            vec![("model-a", "model-a"), ("model-b", "model-b")],
            vec!["model-a", "model-b"],
        );
        let client = ModelClient::new();
        let req = test_request("default");

        let result = handle_with_fallback(&config, &client, &req).await.unwrap();
        assert_eq!(result.actual_model, "model-a");
        assert_eq!(result.data.model, "model-a");

        mock.assert();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fallback_first_fails_second_succeeds() {
        let mut server = start_mock_server().await;

        let mock_fail = server
            .mock("POST", "/v1/chat/completions")
            .match_body(mockito::Matcher::JsonString(
                r#"{"model":"model-a","messages":[{"role":"user","content":"hello"}],"stream":false}"#
                    .to_string(),
            ))
            .with_status(500)
            .with_body("server error")
            .create();

        let mock_success = server
            .mock("POST", "/v1/chat/completions")
            .match_body(mockito::Matcher::JsonString(
                r#"{"model":"model-b","messages":[{"role":"user","content":"hello"}],"stream":false}"#
                    .to_string(),
            ))
            .with_body(success_response("model-b"))
            .create();

        let config = test_config_with_chain(
            &server.url(),
            vec![("model-a", "model-a"), ("model-b", "model-b")],
            vec!["model-a", "model-b"],
        );
        let client = ModelClient::new();
        let req = test_request("default");

        let result = handle_with_fallback(&config, &client, &req).await.unwrap();
        assert_eq!(result.actual_model, "model-b");
        assert_eq!(result.data.model, "model-b");

        mock_fail.assert();
        mock_success.assert();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fallback_all_models_fail() {
        let mut server = start_mock_server().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .expect(2)
            .with_status(500)
            .with_body("server error")
            .create();

        let config = test_config_with_chain(
            &server.url(),
            vec![("model-a", "model-a"), ("model-b", "model-b")],
            vec!["model-a", "model-b"],
        );
        let client = ModelClient::new();
        let req = test_request("default");

        let err = handle_with_fallback(&config, &client, &req)
            .await
            .unwrap_err();
        assert!(matches!(err, FallbackError::AllModelsFailed(_)));

        mock.assert();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fallback_empty_chain() {
        let server = start_mock_server().await;
        let config = test_config_with_chain(&server.url(), vec![("model-a", "model-a")], vec![]);
        let client = ModelClient::new();
        let req = test_request("nonexistent");

        let err = handle_with_fallback(&config, &client, &req)
            .await
            .unwrap_err();
        assert!(matches!(err, FallbackError::NoModelsInChain));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fallback_model_not_configured() {
        let mut server = start_mock_server().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_body(success_response("model-b"))
            .create();

        let config = test_config_with_chain(
            &server.url(),
            vec![("model-b", "model-b")],
            vec!["model-a", "model-b"],
        );
        let client = ModelClient::new();
        let req = test_request("default");

        let result = handle_with_fallback(&config, &client, &req).await.unwrap();
        assert_eq!(result.actual_model, "model-b");

        mock.assert();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_fallback_first_model_success() {
        let mut server = start_mock_server().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_header("Content-Type", "text/event-stream")
            .with_body("data: {\"id\":\"chatcmpl-1\",\"model\":\"model-a\",\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n")
            .create();

        let config = test_config_with_chain(
            &server.url(),
            vec![("model-a", "model-a"), ("model-b", "model-b")],
            vec!["model-a", "model-b"],
        );
        let client = ModelClient::new();
        let req = test_request("default");

        let result = handle_stream_with_fallback(&config, &client, &req)
            .await
            .unwrap();
        assert_eq!(result.actual_model, "model-a");

        mock.assert();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_fallback_first_fails_second_succeeds() {
        let mut server = start_mock_server().await;

        let mock_fail = server
            .mock("POST", "/v1/chat/completions")
            .with_status(500)
            .with_body("server error")
            .create();

        let mock_success = server
            .mock("POST", "/v1/chat/completions")
            .with_header("Content-Type", "text/event-stream")
            .with_body("data: {\"id\":\"chatcmpl-1\",\"model\":\"model-b\",\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n")
            .create();

        let config = test_config_with_chain(
            &server.url(),
            vec![("model-a", "model-a"), ("model-b", "model-b")],
            vec!["model-a", "model-b"],
        );
        let client = ModelClient::new();
        let req = test_request("default");

        let result = handle_stream_with_fallback(&config, &client, &req)
            .await
            .unwrap();
        assert_eq!(result.actual_model, "model-b");

        mock_fail.assert();
        mock_success.assert();
    }
}
