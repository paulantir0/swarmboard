mod api;
mod auth;
mod config;
mod db;
mod domain;
mod email;
mod error;
mod files;
mod hook;
mod mcp;
mod models;
mod state;
mod storage;
mod util;


use axum::{routing::get, Router};
use tower_http::{limit::RequestBodyLimitLayer, trace::TraceLayer};
use tracing_subscriber::EnvFilter;

use crate::{config::Config, state::AppState};

/// Requests are capped a bit above the 10 MB file limit (base64 overhead for MCP uploads).
const BODY_LIMIT: usize = 16 * 1024 * 1024;

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

    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    tracing::info!(bind = %cfg.bind, web = %cfg.web_url, api = %cfg.public_api_url, "swarmboard listening");
    axum::serve(listener, app(state)).await?;
    Ok(())
}
