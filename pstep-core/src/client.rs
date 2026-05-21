use crate::config::ModelConfig;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Message {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: Option<String>,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Choice {
    pub index: Option<u32>,
    pub message: Option<Message>,
    pub delta: Option<Message>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    #[serde(rename = "prompt_tokens")]
    pub prompt_tokens: Option<u32>,
    #[serde(rename = "completion_tokens")]
    pub completion_tokens: Option<u32>,
    #[serde(rename = "total_tokens")]
    pub total_tokens: Option<u32>,
}

/// A single SSE chunk from a streaming response
#[derive(Debug, Clone)]
pub struct StreamChunk {
    pub id: Option<String>,
    pub model: Option<String>,
    pub delta_content: Option<String>,
    pub finish_reason: Option<String>,
    /// Tool call deltas: index, id, name, arguments (partial)
    pub tool_call_deltas: Vec<ToolCallDelta>,
}

#[derive(Debug, Clone)]
pub struct ToolCallDelta {
    pub index: usize,
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: Option<String>,
}

/// Stats extracted from a completed stream
#[derive(Debug, Clone, Default)]
pub struct StreamStats {
    pub usage: Option<Usage>,
    pub latency_ms: u64,
    pub model: String,
}

pub struct ModelClient {
    http: Client,
}

impl ModelClient {
    pub fn new() -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to create HTTP client");
        Self { http }
    }

    pub async fn call_model(
        &self,
        config: &ModelConfig,
        request: &ChatRequest,
    ) -> Result<ChatCompletionResponse, ClientError> {
        let api_key = config.api_key.as_deref().unwrap_or("");
        let remote_model = config.remote_model.as_deref().unwrap_or(&request.model);

        let mut body = serde_json::json!({
            "model": remote_model,
            "messages": request.messages,
            "stream": false,
        });
        if let Some(tools) = &request.tools {
            body["tools"] = serde_json::to_value(tools).unwrap_or_default();
        }
        if let Some(max_tokens) = request.max_tokens {
            body["max_tokens"] = serde_json::json!(max_tokens);
        }

        let resp = self
            .http
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

    /// Send a streaming request and return (chunks_receiver, final_stats_handle).
    /// Each chunk is yielded as a StreamChunk. On stream end, StreamStats is produced.
    pub async fn call_model_stream(
        &self,
        config: &ModelConfig,
        request: &ChatRequest,
    ) -> Result<StreamHandle, ClientError> {
        let api_key = config.api_key.as_deref().unwrap_or("");
        let remote_model = config.remote_model.as_deref().unwrap_or(&request.model);

        let mut body = serde_json::json!({
            "model": remote_model,
            "messages": request.messages,
            "stream": true,
        });
        if let Some(tools) = &request.tools {
            body["tools"] = serde_json::to_value(tools).unwrap_or_default();
        }
        if let Some(max_tokens) = request.max_tokens {
            body["max_tokens"] = serde_json::json!(max_tokens);
        }

        let resp = self
            .http
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
        Ok(StreamHandle {
            response: Some(resp),
            model: model_name,
            start: std::time::Instant::now(),
            buffer: String::new(),
        })
    }
}

/// A handle to an active streaming response. Call `next_chunk()` repeatedly
/// until it returns `None` (stream ended), then call `stats()` for final stats.
#[derive(Debug)]
pub struct StreamHandle {
    response: Option<reqwest::Response>,
    model: String,
    start: std::time::Instant,
    buffer: String,
}

impl StreamHandle {
    /// Create a new StreamHandle from a response
    pub fn new(response: reqwest::Response, model: String) -> Self {
        Self {
            response: Some(response),
            model,
            start: std::time::Instant::now(),
            buffer: String::new(),
        }
    }

    /// Read the next SSE chunk from the stream. Returns `None` when done.
    pub async fn next_chunk(&mut self) -> Option<Result<StreamChunk, ClientError>> {
        loop {
            // Try to find a complete "data: " line in the buffer
            while let Some(newline_pos) = self.buffer.find('\n') {
                let line = self.buffer[..newline_pos].to_string();
                self.buffer = self.buffer[newline_pos + 1..].to_string();

                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }

                // SSE format: "data: {json}" or "data: [DONE]"
                if let Some(json_str) = line.strip_prefix("data: ") {
                    let json_str = json_str.trim();
                    if json_str == "[DONE]" {
                        self.response = None;
                        return None;
                    }
                    match serde_json::from_str::<StreamSseChunk>(json_str) {
                        Ok(parsed) => {
                            let choice = parsed.choices.first();
                            let delta = choice.and_then(|c| c.delta.as_ref());

                            let delta_content = delta.and_then(|d| d.content.clone());
                            let finish_reason = choice.and_then(|c| c.finish_reason.clone());

                            let tool_call_deltas = delta
                                .and_then(|d| d.tool_calls.as_ref())
                                .map(|tcs| {
                                    tcs.iter()
                                        .map(|tc| ToolCallDelta {
                                            index: tc.index,
                                            id: tc.id.clone(),
                                            name: tc.function.as_ref().and_then(|f| f.name.clone()),
                                            arguments: tc.function.as_ref().and_then(|f| f.arguments.clone()),
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();

                            return Some(Ok(StreamChunk {
                                id: parsed.id,
                                model: parsed.model,
                                delta_content,
                                finish_reason,
                                tool_call_deltas,
                            }));
                        }
                        Err(e) => return Some(Err(ClientError::Parse(e.to_string()))),
                    }
                }
            }

            // Buffer has no complete line; read more data
            let resp = self.response.as_mut()?;
            match resp.chunk().await {
                Ok(Some(chunk)) => {
                    let text = String::from_utf8_lossy(&chunk);
                    self.buffer.push_str(&text);
                }
                Ok(None) => {
                    self.response = None;
                    return None;
                }
                Err(e) => return Some(Err(ClientError::Network(e))),
            }
        }
    }

    /// Consume the entire stream and return the final stats.
    pub async fn into_stats(mut self) -> Result<StreamStats, ClientError> {
        let mut last_model = self.model.clone();
        let usage: Option<Usage> = None;

        while let Some(result) = self.next_chunk().await {
            let chunk = result?;
            if let Some(m) = chunk.model {
                last_model = m;
            }
        }

        // Try to extract usage from the last chunk (some providers include it)
        // In practice, many providers put usage only in the final chunk
        let latency_ms = self.start.elapsed().as_millis() as u64;

        Ok(StreamStats {
            usage,
            latency_ms,
            model: last_model,
        })
    }
}

// Internal type for parsing SSE JSON chunks
#[derive(Debug, Deserialize)]
struct StreamSseChunk {
    id: Option<String>,
    model: Option<String>,
    choices: Vec<StreamChoice>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: Option<StreamDelta>,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<StreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct StreamToolCall {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "type")]
    tool_type: Option<String>,
    #[serde(default)]
    function: Option<StreamFunction>,
}

#[derive(Debug, Deserialize)]
struct StreamFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("upstream returned {0}: {1}")]
    Upstream(u16, String),
    #[error("failed to parse response: {0}")]
    Parse(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{Server, ServerGuard};

    fn test_model_config(url: &str) -> ModelConfig {
        ModelConfig {
            url: url.to_string(),
            api_key_env: None,
            api_key: Some("test-key".to_string()),
            remote_model: Some("test-model".to_string()),
        }
    }

    fn test_request() -> ChatRequest {
        ChatRequest {
            model: "test-model".to_string(),
            messages: vec![Message {
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

    async fn start_mock_server() -> ServerGuard {
        Server::new_async().await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn call_model_success() {
        let mut server = start_mock_server().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .match_header("Authorization", "Bearer test-key")
            .match_header("Content-Type", "application/json")
            .with_body(
                r#"{
                    "id": "chatcmpl-123",
                    "model": "test-model",
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "hi there"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8}
                }"#,
            )
            .create();

        let config = test_model_config(&format!("{}/v1/chat/completions", server.url()));
        let client = ModelClient::new();
        let req = test_request();

        let resp = client.call_model(&config, &req).await.unwrap();

        assert_eq!(resp.id.as_deref(), Some("chatcmpl-123"));
        assert_eq!(resp.model, "test-model");
        assert_eq!(resp.choices.len(), 1);
        assert_eq!(
            resp.choices[0].message.as_ref().unwrap().content,
            Some("hi there".to_string())
        );
        let usage = resp.usage.as_ref().unwrap();
        assert_eq!(usage.prompt_tokens, Some(5));
        assert_eq!(usage.completion_tokens, Some(3));

        mock.assert();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn call_model_upstream_error() {
        let mut server = start_mock_server().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(401)
            .with_body(r#"{"error": "unauthorized"}"#)
            .create();

        let config = test_model_config(&format!("{}/v1/chat/completions", server.url()));
        let client = ModelClient::new();
        let req = test_request();

        let err = client.call_model(&config, &req).await.unwrap_err();
        match err {
            ClientError::Upstream(status, body) => {
                assert_eq!(status, 401);
                assert!(body.contains("unauthorized"));
            }
            other => panic!("expected Upstream error, got: {}", other),
        }

        mock.assert();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn call_model_uses_remote_model_name() {
        let mut server = start_mock_server().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .match_body(mockito::Matcher::JsonString(
                r#"{"model":"remote-v2","messages":[{"role":"user","content":"hello"}],"stream":false}"#
                    .to_string(),
            ))
            .with_body(
                r#"{"model":"remote-v2","choices":[{"message":{"role":"assistant","content":"ok"}}]}"#,
            )
            .create();

        let config = ModelConfig {
            url: format!("{}/v1/chat/completions", server.url()),
            api_key_env: None,
            api_key: Some("key".to_string()),
            remote_model: Some("remote-v2".to_string()),
        };
        let client = ModelClient::new();
        let req = test_request();

        let resp = client.call_model(&config, &req).await.unwrap();
        assert_eq!(resp.model, "remote-v2");

        mock.assert();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn call_model_missing_api_key_sends_empty() {
        let mut server = start_mock_server().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_body(
                r#"{"model":"m","choices":[{"message":{"role":"assistant","content":"ok"}}]}"#,
            )
            .create();

        let config = ModelConfig {
            url: format!("{}/v1/chat/completions", server.url()),
            api_key_env: None,
            api_key: None,
            remote_model: Some("m".to_string()),
        };
        let client = ModelClient::new();
        let req = test_request();

        let resp = client.call_model(&config, &req).await.unwrap();
        assert_eq!(resp.model, "m");

        mock.assert();
    }

    // --- Streaming tests ---

    fn sse_body() -> String {
        vec![
            r#"data: {"id":"chatcmpl-1","model":"test-model","choices":[{"delta":{"role":"assistant"},"finish_reason":null}]}"#,
            "",
            r#"data: {"id":"chatcmpl-1","model":"test-model","choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#,
            "",
            r#"data: {"id":"chatcmpl-1","model":"test-model","choices":[{"delta":{"content":" world"},"finish_reason":null}]}"#,
            "",
            r#"data: {"id":"chatcmpl-1","model":"test-model","choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}"#,
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn call_model_stream_parses_chunks() {
        let mut server = start_mock_server().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .match_header("Accept", "text/event-stream")
            .match_body(mockito::Matcher::JsonString(
                r#"{"model":"test-model","messages":[{"role":"user","content":"hello"}],"stream":true}"#
                    .to_string(),
            ))
            .with_header("Content-Type", "text/event-stream")
            .with_body(sse_body())
            .create();

        let config = test_model_config(&format!("{}/v1/chat/completions", server.url()));
        let client = ModelClient::new();
        let req = test_request();

        let mut handle = client.call_model_stream(&config, &req).await.unwrap();

        let mut contents = Vec::new();
        while let Some(result) = handle.next_chunk().await {
            let chunk = result.unwrap();
            if let Some(c) = chunk.delta_content {
                contents.push(c);
            }
        }

        assert_eq!(contents, vec!["Hello", " world"]);

        mock.assert();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn call_model_stream_done_signal() {
        let mut server = start_mock_server().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_header("Content-Type", "text/event-stream")
            .with_body("data: [DONE]\n\n")
            .create();

        let config = test_model_config(&format!("{}/v1/chat/completions", server.url()));
        let client = ModelClient::new();
        let req = test_request();

        let mut handle = client.call_model_stream(&config, &req).await.unwrap();
        let chunk = handle.next_chunk().await;
        assert!(chunk.is_none()); // [DONE] returns None

        mock.assert();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn call_model_stream_upstream_error() {
        let mut server = start_mock_server().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(500)
            .with_body("server error")
            .create();

        let config = test_model_config(&format!("{}/v1/chat/completions", server.url()));
        let client = ModelClient::new();
        let req = test_request();

        let err = client.call_model_stream(&config, &req).await.unwrap_err();
        assert!(matches!(err, ClientError::Upstream(500, _)));

        mock.assert();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn call_model_stream_invalid_json() {
        let mut server = start_mock_server().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_header("Content-Type", "text/event-stream")
            .with_body("data: {not json}\n\ndata: [DONE]\n\n")
            .create();

        let config = test_model_config(&format!("{}/v1/chat/completions", server.url()));
        let client = ModelClient::new();
        let req = test_request();

        let mut handle = client.call_model_stream(&config, &req).await.unwrap();
        let result = handle.next_chunk().await.unwrap();
        assert!(result.is_err()); // parse error

        mock.assert();
    }
}
