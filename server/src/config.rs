//! Runtime configuration, read from environment (and `.env` in dev).

use std::{env, path::PathBuf};

/// Hard cap on humans per team, including the owner.
pub const MAX_TEAM_MEMBERS: i64 = 5;

#[derive(Debug, Clone)]
pub struct Config {
    /// Address to listen on, e.g. `127.0.0.1:8191`.
    pub bind: String,
    /// e.g. `sqlite://data/swarmboard.db`
    pub database_url: String,
    /// Directory for uploaded files (`<data_dir>/files/<id>`).
    pub data_dir: PathBuf,
    /// Public URL of the web app (landing + dashboard), no trailing slash. Magic links and invite links point here.
    pub web_url: String,
    /// Public URL of this API server, no trailing slash. MCP endpoint is `<public_api_url>/mcp`; files are `<public_api_url>/f/…`.
    pub public_api_url: String,
    /// Resend API key. When `None`, emails are logged to stdout instead of sent (local dev).
    pub resend_api_key: Option<String>,
    /// e.g. `swarmboard <login@swarmboard.app>`
    pub email_from: String,
    /// Set the `Secure` attribute on cookies. false for http://localhost dev.
    pub cookie_secure: bool,
}

impl Config {
    pub fn from_env() -> Self {
        let get = |k: &str, d: &str| env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string());
        let trim = |s: String| s.trim_end_matches('/').to_string();
        Config {
            bind: get("BIND", "127.0.0.1:8191"),
            database_url: get("DATABASE_URL", "sqlite://data/swarmboard.db"),
            data_dir: PathBuf::from(get("DATA_DIR", "data")),
            web_url: trim(get("WEB_URL", "http://localhost:5191")),
            public_api_url: trim(get("PUBLIC_API_URL", "http://localhost:8191")),
            resend_api_key: env::var("RESEND_API_KEY").ok().filter(|v| !v.is_empty()),
            email_from: get("EMAIL_FROM", "swarmboard <login@swarmboard.app>"),
            cookie_secure: get("COOKIE_SECURE", "false") == "true",
        }
    }

    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.public_api_url)
    }
}
