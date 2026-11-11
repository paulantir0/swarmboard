mod api;
mod auth;
mod backup;
mod config;
mod db;
mod domain;
mod email;
mod error;
mod files;
mod hook;
mod mcp;
mod models;
mod notify;
mod state;
mod storage;
mod util;


use std::time::Duration;

use axum::{http::StatusCode, routing::get, Router};
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer, trace::TraceLayer};
use tracing_subscriber::EnvFilter;

use crate::{config::Config, state::AppState};

/// Requests are capped a bit above the 10 MB file limit (base64 overhead for MCP uploads).
const BODY_LIMIT: usize = 16 * 1024 * 1024;
/// No handler waits on anything long (remote fetches stop at 20 s); this bounds slow uploads and stuck requests.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(auth::router())
        .merge(api::router())
        .merge(mcp::router())
        .merge(hook::router())
        .merge(files::router())
        .layer(axum::extract::DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT))
        .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, REQUEST_TIMEOUT))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=info,sqlx=warn".into()))
        .init();

    let cfg = Config::from_env();
    let db = db::connect(&cfg).await?;
    let state = AppState::new(db, cfg.clone())?;
    if state.storage.is_s3() {
        tracing::info!(bucket = ?cfg.s3.as_ref().map(|s| &s.bucket), "file uploads go to S3");
    }

    backup::spawn(state.db.clone(), cfg.data_dir.join("backups"), cfg.backup_keep);

    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    tracing::info!(bind = %cfg.bind, web = %cfg.web_url, api = %cfg.public_api_url, "swarmboard listening");
    axum::serve(listener, app(state.clone())).with_graceful_shutdown(shutdown_signal()).await?;
    // Checkpoint the WAL so the database file alone is complete after a stop.
    state.db.close().await;
    tracing::info!("swarmboard stopped");
    Ok(())
}

/// Ctrl-C locally, SIGTERM from `docker stop`: stop accepting, finish in-flight requests, exit.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
    tracing::info!("shutting down");
}
