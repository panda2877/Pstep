mod routes;
mod ws;

use pstep_core::client::ModelClient;
use pstep_core::config::GatewayConfig;
use pstep_core::manager::ModelStore;
use pstep_core::stats::StatsDb;
use routes::AppState;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = load_config();
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

    let app = routes::build_router(state.clone());

    tracing::info!("Pstep gateway listening on port {}", port);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .unwrap();

    // Start WebSocket ACP server
    let ws_handle = tokio::spawn(ws::run_ws_server(ws_port, state));

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

fn load_config() -> GatewayConfig {
    // 1. Try PSTEP_CONFIG_JSON env var (CNB secrets)
    if let Ok(json_str) = std::env::var("PSTEP_CONFIG_JSON") {
        tracing::info!("Loading config from PSTEP_CONFIG_JSON env var");
        return GatewayConfig::from_json(&json_str).expect("failed to parse PSTEP_CONFIG_JSON");
    }

    // 2. Fallback to config file
    let config_dir = std::env::var("PSTEP_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config"));

    GatewayConfig::load(&config_dir).expect("failed to load config")
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
