//! Shared types. These are the JSON shapes the dashboard consumes and the values the
//! collab domain layer (domain::collab) returns to both the REST API and the MCP tools.
//! Keep changes additive: other modules depend on these fields.

use serde::{Deserialize, Serialize};

pub const POST_KINDS: &[&str] = &["task", "note", "discovery", "question", "decision", "blocker", "handoff"];
pub const POST_STATUSES: &[&str] = &["open", "in_progress", "blocked", "done", "closed"];
/// An agent counts as active if seen within this window.
pub const AGENT_ACTIVE_MS: i64 = 15 * 60 * 1000;

/// Who performs an action. Humans acting in the dashboard have `agent_id = None`.
#[derive(Debug, Clone)]
pub struct Actor {
    pub team_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
}

/// Rendered identity: everyone can always see whose agent did something.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Author {
    pub user_id: String,
    /// The human's handle, e.g. "paul".
    pub handle: String,
    pub agent_id: Option<String>,
    /// e.g. "ui-fixer"
    pub agent_name: Option<String>,
    /// "paul/ui-fixer" for agents, "paul" for the human.
    pub display: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Board {
    pub id: String,
    pub key: String,
    pub name: String,
    pub description: String,
    pub created_at: i64,
    pub open_count: i64,
    pub total_count: i64,
    pub last_activity_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Post {
    pub id: String,
    /// Per-team sequence number. Posts are addressed by number everywhere (`#12`).
    pub number: i64,
    pub board_key: String,
    pub kind: String,
    pub status: String,
    pub priority: i64,
    pub title: String,
    /// Full markdown in `get_post`; truncated (~240 chars) in `list_posts`.
    pub body: String,
    pub author: Author,
    pub claimed_by: Option<Author>,
    pub claimed_at: Option<i64>,
    pub comment_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Comment {
    pub id: String,
    pub post_number: i64,
    pub body: String,
    pub author: Author,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PostDetail {
    pub post: Post,
    pub comments: Vec<Comment>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PostFilter {
    /// Board key.
    pub board: Option<String>,
    /// Comma-separated statuses, or "active" (= open,in_progress,blocked). None = all.
    pub status: Option<String>,
    pub kind: Option<String>,
    /// Agent display ("paul/ui-fixer"), agent name, or human handle.
    pub claimed_by: Option<String>,
    /// Agent display or human handle.
    pub author: Option<String>,
    /// Case-insensitive substring search over title/body.
    pub q: Option<String>,
    /// Default 50, max 200.
    pub limit: Option<i64>,
    /// Pagination: only posts with number < this.
    pub before: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewPost {
    /// Board key; must exist.
    pub board: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// One of POST_KINDS; default "note".
    pub kind: Option<String>,
    /// One of POST_STATUSES; default "open".
    pub status: Option<String>,
    /// 0..=4, default 0.
    pub priority: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PostPatch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub status: Option<String>,
    pub kind: Option<String>,
    pub priority: Option<i64>,
    /// Move to another board (key).
    pub board: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ping {
    pub id: String,
    pub from: Author,
    /// As the sender wrote it: "paul/ui-fixer", "@paul", "paul/*", "@all".
    pub target: String,
    pub body: String,
    pub post_number: Option<i64>,
    pub created_at: i64,
    pub delivered_at: Option<i64>,
    pub read_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PingSent {
    pub ping_id: String,
    /// Display names of everyone it was delivered to (agents and/or humans).
    pub recipients: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    /// "paul/ui-fixer"
    pub display: String,
    pub user_id: String,
    pub owner_handle: String,
    pub client: Option<String>,
    pub working_on: Option<String>,
    pub created_at: i64,
    pub last_seen_at: i64,
    pub active: bool,
    /// Numbers of posts this agent currently has claimed (status in_progress/blocked).
    pub claimed: Vec<i64>,
    /// Normalized repo ("github.com/acme/app") and branch the agent said it works in.
    pub repo: Option<String>,
    pub branch: Option<String>,
    /// Scopes this agent currently holds leases on ("server/src/auth/**", "ci", "#12").
    pub leases: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileInfo {
    pub id: String,
    pub filename: String,
    pub content_type: String,
    pub size: i64,
    /// Absolute public URL: `<public_api_url>/f/<id>/<filename>`.
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Activity {
    pub id: i64,
    pub at: i64,
    /// See `events.kind` in the migration.
    pub kind: String,
    pub actor: Option<Author>,
    pub post_number: Option<i64>,
    pub post_title: Option<String>,
    pub data: serde_json::Value,
}

/// Result of authenticating an MCP/API bearer token.
#[derive(Debug, Clone)]
pub struct TokenAuth {
    pub token_id: String,
    pub user_id: String,
    pub team_id: String,
    pub handle: String,
}

/// Copy-paste setup for a freshly created token (rendered by the dashboard "Connect" view).
#[derive(Debug, Clone, Serialize)]
pub struct SetupSnippets {
    pub mcp_url: String,
    /// One-line `claude mcp add …` command.
    pub claude_code: String,
    /// `codex mcp add …` command and/or ~/.codex/config.toml block.
    pub codex: String,
    /// JSON for ~/.claude/settings.json "hooks" (ping delivery on next turn).
    pub claude_hooks: String,
    /// Short paragraph to paste into CLAUDE.md / AGENTS.md.
    pub agents_md: String,
}

/// An exclusive, auto-renewing lease on a path/glob, shared resource or task.
#[derive(Debug, Clone, Serialize)]
pub struct Lease {
    pub scope: String,
    /// "path" | "resource" | "task"
    pub kind: String,
    /// Normalized repo for path leases ("" = team-wide).
    pub repo: String,
    pub holder: Author,
    pub note: String,
    pub expires_at: i64,
    pub created_at: i64,
}

/// Someone queued (claim with wait:true) for a scope.
#[derive(Debug, Clone, Serialize)]
pub struct LeaseWait {
    pub scope: String,
    pub kind: String,
    pub repo: String,
    pub waiter: Author,
    /// 1 = next in line for this scope.
    pub position: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaimsView {
    pub leases: Vec<Lease>,
    pub queue: Vec<LeaseWait>,
}
