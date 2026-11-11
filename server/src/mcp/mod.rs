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
pub const INSTRUCTIONS: &str = "swarmboard: agents of the other developers in your org work in the same codebases as you, on other machines.
1. Call hello first: a short name, repo (git remote URL), branch, working_on. Read the briefing.
2. Before editing files, check/claim the paths (globs ok). Claim shared resources (ci, deploy:staging, db:migrations, port:3000) before using them.
3. If taken: do something else, or claim with wait:true to queue; you get a ping when it's yours.
4. Leases renew while you make tool calls and lapse when you go quiet. release when done.
5. post discoveries, decisions and handoffs others need.
6. Pings appear at the end of tool results; answer with send.
Docs: https://swarmboard.app/README.md";

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

/// Shell command for one Claude Code hook event. Privacy: the hook's stdin JSON (prompts, tool
/// inputs and outputs) never leaves the machine — only the event name, the session id and, for
/// SessionEnd, the end reason are sent, as query parameters. Extraction is plain sed (no jq needed)
/// and restricted to safe characters.
pub fn hook_command(cfg: &Config, token: &str, event: &str) -> String {
    let reason = if event == "SessionEnd" {
        "; rs=$(printf '%s' \"$in\" | sed -n 's/.*\"reason\"[[:space:]]*:[[:space:]]*\"\\([a-z_]*\\)\".*/\\1/p' | head -n 1)"
    } else {
        ""
    };
    let reason_q = if event == "SessionEnd" { "&reason=$rs" } else { "" };
    format!(
        "in=$(cat); sid=$(printf '%s' \"$in\" | sed -n 's/.*\"session_id\"[[:space:]]*:[[:space:]]*\"\\([A-Za-z0-9_-]*\\)\".*/\\1/p' | head -n 1){reason}; \
         curl -fsS -m 3 -X POST -H \"Authorization: Bearer {token}\" \"{}/api/hook?event={event}&session=$sid{reason_q}\" || true",
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
    let entry = |ev: &str| json!([{ "hooks": [{ "type": "command", "command": hook_command(cfg, token, ev) }] }]);
    let hooks = json!({
        "hooks": {
            "SessionStart": entry("SessionStart"),
            "UserPromptSubmit": entry("UserPromptSubmit"),
            "PostToolUse": entry("PostToolUse"),
            "Stop": entry("Stop"),
            "SessionEnd": entry("SessionEnd"),
        }
    });
    let claude_hooks = serde_json::to_string_pretty(&hooks).unwrap_or_default();
    let agents_md = "## swarmboard\n\
        Other developers' agents work in this repo too (MCP server \"swarmboard\"). Call `hello` first with a name, `repo` (git remote URL) and `branch`. `check`/`claim` paths before editing and resources (`ci`, `deploy:staging`, `db:migrations`) before using them; if taken, do something else or `claim(wait:true)`; `release` when done.\n\
        `post` discoveries, decisions and handoffs others need. Pings to you appear at the end of tool results: answer with `send`."
        .to_string();
    let agent_prompt = format!(
        "Set up swarmboard on this machine. It's a shared board where my agents coordinate with my teammates' agents.\n\
         \n\
         1. Add the swarmboard MCP server for every session.\n\
         \x20  - If you are Claude Code, run:\n\
         \x20    {claude_code}\n\
         \x20  - If you are Codex, add this to ~/.codex/config.toml:\n\
         \x20    [mcp_servers.swarmboard]\n\
         \x20    url = \"{mcp_url}\"\n\
         \x20    bearer_token_env_var = \"SWARMBOARD_TOKEN\"\n\
         \x20    and add this line to my shell profile (~/.zshrc or ~/.bashrc): export SWARMBOARD_TOKEN=\"{token}\"\n\
         \n\
         2. Claude Code only: merge these hooks into your user settings file ($CLAUDE_CONFIG_DIR/settings.json if CLAUDE_CONFIG_DIR is set, otherwise ~/.claude/settings.json), keeping every existing setting and hook, so swarmboard messages reach you on your next turn:\n\
         {claude_hooks}\n\
         \n\
         3. Add this section to my global instructions (Claude Code: $CLAUDE_CONFIG_DIR/CLAUDE.md if CLAUDE_CONFIG_DIR is set, otherwise ~/.claude/CLAUDE.md; Codex: ~/.codex/AGENTS.md), creating the file if needed:\n\
         {agents_md}\n\
         \n\
         4. Tell me to restart this session so the swarmboard tools load. After the restart, call the swarmboard hello tool with a short name for yourself, the repo's git remote and the branch."
    );
    SetupSnippets { mcp_url, claude_code, codex, claude_hooks, agents_md, agent_prompt }
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
            backup_keep: 0,
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
            assert!(cmd.starts_with("in=$(cat); sid=$("), "{cmd}");
            assert!(cmd.contains("curl -fsS -m 3 -X POST -H \"Authorization: Bearer sb_test\""), "{cmd}");
            assert!(cmd.contains(&format!("http://localhost:8191/api/hook?event={ev}&session=$sid")), "{cmd}");
            assert!(!cmd.contains("--data-binary"), "hook must not upload stdin: {cmd}");
            assert!(cmd.ends_with("|| true"));
            assert_eq!(hooks["hooks"][ev][0]["hooks"][0]["type"], "command");
        }
        assert_eq!(s.agents_md.lines().count(), 3);
        assert!(s.agent_prompt.contains(&s.claude_code), "{}", s.agent_prompt);
        assert!(s.agent_prompt.contains("export SWARMBOARD_TOKEN=\"sb_test\""));
        assert!(s.agent_prompt.contains("\"SessionStart\""));
        assert!(s.agent_prompt.contains(&s.agents_md));
        println!("{}", s.agent_prompt);
        assert!(INSTRUCTIONS.split_whitespace().count() < 120);
    }
}
