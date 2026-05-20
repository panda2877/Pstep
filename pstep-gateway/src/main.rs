use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use pstep_core::client::{ChatRequest, ModelClient};
use pstep_core::config::GatewayConfig;
use pstep_core::fallback::handle_with_fallback;
use pstep_core::stats::StatsDb;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

struct AppState {
    config: GatewayConfig,
    client: ModelClient,
    stats: StatsDb,
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

    let state = Arc::new(AppState {
        config,
        client: ModelClient::new(),
        stats,
    });

    let port = state.config.server.port;
    let _ws_port = state.config.server.ws_port;

    let app = Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/health", get(health))
        .route("/stats", get(stats_handler))
        .with_state(state);

    tracing::info!("Pstep gateway listening on port {}", port);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatRequest>,
) -> Result<Json<Value>, (axum::http::StatusCode, Json<Value>)> {
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
            // Ensure model field reflects actual model used
            resp["model"] = json!(result.actual_model);
            Ok(Json(resp))
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

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "Pstep Gateway" }))
}

async fn stats_handler(
    State(state): State<Arc<AppState>>,
) -> Json<Value> {
    let records = state.stats.recent(10);
    Json(serde_json::to_value(&records).unwrap())
}
