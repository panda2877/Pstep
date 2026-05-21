use pstep_core::client::{ChatRequest, Message};
use pstep_core::fallback::handle_with_fallback;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::routes::AppState;

#[derive(Deserialize)]
struct JsonRpcRequest {
    #[expect(dead_code)]
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

pub async fn run_ws_server(port: u16, state: Arc<AppState>) {
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
                    write.send(WsMessage::Text(response_json)).await?;
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to parse JSON-RPC request");
                }
            }
        }
    }

    Ok(())
}

async fn handle_rpc_request(request: JsonRpcRequest, state: &Arc<AppState>) -> JsonRpcResponse {
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
                    reasoning_content: None,
                }],
                stream: None,
                tools: None,
                max_tokens: None,
            };

            match handle_with_fallback(&state.config, &state.client, &chat_request).await {
                Ok(result) => {
                    let content = result
                        .data
                        .choices
                        .first()
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
