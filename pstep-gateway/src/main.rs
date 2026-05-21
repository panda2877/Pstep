use axum::extract::{Path, State};
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures::stream::Stream;
use pstep_core::client::{ChatRequest, ModelClient, Message};
use pstep_core::config::{GatewayConfig, ModelConfig};
use pstep_core::fallback::{handle_stream_with_fallback, handle_with_fallback};
use pstep_core::manager::{ModelEntry, ModelStore};
use pstep_core::stats::StatsDb;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMessage;

struct AppState {
    config: GatewayConfig,
    client: ModelClient,
    stats: StatsDb,
    models: ModelStore,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config_dir = std::env::var("PSTEP_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config"));

    let config = GatewayConfig::load(&config_dir).expect("failed to load config");
    let stats_path = std::env::current_dir()
        .expect("failed to get cwd")
        .join("stats.db");
    let stats = StatsDb::open(&stats_path).expect("failed to open stats db");

    let models = ModelStore::load_from_config(&config);

    let state = Arc::new(AppState {
        config,
        client: ModelClient::new(),
        stats,
        models,
    });

    let port = state.config.server.port;
    let ws_port = state.config.server.ws_port;

    let app = Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/messages", post(anthropic_messages))
        .route("/v1/models", get(list_models))
        .route("/v1/models", post(upsert_model))
        .route("/v1/models/{name}", delete(delete_model))
        .route("/health", get(health))
        .route("/stats", get(stats_handler))
        .with_state(state.clone());

    tracing::info!("Pstep gateway listening on port {}", port);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .unwrap();

    // Start WebSocket ACP server
    let ws_handle = tokio::spawn(run_ws_server(ws_port, state));

    // Start HTTP server with graceful shutdown
    let http_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .unwrap();
    });

    // Wait for shutdown signal
    shutdown_signal().await;
    tracing::info!("Shutting down...");

    // Abort WebSocket server
    ws_handle.abort();

    // Wait for HTTP server to finish
    http_handle.abort();

    tracing::info!("Server stopped");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Received shutdown signal");
}

async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatRequest>,
) -> Result<
    axum::response::Response,
    (axum::http::StatusCode, Json<Value>),
> {
    let is_stream = request.stream.unwrap_or(false);

    if is_stream {
        let start = std::time::Instant::now();

        match handle_stream_with_fallback(&state.config, &state.client, &request).await {
            Ok(result) => {
                let model_name = result.actual_model.clone();
                let requested_model = result.requested_model.clone();
                let stats = state.stats.clone();

                let stream = create_sse_stream(result.handle, model_name, requested_model, stats, start);

                Ok(Sse::new(stream).into_response())
            }
            Err(e) => {
                let latency = start.elapsed().as_millis() as u64;
                state.stats.log_usage(
                    "unknown",
                    &request.model,
                    false,
                    0,
                    0,
                    0,
                    latency,
                    Some(&e.to_string()),
                );
                Err((
                    axum::http::StatusCode::BAD_GATEWAY,
                    Json(json!({ "error": e.to_string() })),
                ))
            }
        }
    } else {
        let start = std::time::Instant::now();

        match handle_with_fallback(&state.config, &state.client, &request).await {
            Ok(result) => {
                let latency = start.elapsed().as_millis() as u64;
                let usage = result.data.usage.as_ref();
                state.stats.log_usage(
                    &result.actual_model,
                    &result.requested_model,
                    true,
                    usage.and_then(|u| u.prompt_tokens).unwrap_or(0),
                    usage.and_then(|u| u.completion_tokens).unwrap_or(0),
                    usage.and_then(|u| u.total_tokens).unwrap_or(0),
                    latency,
                    None,
                );

                let mut resp = serde_json::to_value(&result.data).unwrap();
                resp["model"] = json!(result.actual_model);
                Ok(Json(resp).into_response())
            }
            Err(e) => {
                let latency = start.elapsed().as_millis() as u64;
                state.stats.log_usage(
                    "unknown",
                    &request.model,
                    false,
                    0,
                    0,
                    0,
                    latency,
                    Some(&e.to_string()),
                );
                Err((
                    axum::http::StatusCode::BAD_GATEWAY,
                    Json(json!({ "error": e.to_string() })),
                ))
            }
        }
    }
}

// --- Anthropic Messages API ---

#[derive(Deserialize)]
struct AnthropicMessageRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    stream: Option<bool>,
    #[serde(default)]
    system: Option<Value>,
    #[serde(default)]
    tools: Option<Vec<Value>>,
}

#[derive(Deserialize, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Value,
}

fn convert_anthropic_messages_to_openai(msgs: &[AnthropicMessage]) -> Vec<Message> {
    msgs.iter()
        .flat_map(|m| {
            // Handle tool_result messages (role: "user" with tool_result content)
            if m.role == "user" && m.content.is_array() {
                let blocks = m.content.as_array().unwrap();
                let has_tool_results = blocks.iter().any(|b| {
                    b.get("type").and_then(|t| t.as_str()) == Some("tool_result")
                });
                if has_tool_results {
                    return blocks
                        .iter()
                        .filter_map(|b| {
                            if b.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                                let _tool_use_id = b.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("");
                                let content = b.get("content").and_then(|c| {
                                    if c.is_string() {
                                        c.as_str().map(String::from)
                                    } else {
                                        serde_json::to_string(c).ok()
                                    }
                                }).unwrap_or_default();
                                Some(Message {
                                    role: Some("tool".to_string()),
                                    content: Some(content),
                                    tool_calls: None,
                                    tool_call_id: None,
                                })
                            } else {
                                // Regular text block
                                let text = b.get("text").and_then(|t| t.as_str()).unwrap_or("");
                                if text.is_empty() {
                                    None
                                } else {
                                    Some(Message {
                                        role: Some("user".to_string()),
                                        content: Some(text.to_string()),
                                        tool_calls: None,
                                        tool_call_id: None,
                                    })
                                }
                            }
                        })
                        .collect::<Vec<_>>();
                }
            }

            // Handle assistant messages with tool_use content
            if m.role == "assistant" && m.content.is_array() {
                let blocks = m.content.as_array().unwrap();
                let has_tool_use = blocks.iter().any(|b| {
                    b.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                });
                if has_tool_use {
                    let mut result = Vec::new();
                    // Collect text content
                    let text_parts: Vec<String> = blocks
                        .iter()
                        .filter_map(|b| {
                            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                                b.get("text").and_then(|t| t.as_str()).map(String::from)
                            } else {
                                None
                            }
                        })
                        .collect();
                    if !text_parts.is_empty() {
                        result.push(Message {
                            role: Some("assistant".to_string()),
                            content: Some(text_parts.join("")),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                    // Convert tool_use to assistant message with tool_calls
                    let tool_calls: Vec<Value> = blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
                        .filter_map(|b| {
                            let id = b.get("id")?.as_str()?;
                            let name = b.get("name")?.as_str()?;
                            let input = b.get("input").cloned().unwrap_or(json!({}));
                            Some(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(&input).unwrap_or_default()
                                }
                            }))
                        })
                        .collect();
                    if !tool_calls.is_empty() {
                        result.push(Message {
                            role: Some("assistant".to_string()),
                            content: None,
                            tool_calls: Some(tool_calls),
                            tool_call_id: None,
                        });
                    }
                    return result;
                }
            }

            // Regular message
            let content = if m.content.is_string() {
                m.content.as_str().unwrap_or("").to_string()
            } else if let Some(arr) = m.content.as_array() {
                arr.iter()
                    .filter_map(|block| {
                        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                            block.get("text").and_then(|t| t.as_str()).map(String::from)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("")
            } else {
                String::new()
            };
            vec![Message {
                role: Some(m.role.clone()),
                content: Some(content),
                tool_calls: None,
                tool_call_id: None,
            }]
        })
        .collect()
}

/// Convert Anthropic tools format to OpenAI tools format
fn convert_anthropic_tools_to_openai(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|tool| {
            let name = tool.get("name")?.as_str()?;
            let description = tool.get("description").and_then(|d| d.as_str()).unwrap_or("");
            let input_schema = tool.get("input_schema").cloned().unwrap_or(json!({}));

            Some(json!({
                "type": "function",
                "function": {
                    "name": name,
                    "description": description,
                    "parameters": input_schema
                }
            }))
        })
        .collect()
}

/// Convert OpenAI tool_calls to Anthropic tool_use content blocks
fn convert_openai_tool_calls_to_anthropic(tool_calls: &Value) -> Vec<Value> {
    let Some(arr) = tool_calls.as_array() else {
        return vec![];
    };

    arr.iter()
        .filter_map(|tc| {
            let id = tc.get("id").and_then(|v| v.as_str())?;
            let func = tc.get("function")?;
            let name = func.get("name").and_then(|v| v.as_str())?;
            let args_str = func.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
            let input: Value = serde_json::from_str(args_str).unwrap_or(json!({}));

            Some(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            }))
        })
        .collect()
}

async fn anthropic_messages(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AnthropicMessageRequest>,
) -> Result<
    axum::response::Response,
    (axum::http::StatusCode, Json<Value>),
> {
    let is_stream = request.stream.unwrap_or(false);
    tracing::info!(model = %request.model, stream = is_stream, msgs = request.messages.len(), "anthropic_messages request");

    // Build OpenAI messages: prepend system prompt if present
    let mut openai_messages = Vec::new();
    if let Some(system) = &request.system {
        let sys_text = if system.is_string() {
            system.as_str().unwrap_or("").to_string()
        } else if let Some(arr) = system.as_array() {
            arr.iter()
                .filter_map(|block| {
                    if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                        block.get("text").and_then(|t| t.as_str()).map(String::from)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        } else {
            String::new()
        };
        if !sys_text.is_empty() {
            openai_messages.push(Message {
                role: Some("system".to_string()),
                content: Some(sys_text),
                tool_calls: None,
                tool_call_id: None,
            });
        }
    }
    openai_messages.extend(convert_anthropic_messages_to_openai(&request.messages));

    // Convert Anthropic tools to OpenAI format
    let openai_tools = request.tools.as_ref().map(|t| convert_anthropic_tools_to_openai(t));

    let chat_request = ChatRequest {
        model: request.model.clone(),
        messages: openai_messages,
        stream: Some(is_stream),
        tools: openai_tools,
        max_tokens: request.max_tokens,
    };

    let start = std::time::Instant::now();

    if is_stream {
        match handle_stream_with_fallback(&state.config, &state.client, &chat_request).await {
            Ok(result) => {
                let model_name = result.actual_model.clone();
                let requested_model = result.requested_model.clone();
                tracing::info!(model = %model_name, "stream fallback succeeded");
                let stats = state.stats.clone();

                let stream = create_anthropic_sse_stream(
                    result.handle,
                    model_name,
                    requested_model,
                    stats,
                    start,
                );
                Ok(Sse::new(stream).into_response())
            }
            Err(e) => {
                let latency = start.elapsed().as_millis() as u64;
                tracing::error!(error = %e, model = %request.model, latency_ms = latency, "stream fallback failed");
                state.stats.log_usage(
                    "unknown",
                    &request.model,
                    false,
                    0, 0, 0,
                    latency,
                    Some(&e.to_string()),
                );
                Err((
                    axum::http::StatusCode::BAD_GATEWAY,
                    Json(json!({ "error": { "type": "api_error", "message": e.to_string() } })),
                ))
            }
        }
    } else {
        match handle_with_fallback(&state.config, &state.client, &chat_request).await {
            Ok(result) => {
                let latency = start.elapsed().as_millis() as u64;
                let usage = result.data.usage.as_ref();
                tracing::info!(model = %result.actual_model, latency_ms = latency, "non-stream response OK");
                state.stats.log_usage(
                    &result.actual_model,
                    &result.requested_model,
                    true,
                    usage.and_then(|u| u.prompt_tokens).unwrap_or(0),
                    usage.and_then(|u| u.completion_tokens).unwrap_or(0),
                    usage.and_then(|u| u.total_tokens).unwrap_or(0),
                    latency,
                    None,
                );

                let content_text = result.data.choices.first()
                    .and_then(|c| c.message.as_ref())
                    .and_then(|m| m.content.as_deref())
                    .unwrap_or("");

                let input_tokens = usage.and_then(|u| u.prompt_tokens).unwrap_or(0);
                let output_tokens = usage.and_then(|u| u.completion_tokens).unwrap_or(0);

                let msg_id = format!("msg_{}", result.data.id.unwrap_or_default());

                // Build content blocks - include text and tool_use if present
                let mut content_blocks = Vec::new();
                if !content_text.is_empty() {
                    content_blocks.push(json!({"type": "text", "text": content_text}));
                }

                // Check for tool_calls in the response
                let choice = result.data.choices.first();
                let message = choice.and_then(|c| c.message.as_ref());
                let stop_reason = if let Some(msg) = message {
                    if let Some(tool_calls) = &msg.tool_calls {
                        if !tool_calls.is_empty() {
                            let tool_blocks = convert_openai_tool_calls_to_anthropic(&Value::Array(tool_calls.clone()));
                            content_blocks.extend(tool_blocks);
                            "tool_use"
                        } else {
                            "end_turn"
                        }
                    } else {
                        "end_turn"
                    }
                } else {
                    "end_turn"
                };

                Ok(Json(json!({
                    "id": msg_id,
                    "type": "message",
                    "role": "assistant",
                    "content": content_blocks,
                    "model": result.actual_model,
                    "stop_reason": stop_reason,
                    "stop_sequence": null,
                    "usage": {
                        "input_tokens": input_tokens,
                        "output_tokens": output_tokens
                    }
                })).into_response())
            }
            Err(e) => {
                let latency = start.elapsed().as_millis() as u64;
                tracing::error!(error = %e, model = %request.model, latency_ms = latency, "non-stream fallback failed");
                state.stats.log_usage(
                    "unknown",
                    &request.model,
                    false,
                    0, 0, 0,
                    latency,
                    Some(&e.to_string()),
                );
                Err((
                    axum::http::StatusCode::BAD_GATEWAY,
                    Json(json!({ "error": { "type": "api_error", "message": e.to_string() } })),
                ))
            }
        }
    }
}

fn create_anthropic_sse_stream(
    mut handle: pstep_core::client::StreamHandle,
    model_name: String,
    requested_model: String,
    stats: StatsDb,
    start: std::time::Instant,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let msg_id = format!("msg_{}", uuid::Uuid::new_v4());
        let mut event_count: u32 = 0;

        // message_start
        let start_event = json!({
            "type": "message_start",
            "message": {
                "id": msg_id,
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": model_name,
                "stop_reason": null,
                "stop_sequence": null,
                "usage": {"input_tokens": 0, "output_tokens": 0}
            }
        });
        yield Ok(Event::default().event("message_start").data(start_event.to_string()));
        event_count += 1;

        let mut content = String::new();
        let mut current_block_index: usize = 0;
        let mut has_text_block = false;
        let mut tool_calls: std::collections::HashMap<usize, (String, String, String)> = std::collections::HashMap::new(); // index -> (id, name, arguments)
        let mut tool_block_indices: Vec<usize> = Vec::new();

        while let Some(result) = handle.next_chunk().await {
            match result {
                Ok(chunk) => {
                    // Handle text content
                    let delta_content = chunk.delta_content.unwrap_or_default();
                    if !delta_content.is_empty() {
                        if !has_text_block {
                            // Start text block
                            let block_start = json!({
                                "type": "content_block_start",
                                "index": current_block_index,
                                "content_block": {"type": "text", "text": ""}
                            });
                            yield Ok(Event::default().event("content_block_start").data(block_start.to_string()));
                            event_count += 1;
                            has_text_block = true;
                        }
                        content.push_str(&delta_content);
                        let delta_event = json!({
                            "type": "content_block_delta",
                            "index": current_block_index,
                            "delta": {"type": "text_delta", "text": delta_content}
                        });
                        yield Ok(Event::default().event("content_block_delta").data(delta_event.to_string()));
                        event_count += 1;
                    }

                    // Handle tool_call deltas
                    for tc_delta in &chunk.tool_call_deltas {
                        let idx = tc_delta.index;
                        let entry = tool_calls.entry(idx).or_insert_with(|| {
                            (String::new(), String::new(), String::new())
                        });

                        // Update id if provided
                        if let Some(id) = &tc_delta.id {
                            entry.0 = id.clone();
                        }

                        // Update name if provided (start new tool_use block)
                        if let Some(name) = &tc_delta.name {
                            entry.1 = name.clone();
                            let block_idx = if has_text_block {
                                current_block_index + 1 + idx
                            } else {
                                current_block_index + idx
                            };
                            tool_block_indices.push(block_idx);

                            let block_start = json!({
                                "type": "content_block_start",
                                "index": block_idx,
                                "content_block": {
                                    "type": "tool_use",
                                    "id": entry.0,
                                    "name": name,
                                    "input": {}
                                }
                            });
                            yield Ok(Event::default().event("content_block_start").data(block_start.to_string()));
                            event_count += 1;
                        }

                        // Append arguments
                        if let Some(args) = &tc_delta.arguments {
                            entry.2.push_str(args);
                            let block_idx = if has_text_block {
                                current_block_index + 1 + idx
                            } else {
                                current_block_index + idx
                            };
                            let delta_event = json!({
                                "type": "content_block_delta",
                                "index": block_idx,
                                "delta": {"type": "input_json_delta", "partial_json": args}
                            });
                            yield Ok(Event::default().event("content_block_delta").data(delta_event.to_string()));
                            event_count += 1;
                        }
                    }
                }
                Err(e) => {
                    let err_event = json!({
                        "type": "error",
                        "error": {"type": "api_error", "message": e.to_string()}
                    });
                    yield Ok(Event::default().event("error").data(err_event.to_string()));
                    event_count += 1;
                    break;
                }
            }
        }

        // Close text block if it was started
        if has_text_block {
            yield Ok(Event::default().event("content_block_stop").data(json!({"type": "content_block_stop", "index": current_block_index}).to_string()));
            event_count += 1;
        }

        // Close all tool_use blocks
        for block_idx in &tool_block_indices {
            yield Ok(Event::default().event("content_block_stop").data(json!({"type": "content_block_stop", "index": block_idx}).to_string()));
            event_count += 1;
        }

        // Determine stop_reason
        let stop_reason = if tool_calls.is_empty() { "end_turn" } else { "tool_use" };

        // message_delta with stop_reason
        let delta_event = json!({
            "type": "message_delta",
            "delta": {"stop_reason": stop_reason, "stop_sequence": null},
            "usage": {"output_tokens": content.split_whitespace().count() as u32}
        });
        yield Ok(Event::default().event("message_delta").data(delta_event.to_string()));
        event_count += 1;

        // message_stop
        yield Ok(Event::default().event("message_stop").data(json!({"type": "message_stop"}).to_string()));
        event_count += 1;
        tracing::info!(events = event_count, content_len = content.len(), tool_calls = tool_calls.len(), model = %model_name, "stream complete");

        // Log usage
        let latency = start.elapsed().as_millis() as u64;
        stats.log_usage(
            &model_name,
            &requested_model,
            true,
            0, 0, 0,
            latency,
            None,
        );
    }
}

fn create_sse_stream(
    mut handle: pstep_core::client::StreamHandle,
    model_name: String,
    requested_model: String,
    stats: StatsDb,
    start: std::time::Instant,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        while let Some(result) = handle.next_chunk().await {
            match result {
                Ok(chunk) => {
                    let chunk_id = chunk.id.unwrap_or_default();
                    let delta_content = chunk.delta_content.unwrap_or_default();
                    let finish_reason = chunk.finish_reason.clone();

                    let sse_chunk = json!({
                        "id": chunk_id,
                        "object": "chat.completion.chunk",
                        "created": chrono::Utc::now().timestamp(),
                        "model": model_name,
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "content": delta_content
                            },
                            "finish_reason": finish_reason
                        }]
                    });

                    yield Ok(Event::default().data(sse_chunk.to_string()));
                }
                Err(e) => {
                    let error_chunk = json!({
                        "error": { "message": e.to_string(), "type": "server_error" }
                    });
                    yield Ok(Event::default().data(error_chunk.to_string()));
                    break;
                }
            }
        }

        // Send [DONE] marker
        yield Ok(Event::default().data("[DONE]"));

        // Log usage stats
        let latency = start.elapsed().as_millis() as u64;
        stats.log_usage(
            &model_name,
            &requested_model,
            true,
            0, 0, 0, // Usage not available from streaming chunks
            latency,
            None,
        );
    }
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "Pstep Gateway" }))
}

async fn stats_handler(
    State(state): State<Arc<AppState>>,
) -> Json<Value> {
    let records = state.stats.recent(10);
    Json(serde_json::to_value(&records).unwrap())
}

// --- Model Management ---

#[derive(Serialize)]
struct ModelInfo {
    id: String,
    object: &'static str,
    created: i64,
    owned_by: &'static str,
}

#[derive(Deserialize)]
struct CreateModelRequest {
    id: String,
    #[serde(default)]
    api_key_env: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    remote_model: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

async fn list_models(
    State(state): State<Arc<AppState>>,
) -> Json<Value> {
    let names = state.models.list();
    let models: Vec<ModelInfo> = names
        .iter()
        .map(|name| ModelInfo {
            id: name.clone(),
            object: "model",
            created: 1700000000,
            owned_by: "pstep",
        })
        .collect();

    Json(json!({
        "object": "list",
        "data": models
    }))
}

async fn upsert_model(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateModelRequest>,
) -> Result<Json<Value>, (axum::http::StatusCode, Json<Value>)> {
    let url = req.url.unwrap_or_else(|| {
        format!("{}/v1/chat/completions", state.config.server.port)
    });

    let config = ModelConfig {
        url,
        api_key_env: req.api_key_env,
        api_key: req.api_key,
        remote_model: req.remote_model,
    };

    let entry = ModelEntry {
        name: req.id.clone(),
        config,
        fallback_chain: vec![],
    };

    let is_new = state.models.get(&entry.name).is_none();
    state.models.add(entry);

    let status = if is_new { "created" } else { "updated" };

    Ok(Json(json!({
        "id": req.id,
        "object": "model",
        "status": status
    })))
}

async fn delete_model(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, (axum::http::StatusCode, Json<Value>)> {
    match state.models.remove(&name) {
        Some(_) => Ok(Json(json!({
            "id": name,
            "object": "model",
            "deleted": true
        }))),
        None => Err((
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "error": "model not found" })),
        )),
    }
}

// --- WebSocket ACP Server ---

#[derive(Deserialize)]
struct JsonRpcRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

async fn run_ws_server(port: u16, state: Arc<AppState>) {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .expect("failed to bind WebSocket port");

    tracing::info!("ACP WebSocket server listening on port {}", port);

    loop {
        let (stream, addr) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "WebSocket accept failed");
                continue;
            }
        };

        tracing::info!(addr = %addr, "ACP client connected");

        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_ws_connection(stream, state).await {
                tracing::error!(addr = %addr, error = %e, "WebSocket connection error");
            }
            tracing::info!(addr = %addr, "ACP client disconnected");
        });
    }
}

async fn handle_ws_connection(
    stream: tokio::net::TcpStream,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use futures::{SinkExt, StreamExt};

    let ws_stream = tokio_tungstenite::accept_async(stream).await?;
    let (mut write, mut read) = ws_stream.split();

    while let Some(msg) = read.next().await {
        let msg = msg?;
        if msg.is_text() {
            let text = msg.to_text()?;
            match serde_json::from_str::<JsonRpcRequest>(text) {
                Ok(request) => {
                    let response = handle_rpc_request(request, &state).await;
                    let response_json = serde_json::to_string(&response)?;
                    write.send(WsMessage::Text(response_json.into())).await?;
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to parse JSON-RPC request");
                }
            }
        }
    }

    Ok(())
}

async fn handle_rpc_request(
    request: JsonRpcRequest,
    state: &Arc<AppState>,
) -> JsonRpcResponse {
    let jsonrpc = "2.0";

    match request.method.as_str() {
        "initialize" => JsonRpcResponse {
            jsonrpc,
            id: request.id,
            result: Some(json!({ "capabilities": {} })),
            error: None,
        },
        "prompt" => {
            let params = request.params.unwrap_or(json!({}));
            let messages = params.get("messages").and_then(|m| m.as_array());

            let prompt = messages
                .and_then(|msgs| {
                    msgs.iter()
                        .rev()
                        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
                        .and_then(|m| m.get("content").and_then(|c| c.as_str()))
                        .map(|s| s.to_string())
                })
                .unwrap_or_default();

            let chat_request = ChatRequest {
                model: "default".to_string(),
                messages: vec![Message {
                    role: Some("user".to_string()),
                    content: Some(prompt),
                    tool_calls: None,
                    tool_call_id: None,
                }],
                stream: None,
                tools: None,
                max_tokens: None,
            };

            match handle_with_fallback(&state.config, &state.client, &chat_request).await {
                Ok(result) => {
                    let content = result.data.choices.first()
                        .and_then(|c| c.message.as_ref())
                        .and_then(|m| m.content.as_deref())
                        .unwrap_or("");

                    JsonRpcResponse {
                        jsonrpc,
                        id: request.id,
                        result: Some(json!({ "content": content })),
                        error: None,
                    }
                }
                Err(e) => JsonRpcResponse {
                    jsonrpc,
                    id: request.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32000,
                        message: e.to_string(),
                    }),
                },
            }
        }
        _ => JsonRpcResponse {
            jsonrpc,
            id: request.id,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: "Method not found".to_string(),
            }),
        },
    }
}
