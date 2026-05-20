use crate::client::{ChatRequest, ChatCompletionResponse, ModelClient};
use crate::config::GatewayConfig;

pub struct FallbackResult {
    pub data: ChatCompletionResponse,
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
                // Replace model field with actual model used
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

#[derive(Debug, thiserror::Error)]
pub enum FallbackError {
    #[error("no models in fallback chain")]
    NoModelsInChain,
    #[error("all models failed: {0}")]
    AllModelsFailed(String),
}
