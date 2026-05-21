use axum::extract::{Path, State};
use axum::Json;
use pstep_core::config::ModelConfig;
use pstep_core::manager::ModelEntry;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

use super::AppState;

#[derive(Serialize)]
struct ModelInfo {
    id: String,
    object: &'static str,
    created: i64,
    owned_by: &'static str,
}

#[derive(Deserialize)]
pub struct CreateModelRequest {
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

pub async fn list_models(State(state): State<Arc<AppState>>) -> Json<Value> {
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

pub async fn upsert_model(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateModelRequest>,
) -> Result<Json<Value>, (axum::http::StatusCode, Json<Value>)> {
    let url = req
        .url
        .unwrap_or_else(|| format!("{}/v1/chat/completions", state.config.server.port));

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

pub async fn delete_model(
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
