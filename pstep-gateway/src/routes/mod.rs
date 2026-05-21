pub mod anthropic;
pub mod chat;
pub mod models;

use axum::routing::{delete, get, post};
use axum::{Json, Router};
use pstep_core::config::GatewayConfig;
use pstep_core::manager::ModelStore;
use pstep_core::stats::StatsDb;
use pstep_core::client::ModelClient;
use serde_json::{Value, json};
use std::sync::Arc;

pub struct AppState {
    pub config: GatewayConfig,
    pub client: ModelClient,
    pub stats: StatsDb,
    pub models: ModelStore,
}

pub fn build_router(state: Arc<AppState>) -> Router {
    // /anthropic/v1/messages — for Claude Code (base URL = .../anthropic)
    let anthropic_routes = Router::new()
        .route("/v1/messages", post(anthropic::anthropic_messages))
        .with_state(state.clone());

    Router::new()
        .route("/v1/chat/completions", post(chat::chat_completions))
        .route("/v1/messages", post(anthropic::anthropic_messages))
        .route("/v1/models", get(models::list_models))
        .route("/v1/models", post(models::upsert_model))
        .route("/v1/models/{name}", delete(models::delete_model))
        .route("/health", get(health))
        .route("/stats", get(stats_handler))
        .nest("/anthropic", anthropic_routes)
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "Pstep Gateway" }))
}

async fn stats_handler(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Json<Value> {
    let records = state.stats.recent(10);
    Json(serde_json::to_value(&records).unwrap())
}
