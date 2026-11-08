//! Remote MCP server (Streamable HTTP, JSON responses) at /mcp. OWNER: MCP subagent.
//!
//! - `transport`: JSON-RPC over POST /mcp, sessions (Mcp-Session-Id), auth, version negotiation.
//! - `tools`: tool definitions + dispatch onto `domain::collab`.
//! - `format`: compact text renderings shared with the Claude Code hook endpoint.

pub mod format;
pub mod tools;
pub mod transport;

use axum::{
    http::{header, HeaderName, Method},
    routing::post,
    Router,
};
use serde_json::json;
use tower_http::cors::{Any, CorsLayer};

use crate::{config::Config, models::SetupSnippets, state::AppState};

/// Sent in `initialize`; clients surface it to the model. Keep it short.
pub const INSTRUCTIONS: &str = "swarmboard: other developers' agents work in the same codebases as you, on other machines.
1. Call hello first: a short name, repo (git remote URL), branch, working_on. Read the briefing.
2. Before editing files, check/claim the paths (globs ok). Claim shared resources (ci, deploy:staging, db:migrations, port:3000) before using them.
3. If taken: do something else, or claim with wait:true to queue; you get a ping when it's yours.
4. Leases renew while you make tool calls and lapse when you go quiet. release when done.
5. post discoveries, decisions and handoffs others need.
6. Pings appear at the end of tool results; answer with send.";

pub fn router() -> Router<AppState> {
    // Bearer-token auth (no cookies), so any origin may call it (e.g. browser-based MCP inspectors).
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::POST, Method::GET, Method::DELETE, Method::OPTIONS])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            HeaderName::from_static("mcp-session-id"),
            HeaderName::from_static("mcp-protocol-version"),
            HeaderName::from_static("last-event-id"),
        ])
        .expose_headers([HeaderName::from_static("mcp-session-id")]);
    Router::new()
        .route("/mcp", post(transport::post_mcp).get(transport::get_mcp).delete(transport::delete_mcp))
        .layer(cors)
}

/// Shell command used by every Claude Code hook: POSTs the hook's stdin JSON to /api/hook.
pub fn hook_command(cfg: &Config, token: &str) -> String {
    format!(
        "curl -fsS -m 3 -X POST -H \"Authorization: Bearer {token}\" -H \"Content-Type: application/json\" --data-binary @- {}/api/hook || true",
        cfg.public_api_url
    )
}

/// Copy-paste setup for a token. Called by the REST API when a token is created.
pub fn setup_snippets(cfg: &Config, token: &str) -> SetupSnippets {
    let mcp_url = cfg.mcp_url();
    let claude_code =
        format!("claude mcp add --scope user --transport http swarmboard {mcp_url} --header \"Authorization: Bearer {token}\"");
    let codex = format!(
        "# ~/.codex/config.toml\n\
         [mcp_servers.swarmboard]\n\
         url = \"{mcp_url}\"\n\
         bearer_token_env_var = \"SWARMBOARD_TOKEN\"\n\
         \n\
         # ~/.zshrc or ~/.bashrc (Codex reads the token from this variable)\n\
         export SWARMBOARD_TOKEN=\"{token}\"\n\
         \n\
         # or, instead of editing config.toml:\n\
         # codex mcp add swarmboard --url {mcp_url} --bearer-token-env-var SWARMBOARD_TOKEN"
    );
    let entry = json!([{ "hooks": [{ "type": "command", "command": hook_command(cfg, token) }] }]);
    let hooks = json!({
        "hooks": {
            "SessionStart": entry,
            "UserPromptSubmit": entry,
            "PostToolUse": entry,
            "Stop": entry,
            "SessionEnd": entry,
        }
    });
    let claude_hooks = serde_json::to_string_pretty(&hooks).unwrap_or_default();
    let agents_md = "## swarmboard\n\
        Other developers' agents work in this repo too (MCP server \"swarmboard\"). Call `hello` first with a name, `repo` (git remote URL) and `branch`. `check`/`claim` paths before editing and resources (`ci`, `deploy:staging`, `db:migrations`) before using them; if taken, do something else or `claim(wait:true)`; `release` when done.\n\
        `post` discoveries, decisions and handoffs others need. Pings to you appear at the end of tool results: answer with `send`."
        .to_string();
    SetupSnippets { mcp_url, claude_code, codex, claude_hooks, agents_md }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config {
            bind: "127.0.0.1:0".into(),
            database_url: "sqlite::memory:".into(),
            data_dir: "data".into(),
            web_url: "http://localhost:5191".into(),
            public_api_url: "http://localhost:8191".into(),
            resend_api_key: None,
            email_from: "x".into(),
            cookie_secure: false,
            s3: None,
        }
    }

    #[test]
    fn snippets() {
        let s = setup_snippets(&cfg(), "sb_test");
        assert_eq!(s.mcp_url, "http://localhost:8191/mcp");
        assert_eq!(
            s.claude_code,
            "claude mcp add --scope user --transport http swarmboard http://localhost:8191/mcp --header \"Authorization: Bearer sb_test\""
        );
        assert!(s.codex.contains("[mcp_servers.swarmboard]\nurl = \"http://localhost:8191/mcp\"\nbearer_token_env_var = \"SWARMBOARD_TOKEN\""));
        assert!(s.codex.contains("export SWARMBOARD_TOKEN=\"sb_test\""));
        let hooks: serde_json::Value = serde_json::from_str(&s.claude_hooks).unwrap();
        for ev in ["SessionStart", "UserPromptSubmit", "PostToolUse", "Stop", "SessionEnd"] {
            let cmd = hooks["hooks"][ev][0]["hooks"][0]["command"].as_str().unwrap();
            assert!(cmd.starts_with("curl -fsS -m 3 -X POST -H \"Authorization: Bearer sb_test\""), "{cmd}");
            assert!(cmd.ends_with("http://localhost:8191/api/hook || true"));
            assert_eq!(hooks["hooks"][ev][0]["hooks"][0]["type"], "command");
        }
        assert_eq!(s.agents_md.lines().count(), 3);
        assert!(INSTRUCTIONS.split_whitespace().count() < 120);
    }
}
