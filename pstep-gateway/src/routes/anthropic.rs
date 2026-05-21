use axum::extract::State;
use axum::response::IntoResponse;
use axum::response::sse::{Event, Sse};
use axum::Json;
use futures::stream::Stream;
use pstep_core::client::{ChatRequest, Message};
use pstep_core::fallback::{handle_stream_with_fallback, handle_with_fallback};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::convert::Infallible;
use std::sync::Arc;

use super::AppState;

#[derive(Deserialize)]
pub struct AnthropicMessageRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    #[expect(dead_code)]
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

pub async fn anthropic_messages(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AnthropicMessageRequest>,
) -> Result<axum::response::Response, (axum::http::StatusCode, Json<Value>)> {
    // Force non-streaming — Claude Code doesn't handle SSE
    let is_stream = false;
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
                reasoning_content: None,
            });
        }
    }
    openai_messages.extend(convert_anthropic_messages_to_openai(&request.messages));

    // Convert Anthropic tools to OpenAI format
    let openai_tools = request
        .tools
        .as_ref()
        .map(|t| convert_anthropic_tools_to_openai(t));

    let chat_request = ChatRequest {
        model: request.model.clone(),
        messages: openai_messages,
        stream: Some(false),
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
                    0,
                    0,
                    0,
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

                let content_text = result
                    .data
                    .choices
                    .first()
                    .and_then(|c| c.message.as_ref())
                    .and_then(|m| m.content.as_deref())
                    .unwrap_or("");

                let input_tokens = usage.and_then(|u| u.prompt_tokens).unwrap_or(0);
                let output_tokens = usage.and_then(|u| u.completion_tokens).unwrap_or(0);

                // Use upstream id directly (no msg_ prefix)
                let msg_id = result.data.id.unwrap_or_default();

                // Build content blocks - include thinking, text, and tool_use
                let mut content_blocks = Vec::new();

                // Extract reasoning_content → thinking block
                let choice = result.data.choices.first();
                let message = choice.and_then(|c| c.message.as_ref());
                if let Some(msg) = message {
                    if let Some(ref reasoning) = msg.reasoning_content {
                        if !reasoning.is_empty() {
                            content_blocks.push(json!({
                                "type": "thinking",
                                "thinking": reasoning,
                                "signature": ""
                            }));
                        }
                    }
                }

                if !content_text.is_empty() {
                    content_blocks.push(json!({"type": "text", "text": content_text}));
                }

                // Check for tool_calls in the response
                let stop_reason = if let Some(msg) = message {
                    if let Some(tool_calls) = &msg.tool_calls {
                        if !tool_calls.is_empty() {
                            let tool_blocks = convert_openai_tool_calls_to_anthropic(
                                &Value::Array(tool_calls.clone()),
                            );
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

                // Build usage with cache info
                let mut usage_json = json!({
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens
                });
                if let Some(ref u) = result.data.usage {
                    if let Some(ref details) = u.prompt_tokens_details {
                        if let Some(cached) = details.cached_tokens {
                            usage_json["cache_read_input_tokens"] = json!(cached);
                        }
                    }
                }

                Ok(Json(json!({
                    "id": msg_id,
                    "type": "message",
                    "role": "assistant",
                    "content": content_blocks,
                    "model": result.actual_model,
                    "stop_reason": stop_reason,
                    "stop_sequence": null,
                    "usage": usage_json
                }))
                .into_response())
            }
            Err(e) => {
                let latency = start.elapsed().as_millis() as u64;
                tracing::error!(error = %e, model = %request.model, latency_ms = latency, "non-stream fallback failed");
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
                    Json(json!({ "error": { "type": "api_error", "message": e.to_string() } })),
                ))
            }
        }
    }
}

fn convert_anthropic_messages_to_openai(msgs: &[AnthropicMessage]) -> Vec<Message> {
    msgs.iter()
        .flat_map(|m| {
            // Handle tool_result messages (role: "user" with tool_result content)
            if m.role == "user" && m.content.is_array() {
                let blocks = m.content.as_array().unwrap();
                let has_tool_results = blocks
                    .iter()
                    .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"));
                if has_tool_results {
                    return blocks
                        .iter()
                        .filter_map(|b| {
                            if b.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                                let tool_use_id = b
                                    .get("tool_use_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let content = b
                                    .get("content")
                                    .and_then(|c| {
                                        if c.is_string() {
                                            c.as_str().map(String::from)
                                        } else {
                                            serde_json::to_string(c).ok()
                                        }
                                    })
                                    .unwrap_or_default();
                                Some(Message {
                                    role: Some("tool".to_string()),
                                    content: Some(content),
                                    tool_calls: None,
                                    tool_call_id: Some(tool_use_id),
                                    reasoning_content: None,
                                })
                            } else {
                                let text = b.get("text").and_then(|t| t.as_str()).unwrap_or("");
                                if text.is_empty() {
                                    None
                                } else {
                                    Some(Message {
                                        role: Some("user".to_string()),
                                        content: Some(text.to_string()),
                                        tool_calls: None,
                                        tool_call_id: None,
                                        reasoning_content: None,
                                    })
                                }
                            }
                        })
                        .collect::<Vec<_>>();
                }
            }

            // Handle assistant messages (may contain thinking, text, tool_use blocks)
            if m.role == "assistant" && m.content.is_array() {
                let blocks = m.content.as_array().unwrap();

                // Extract thinking content for reasoning_content
                let thinking_parts: Vec<String> = blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("thinking"))
                    .filter_map(|b| b.get("thinking").and_then(|t| t.as_str()).map(String::from))
                    .collect();
                let reasoning = if thinking_parts.is_empty() {
                    None
                } else {
                    Some(thinking_parts.join("\n"))
                };

                let has_tool_use = blocks
                    .iter()
                    .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"));

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
                            reasoning_content: reasoning.clone(),
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
                            reasoning_content: reasoning,
                        });
                    }
                    return result;
                }

                // Assistant message with only text (and possibly thinking)
                let content = blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()).map(String::from))
                    .collect::<Vec<_>>()
                    .join("");

                return vec![Message {
                    role: Some("assistant".to_string()),
                    content: if content.is_empty() { None } else { Some(content) },
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content: reasoning,
                }];
            }

            // Regular message (string content)
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
                reasoning_content: None,
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
            let description = tool
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("");
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
            let args_str = func
                .get("arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
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

fn create_anthropic_sse_stream(
    mut handle: pstep_core::client::StreamHandle,
    model_name: String,
    requested_model: String,
    stats: pstep_core::stats::StatsDb,
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
        let current_block_index: usize = 0;
        let mut has_text_block = false;
        let mut tool_calls: std::collections::HashMap<usize, (String, String, String)> = std::collections::HashMap::new();
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
