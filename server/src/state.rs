use std::sync::Arc;

use sqlx::SqlitePool;

use crate::{config::Config, storage::Storage};

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub cfg: Arc<Config>,
    pub http: reqwest::Client,
    pub storage: Storage,
}

impl AppState {
    pub fn new(db: SqlitePool, cfg: Config) -> anyhow::Result<Self> {
        let storage = Storage::from_config(&cfg)?;
        Ok(AppState { db, cfg: Arc::new(cfg), http: reqwest::Client::new(), storage })
    }
}
