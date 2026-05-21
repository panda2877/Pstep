use axum::extract::State;
use axum::response::IntoResponse;
use axum::response::sse::{Event, Sse};
use axum::Json;
use futures::stream::Stream;
use pstep_core::client::ChatRequest;
use pstep_core::fallback::{handle_stream_with_fallback, handle_with_fallback};
use serde_json::{Value, json};
use std::convert::Infallible;
use std::sync::Arc;

use super::AppState;

pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatRequest>,
) -> Result<axum::response::Response, (axum::http::StatusCode, Json<Value>)> {
    // Force non-streaming — Claude Code doesn't handle SSE
    let is_stream = false;
    let mut request = request;
    request.stream = Some(false);

    if is_stream {
        let start = std::time::Instant::now();

        match handle_stream_with_fallback(&state.config, &state.client, &request).await {
            Ok(result) => {
                let model_name = result.actual_model.clone();
                let requested_model = result.requested_model.clone();
                let stats = state.stats.clone();

                let stream =
                    create_sse_stream(result.handle, model_name, requested_model, stats, start);

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

pub fn create_sse_stream(
    mut handle: pstep_core::client::StreamHandle,
    model_name: String,
    requested_model: String,
    stats: pstep_core::stats::StatsDb,
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
