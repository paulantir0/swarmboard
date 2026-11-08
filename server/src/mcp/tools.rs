//! MCP tools: definitions (JSON Schema) and dispatch onto `domain::collab`.
//! Results are compact text; every result except `inbox` gets undelivered pings appended.

use base64::Engine;
use serde_json::{json, Map, Value};

use super::format;
use crate::{
    domain::collab::{
        self,
        leases::{self, ClaimOutcome, LeaseRow, ScopeKind},
        AgentRef, HelloOutcome, McpSession,
    },
    error::AppError,
    models::{Actor, NewPost, Post, PostFilter, PostPatch, TokenAuth},
    state::AppState,
    util::now_ms,
};

pub struct ToolCtx<'a> {
    pub state: &'a AppState,
    pub auth: &'a TokenAuth,
    pub session: &'a mut McpSession,
}

/// A JSON-RPC-level failure of `tools/call` (unknown tool or malformed params).
#[derive(Debug)]
pub struct CallError {
    pub code: i64,
    pub message: String,
}

pub const TOOL_NAMES: &[&str] = &[
    "hello", "status", "check", "claim", "release", "claims", "post", "feed", "read", "reply", "update", "send", "inbox",
    "agents", "topics", "upload",
];

const KINDS: &[&str] = &["task", "discovery", "decision", "question", "blocker", "handoff", "note"];
const STATUSES: &[&str] = &["open", "in_progress", "blocked", "done", "closed"];

fn tool(name: &str, title: &str, description: &str, props: Value, required: &[&str], read_only: bool) -> Value {
    let mut annotations = json!({ "title": title, "readOnlyHint": read_only, "openWorldHint": false });
    if !read_only {
        annotations["destructiveHint"] = json!(false);
    }
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": props,
            "required": required,
            "additionalProperties": false,
        },
        "annotations": annotations,
    })
}

pub fn definitions() -> Value {
    let number = |d: &str| json!({ "type": "integer", "minimum": 1, "description": d });
    let scope_list = |d: &str| json!({ "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": d });
    json!([
        tool(
            "hello",
            "Hello / briefing",
            "Call first. Names you and returns a briefing: active agents (whose, repo@branch, what they hold), \
             leases and queues in your repo, open tasks, recent discoveries/decisions, your pings. Enough to start safely.",
            json!({
                "name": { "type": "string", "description": "Short lowercase name, e.g. \"api-agent\". Reusing a name resumes its leases and inbox." },
                "repo": { "type": "string", "description": "Git remote URL (`git remote get-url origin`) or project name. Path leases are per repo." },
                "branch": { "type": "string", "description": "Current git branch." },
                "working_on": { "type": "string", "description": "One line: what you are about to do." },
                "session": { "type": "string", "description": "Session key from the swarmboard SessionStart hook message, if you saw one." }
            }),
            &["name"],
            false,
        ),
        tool(
            "status",
            "Set status",
            "Update your status line (and repo/branch if you switched). Others see it in their briefing.",
            json!({
                "working_on": { "type": "string", "description": "e.g. \"migrating sessions table (#12)\". Empty clears." },
                "repo": { "type": "string", "description": "Git remote URL or project name." },
                "branch": { "type": "string", "description": "Current git branch." }
            }),
            &["working_on"],
            false,
        ),
        tool(
            "check",
            "Check before editing",
            "Before editing files or using a shared resource: who holds overlapping leases (note, expiry), who is queued, \
             and recent posts mentioning those paths. Read-only and fast.",
            json!({
                "paths": scope_list("Files, dirs or globs in your repo, e.g. [\"server/src/auth/login.rs\", \"web/src/**\"]."),
                "scopes": scope_list("Resources or tasks, e.g. [\"ci\", \"deploy:staging\", \"#12\"].")
            }),
            &[],
            true,
        ),
        tool(
            "claim",
            "Claim lease",
            "Take an exclusive lease before editing files or using a shared resource. scope: path/glob in your repo \
             (\"server/src/auth/**\"), resource (\"ci\", \"deploy:staging\", \"db:migrations\", \"port:3000\", \"release\") \
             or task (\"#12\", also marks it in_progress). Path leases cover every branch of the repo. If someone holds an overlapping lease you get an error naming \
             them; do something else, or pass wait:true to queue (you're pinged when it's yours). Leases renew on every \
             tool call you make and lapse after `minutes` of silence.",
            json!({
                "scope": { "type": "string", "description": "Path/glob, resource name, or \"#<post number>\"." },
                "note": { "type": "string", "description": "What you're doing, shown to others, e.g. \"rewriting token refresh\"." },
                "minutes": { "type": "integer", "minimum": 1, "maximum": 480, "description": "Lapse after this many minutes without tool calls from you (default 30)." },
                "wait": { "type": "boolean", "description": "If taken, join the FIFO queue instead of failing." }
            }),
            &["scope"],
            false,
        ),
        tool(
            "release",
            "Release lease",
            "Release a lease when done (also leaves its queue); the next waiter gets it along with your note. all:true \
             releases everything. For a task scope \"#12\", status sets the post status (e.g. \"done\") and note is also \
             posted as a reply.",
            json!({
                "scope": { "type": "string", "description": "Scope exactly as claimed." },
                "all": { "type": "boolean", "description": "Release all your leases and queue spots." },
                "status": { "type": "string", "enum": STATUSES, "description": "For \"#n\": post status after release (default open)." },
                "note": { "type": "string", "description": "Short outcome (\"tests green\"), shown to the next holder and in the feed; for \"#n\" also posted as a reply." }
            }),
            &[],
            false,
        ),
        tool(
            "claims",
            "List leases",
            "Current leases and queues in your repo plus shared resources and tasks. all:true shows every repo.",
            json!({ "all": { "type": "boolean", "description": "Include other repos." } }),
            &[],
            true,
        ),
        tool(
            "post",
            "Post",
            "Share something others need: task (work to do; claim \"#n\" to take it), discovery (a fact you learned), \
             decision, question, blocker, handoff (state + next steps for whoever continues), note. \
             @handle/agent or @handle in the body pings them.",
            json!({
                "topic": { "type": "string", "description": "Topic slug, e.g. \"auth\", \"ci\", \"ui\". Created if new." },
                "title": { "type": "string", "description": "Short, specific." },
                "body": { "type": "string", "description": "Markdown. Mention files by path so check() finds it." },
                "kind": { "type": "string", "enum": KINDS, "description": "Default \"note\"." }
            }),
            &["topic", "title"],
            false,
        ),
        tool(
            "feed",
            "Feed",
            "Recent posts, newest first, one line each.",
            json!({
                "topic": { "type": "string", "description": "Topic slug." },
                "kind": { "type": "string", "description": "Comma-separated kinds, e.g. \"discovery,decision\"." },
                "status": { "type": "string", "description": "Comma-separated statuses, or \"active\" (open, in_progress, blocked)." },
                "q": { "type": "string", "description": "Text search in title and body (e.g. a file path)." },
                "since_minutes": { "type": "integer", "minimum": 1, "description": "Only posts updated in the last N minutes." },
                "limit": { "type": "integer", "minimum": 1, "maximum": 100, "description": "Default 20." }
            }),
            &[],
            true,
        ),
        tool(
            "read",
            "Read post",
            "One post in full, with replies.",
            json!({ "number": number("Post number, e.g. 12.") }),
            &["number"],
            true,
        ),
        tool(
            "reply",
            "Reply",
            "Reply to a post. Its author and whoever has it claimed are pinged.",
            json!({
                "number": number("Post number."),
                "body": { "type": "string", "description": "Markdown." }
            }),
            &["number", "body"],
            false,
        ),
        tool(
            "update",
            "Update post",
            "Change a post's status, title, body, topic or kind. done/closed also clears its claim.",
            json!({
                "number": number("Post number."),
                "status": { "type": "string", "enum": STATUSES, "description": "New status." },
                "title": { "type": "string", "description": "New title." },
                "body": { "type": "string", "description": "New body (replaces the old one)." },
                "topic": { "type": "string", "description": "Move to this topic (created if new)." },
                "kind": { "type": "string", "enum": KINDS, "description": "New kind." }
            }),
            &["number"],
            false,
        ),
        tool(
            "send",
            "Send message",
            "Message another agent or human; they see it on their next turn. to: \"handle/agent\", \"agent\" (if unique), \
             \"@handle\" (the human), \"handle/*\" (all their agents) or \"@all\". To answer a ping, send to its sender.",
            json!({
                "to": { "type": "string", "description": "e.g. \"bob/ui-agent\", \"ui-agent\", \"@bob\", \"bob/*\", \"@all\"." },
                "message": { "type": "string", "description": "What you need or want them to know." },
                "re": { "type": "integer", "minimum": 1, "description": "Related post number." }
            }),
            &["to", "message"],
            false,
        ),
        tool(
            "inbox",
            "Inbox",
            "Your unread pings (marks them read). New pings are also appended to every tool result. all:true shows the last 30.",
            json!({ "all": { "type": "boolean", "description": "Include already-read pings." } }),
            &[],
            false,
        ),
        tool(
            "agents",
            "Agents",
            "Who is around: whose agent, repo@branch, status line, what they hold.",
            json!({ "all": { "type": "boolean", "description": "Include inactive agents." } }),
            &[],
            true,
        ),
        tool("topics", "Topics", "Topics with open/total post counts.", json!({}), &[], true),
        tool(
            "upload",
            "Upload file",
            "Store a file (screenshot, log, diff) and get a URL to embed as ![](url). Pass base64 (max 10 MB) or a public \
             url. With neither, returns a one-time curl command that uploads a local file directly (best for files on disk).",
            json!({
                "filename": { "type": "string", "description": "File name, e.g. \"login-bug.png\" (a path is fine for the curl command)." },
                "base64": { "type": "string", "description": "File contents, base64 (a data: URL is fine)." },
                "url": { "type": "string", "description": "Public http(s) URL to copy." }
            }),
            &["filename"],
            false,
        ),
    ])
}

// ---------------------------------------------------------------------------------------------
// Argument parsing (lenient types, strict keys)
// ---------------------------------------------------------------------------------------------

struct Args {
    map: Map<String, Value>,
}

impl Args {
    fn parse(v: &Value, allowed: &[&str]) -> Result<Args, String> {
        let map = match v {
            Value::Null => Map::new(),
            Value::Object(m) => m.clone(),
            _ => return Err("arguments must be an object".into()),
        };
        if let Some(k) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(if allowed.is_empty() {
                format!("unknown argument \"{k}\" (this tool takes no arguments)")
            } else {
                format!("unknown argument \"{k}\" (allowed: {})", allowed.join(", "))
            });
        }
        Ok(Args { map })
    }

    fn get(&self, k: &str) -> Option<&Value> {
        self.map.get(k).filter(|v| !v.is_null())
    }

    fn str(&self, k: &str) -> Result<Option<String>, String> {
        match self.get(k) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(Value::Number(n)) => Ok(Some(n.to_string())),
            Some(Value::Bool(b)) => Ok(Some(b.to_string())),
            Some(_) => Err(format!("\"{k}\" must be a string")),
        }
    }

    fn req_str(&self, k: &str) -> Result<String, String> {
        self.str(k)?.filter(|s| !s.trim().is_empty()).ok_or_else(|| format!("\"{k}\" is required"))
    }

    fn int(&self, k: &str) -> Result<Option<i64>, String> {
        match self.get(k) {
            None => Ok(None),
            Some(Value::Number(n)) => n
                .as_i64()
                .or_else(|| n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
                .map(Some)
                .ok_or_else(|| format!("\"{k}\" must be an integer")),
            Some(Value::String(s)) => {
                let t = s.trim().trim_start_matches('#');
                if t.is_empty() {
                    return Ok(None);
                }
                t.parse().map(Some).map_err(|_| format!("\"{k}\" must be an integer"))
            }
            Some(_) => Err(format!("\"{k}\" must be an integer")),
        }
    }

    /// A string array (a single string or comma-separated string is accepted too).
    fn str_list(&self, k: &str) -> Result<Vec<String>, String> {
        let out: Vec<String> = match self.get(k) {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|v| v.as_str().map(str::to_string).ok_or_else(|| format!("\"{k}\" must be an array of strings")))
                .collect::<Result<_, _>>()?,
            Some(Value::String(s)) => s.split(',').map(str::to_string).collect(),
            Some(_) => return Err(format!("\"{k}\" must be an array of strings")),
        };
        Ok(out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).take(50).collect())
    }

    fn req_int(&self, k: &str) -> Result<i64, String> {
        self.int(k)?.ok_or_else(|| format!("\"{k}\" is required"))
    }

    fn bool(&self, k: &str) -> Result<Option<bool>, String> {
        match self.get(k) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(Value::String(s)) => match s.trim().to_lowercase().as_str() {
                "true" | "yes" | "1" => Ok(Some(true)),
                "false" | "no" | "0" | "" => Ok(Some(false)),
                _ => Err(format!("\"{k}\" must be true or false")),
            },
            Some(Value::Number(n)) => Ok(Some(n.as_i64().unwrap_or(0) != 0)),
            Some(_) => Err(format!("\"{k}\" must be true or false")),
        }
    }
}

fn err_text(e: AppError) -> String {
    match e {
        AppError::Internal(err) => {
            tracing::error!(error = ?err, "mcp tool internal error");
            "internal error; try again".to_string()
        }
        other => other.to_string(),
    }
}

fn human_size(n: i64) -> String {
    if n >= 1_048_576 {
        format!("{:.1} MB", n as f64 / 1_048_576.0)
    } else if n >= 1024 {
        format!("{} KB", n / 1024)
    } else {
        format!("{n} B")
    }
}

// ---------------------------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------------------------

/// Handle `tools/call` params; returns the CallToolResult JSON.
pub async fn call(ctx: &mut ToolCtx<'_>, params: &Value) -> Result<Value, CallError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| CallError { code: -32602, message: "tools/call needs params.name".into() })?
        .to_string();
    if !TOOL_NAMES.contains(&name.as_str()) {
        return Err(CallError { code: -32602, message: format!("Unknown tool: {name}") });
    }
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    let db = &ctx.state.db;

    let mut fresh_provisional = false;
    let agent: Option<AgentRef> = if name == "hello" {
        None
    } else {
        fresh_provisional = ctx.session.agent_id.is_none();
        match collab::ensure_session_agent(db, ctx.session).await {
            Ok(a) => {
                if !fresh_provisional {
                    let _ = collab::touch_agent(db, &a.id).await;
                    // Activity keeps the agent's leases alive.
                    if let Err(e) = leases::renew(db, &a.id).await {
                        tracing::warn!(error = %e, "lease renew failed");
                    }
                }
                Some(a)
            }
            Err(e) => return Ok(result(vec![err_text(e)], true)),
        }
    };

    if let Err(e) = leases::tick(db, &ctx.auth.team_id).await {
        tracing::warn!(error = %e, "lease tick failed");
    }
    let outcome = run(ctx, &name, &args, agent.as_ref()).await;
    let (mut text, is_error) = match outcome {
        Ok(t) => (t, false),
        Err(t) => (t, true),
    };
    if fresh_provisional && let Some(a) = &agent {
        text.push_str(&format!("\n(You are \"{}\" for now. Call hello with a short name and your repo first.)", a.display));
    }
    let mut blocks = vec![text];

    if name != "inbox"
        && let Some(aid) = ctx.session.agent_id.clone()
    {
        {
            match collab::take_agent_pings(db, &aid).await {
                Ok(pings) if !pings.is_empty() => {
                    tracing::debug!(tool = %name, delivered = pings.len(), "ping piggyback");
                    let me = collab::agent_by_id(db, &aid).await.ok().flatten().map(|a| a.display);
                    blocks.push(format::pings_block(&pings, me.as_deref(), now_ms()));
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "ping piggyback failed"),
            }
        }
    }
    Ok(result(blocks, is_error))
}

fn result(blocks: Vec<String>, is_error: bool) -> Value {
    let content: Vec<Value> = blocks.into_iter().map(|t| json!({ "type": "text", "text": t })).collect();
    json!({ "content": content, "isError": is_error })
}

const PROTOCOL: &str = "Protocol: check/claim paths before editing and resources (ci, deploy:*, db:migrations, port:*) before using them. \
If taken: do something else, or claim(wait:true) to queue. Leases renew while you make tool calls and lapse when you go quiet; release when done.\n\
post discoveries/decisions/handoffs others need. Pings appear at the end of tool results; answer with send.";

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

async fn run(ctx: &mut ToolCtx<'_>, name: &str, args: &Value, agent: Option<&AgentRef>) -> Result<String, String> {
    let now = now_ms();
    let state = ctx.state;
    let db = &state.db;
    if name == "hello" {
        let a = Args::parse(args, &["name", "repo", "branch", "working_on", "session"])?;
        let wanted = a.req_str("name")?;
        let working_on = a.str("working_on")?;
        let key = a.str("session")?;
        let (repo, branch) = (a.str("repo")?, a.str("branch")?);
        let mut out = collab::hello(db, ctx.session, &wanted, working_on.as_deref(), key.as_deref()).await.map_err(err_text)?;
        if repo.is_some() || branch.is_some() {
            collab::set_location(db, &out.agent.id, repo.as_deref(), branch.as_deref()).await.map_err(err_text)?;
            if let Ok(Some(fresh)) = collab::agent_by_id(db, &out.agent.id).await {
                out.agent = fresh;
            }
        }
        let _ = leases::renew(db, &out.agent.id).await;
        return hello_text(ctx, &out, key.is_some()).await.map_err(err_text);
    }
    let agent = agent.ok_or("no agent for this session")?;
    let actor = Actor { team_id: ctx.auth.team_id.clone(), user_id: ctx.auth.user_id.clone(), agent_id: Some(agent.id.clone()) };
    let repo = agent.repo.as_deref();

    match name {
        "status" => {
            let a = Args::parse(args, &["working_on", "repo", "branch"])?;
            let w = a.str("working_on")?.unwrap_or_default();
            collab::set_working_on(db, &agent.id, &w).await.map_err(err_text)?;
            let (r, b) = (a.str("repo")?, a.str("branch")?);
            if r.is_some() || b.is_some() {
                collab::set_location(db, &agent.id, r.as_deref(), b.as_deref()).await.map_err(err_text)?;
            }
            let fresh = collab::agent_by_id(db, &agent.id).await.map_err(err_text)?.unwrap_or_else(|| agent.clone());
            let loc = format::repo_label(fresh.repo.as_deref(), fresh.branch.as_deref()).map(|l| format!(" · {l}")).unwrap_or_default();
            Ok(if w.trim().is_empty() {
                format!("Status cleared ({}{loc}).", fresh.display)
            } else {
                format!("Status: {}{loc} — {}", fresh.display, w.trim())
            })
        }
        "claim" => {
            let a = Args::parse(args, &["scope", "note", "minutes", "wait"])?;
            let scope = a.req_str("scope")?;
            let note = a.str("note")?;
            let wait = a.bool("wait")?.unwrap_or(false);
            let o = leases::claim(db, &actor, repo, &scope, note.as_deref(), a.int("minutes")?, wait).await;
            match o {
                Ok(ClaimOutcome::Granted { scope, repo: r, minutes, renewed, post, .. }) => {
                    let where_ = if r.is_empty() { String::new() } else { format!(" in {r}") };
                    let mut lines = vec![format!(
                        "{} {}{where_} for {minutes}m; it renews with each tool call you make and lapses if you go quiet. \
                         release(scope:\"{}\") when done.",
                        if renewed { "Renewed" } else { "Claimed" },
                        scope.text,
                        scope.text
                    )];
                    if let Some(p) = post {
                        lines.push(format::post_line(&p, now));
                    }
                    if scope.kind == ScopeKind::Path && r.is_empty() {
                        lines.push("No repo set, so this blocks that path in every repo. Pass repo to status or hello.".into());
                    }
                    Ok(lines.join("\n"))
                }
                Ok(ClaimOutcome::Taken { scope, holders, .. }) => {
                    let mut lines = vec![format!("TAKEN: {} overlaps:", scope.text)];
                    lines.extend(holders.iter().map(|h| format!("  {}", format::lease_line(h, None, now))));
                    let who = holders.first().map(LeaseRow::display).unwrap_or_default();
                    lines.push(format!(
                        "Don't touch it. Work on something else, send(to:\"{who}\") to coordinate, or claim(scope:\"{}\", wait:true) to queue.",
                        scope.text
                    ));
                    Err(lines.join("\n"))
                }
                Ok(ClaimOutcome::Queued { scope, position, holders, .. }) => {
                    let mut lines = vec![format!("Queued for {} at position {position}. Held by:", scope.text)];
                    lines.extend(holders.iter().map(|h| format!("  {}", format::lease_line(h, None, now))));
                    lines.push("You'll get a ping when it's yours; don't touch it until then. Work on something else meanwhile.".into());
                    Ok(lines.join("\n"))
                }
                Err(e) => Err(err_text(e)),
            }
        }
        "release" => {
            let a = Args::parse(args, &["scope", "all", "status", "note"])?;
            let scope = a.str("scope")?;
            let all = a.bool("all")?.unwrap_or(false);
            let (status, note) = (a.str("status")?, a.str("note")?);
            let o = leases::release(db, &actor, scope.as_deref(), all, status.as_deref(), note.as_deref()).await.map_err(err_text)?;
            let mut lines = Vec::new();
            if !o.released.is_empty() {
                lines.push(format::released_line(&o.released, &o.note, &o.granted));
            }
            if !o.left_queue.is_empty() {
                lines.push(format!("Left the queue for {}.", o.left_queue.join(", ")));
            }
            lines.extend(o.posts.iter().map(|p| format::post_line(p, now)));
            lines.extend(o.warnings);
            if lines.is_empty() {
                lines.push("Nothing to release.".into());
            }
            Ok(lines.join("\n"))
        }
        "check" => {
            let a = Args::parse(args, &["paths", "scopes"])?;
            let mut scopes = a.str_list("paths")?;
            scopes.extend(a.str_list("scopes")?);
            if scopes.is_empty() {
                return Err("pass paths (files/globs) and/or scopes (resources like \"ci\", tasks like \"#12\")".into());
            }
            let items = leases::check(db, &actor, repo, &scopes).await.map_err(err_text)?;
            let mut lines = Vec::new();
            let mut terms: Vec<String> = Vec::new();
            for it in &items {
                let mut l = format!("{}: ", it.scope.text);
                if !it.others.is_empty() {
                    let hs: Vec<String> = it
                        .others
                        .iter()
                        .map(|h| {
                            let note = if h.note.is_empty() { String::new() } else { format!(" \"{}\"", collab::truncate_chars(&h.note, 80)) };
                            format!("{} ({}{note}, expires {})", h.display(), h.scope, format::until(h.expires_at, now))
                        })
                        .collect();
                    l.push_str(&format!("TAKEN by {}", hs.join("; ")));
                } else if !it.mine.is_empty() {
                    l.push_str(&format!("yours ({})", it.mine.iter().map(|m| m.scope.clone()).collect::<Vec<_>>().join(", ")));
                } else {
                    l.push_str("free");
                }
                if !it.waiting.is_empty() {
                    l.push_str(&format!(" · waiting: {}", it.waiting.join(", ")));
                }
                lines.push(l);
                if it.scope.kind == ScopeKind::Path {
                    let lit = it.scope.text.split(['*', '?', '[']).next().unwrap_or("").trim_end_matches('/');
                    if lit.len() >= 3 {
                        terms.push(lit.to_string());
                    }
                    if let Some(base) = lit.rsplit('/').next().filter(|b| b.contains('.') && b.len() >= 4 && *b != lit) {
                        terms.push(base.to_string());
                    }
                }
            }
            let mut related: Vec<Post> = Vec::new();
            for t in terms.iter().take(10) {
                let f = PostFilter { q: Some(t.clone()), limit: Some(5), ..Default::default() };
                for p in collab::list_posts(db, &actor.team_id, &f).await.map_err(err_text)? {
                    if p.updated_at > now - 7 * 86_400_000 && !related.iter().any(|r| r.number == p.number) {
                        related.push(p);
                    }
                }
            }
            if !related.is_empty() {
                related.sort_by_key(|p| -p.updated_at);
                lines.push("Related posts:".into());
                lines.extend(related.iter().take(6).map(|p| format!("  {}", format::post_line(p, now))));
            }
            Ok(lines.join("\n"))
        }
        "claims" => {
            let a = Args::parse(args, &["all"])?;
            let all = a.bool("all")?.unwrap_or(false);
            claims_text(db, &actor.team_id, &agent.id, if all { None } else { repo }, now).await.map_err(err_text)
        }
        "post" => {
            let a = Args::parse(args, &["topic", "title", "body", "kind"])?;
            let topic = collab::ensure_board(db, &actor, &a.req_str("topic")?).await.map_err(err_text)?;
            let new = NewPost {
                board: topic.clone(),
                title: a.req_str("title")?,
                body: a.str("body")?.unwrap_or_default(),
                kind: a.str("kind")?,
                status: None,
                priority: None,
            };
            let p = collab::create_post(db, &actor, new).await.map_err(err_text)?;
            let mentions = collab::parse_mentions(&p.body);
            let mut out = format!("Posted {}", format::post_line(&p, now));
            if !mentions.is_empty() {
                out.push_str(&format!("\nPinged mentions: {}.", mentions.iter().map(|m| format!("@{m}")).collect::<Vec<_>>().join(", ")));
            }
            if p.kind == "task" {
                out.push_str(&format!("\nclaim(scope:\"#{}\") to take it yourself.", p.number));
            }
            Ok(out)
        }
        "feed" => {
            let a = Args::parse(args, &["topic", "kind", "status", "q", "since_minutes", "limit"])?;
            let limit = a.int("limit")?.unwrap_or(20).clamp(1, 100) as usize;
            let since = a.int("since_minutes")?.map(|m| now - m.max(1) * 60_000);
            let filter = PostFilter {
                board: a.str("topic")?,
                status: a.str("status")?,
                kind: a.str("kind")?,
                q: a.str("q")?,
                limit: Some(if since.is_some() { 200 } else { limit as i64 }),
                ..Default::default()
            };
            let mut posts = collab::list_posts(db, &actor.team_id, &filter).await.map_err(err_text)?;
            if let Some(t) = since {
                posts.retain(|p| p.updated_at >= t);
            }
            posts.sort_by_key(|p| -p.updated_at);
            posts.truncate(limit);
            if posts.is_empty() {
                return Ok("No posts match.".into());
            }
            let mut lines = vec![format!("{}:", plural(posts.len(), "post", "posts"))];
            lines.extend(posts.iter().map(|p| format::post_line(p, now)));
            Ok(lines.join("\n"))
        }
        "read" => {
            let a = Args::parse(args, &["number"])?;
            let d = collab::get_post(db, &actor.team_id, a.req_int("number")?).await.map_err(err_text)?;
            Ok(format::post_full(&d.post, &d.comments, now))
        }
        "reply" => {
            let a = Args::parse(args, &["number", "body"])?;
            let number = a.req_int("number")?;
            let (_, notified) = collab::reply(db, &actor, number, &a.req_str("body")?).await.map_err(err_text)?;
            Ok(if notified.is_empty() {
                format!("Replied on #{number}.")
            } else {
                format!("Replied on #{number}. Notified {}.", notified.iter().map(|r| format::recipient_label(r)).collect::<Vec<_>>().join(", "))
            })
        }
        "update" => {
            let a = Args::parse(args, &["number", "status", "title", "body", "topic", "kind"])?;
            let number = a.req_int("number")?;
            let topic = match a.str("topic")?.filter(|t| !t.trim().is_empty()) {
                Some(t) => Some(collab::ensure_board(db, &actor, &t).await.map_err(err_text)?),
                None => None,
            };
            let patch = PostPatch {
                title: a.str("title")?,
                body: a.str("body")?,
                status: a.str("status")?,
                kind: a.str("kind")?,
                priority: None,
                board: topic,
            };
            if patch.title.is_none() && patch.body.is_none() && patch.status.is_none() && patch.kind.is_none() && patch.board.is_none() {
                return Err("nothing to update: pass status, title, body, topic or kind".into());
            }
            let p = collab::update_post(db, &actor, number, patch).await.map_err(err_text)?;
            Ok(format!("Updated {}", format::post_line(&p, now)))
        }
        "send" => {
            let a = Args::parse(args, &["to", "message", "re"])?;
            let to = a.req_str("to")?;
            let message = a.req_str("message")?;
            let sent = collab::send_ping(db, &actor, &to, &message, a.int("re")?).await.map_err(err_text)?;
            let who: Vec<String> = sent.recipients.iter().map(|r| format::recipient_label(r)).collect();
            Ok(format!("Sent to {}. They see it on their next turn.", who.join(", ")))
        }
        "inbox" => {
            let a = Args::parse(args, &["all"])?;
            let all = a.bool("all")?.unwrap_or(false);
            let pings = collab::agent_inbox(db, &agent.id, all).await.map_err(err_text)?;
            if pings.is_empty() {
                return Ok(if all { "No pings yet.".into() } else { "No new pings. (inbox all:true shows recent ones.)".into() });
            }
            let mut lines = vec![if all {
                format!("Last {} pings to {}:", pings.len(), agent.display)
            } else {
                format!("{} — answer with send(to:<sender>):", plural(pings.len(), "new ping", "new pings"))
            }];
            lines.extend(pings.iter().map(|p| format::ping_line(p, Some(&agent.display), now)));
            Ok(lines.join("\n"))
        }
        "agents" => {
            let a = Args::parse(args, &["all"])?;
            let all = a.bool("all")?.unwrap_or(false);
            let agents = collab::list_agents(db, &actor.team_id, !all).await.map_err(err_text)?;
            if agents.is_empty() {
                return Ok("No agents.".into());
            }
            let mut lines = vec![format!("{}{}:", plural(agents.len(), "agent", "agents"), if all { "" } else { " active" })];
            lines.extend(agents.iter().map(|x| format::agent_line(x, Some(&agent.id), now)));
            Ok(lines.join("\n"))
        }
        "topics" => {
            Args::parse(args, &[])?;
            let boards = collab::list_boards(db, &actor.team_id).await.map_err(err_text)?;
            if boards.is_empty() {
                return Ok("No topics yet; post(topic:…) creates one.".into());
            }
            let mut lines = vec![format!("{}:", plural(boards.len(), "topic", "topics"))];
            lines.extend(boards.iter().map(|b| format::board_line(b, now)));
            Ok(lines.join("\n"))
        }
        "upload" => upload(state, &actor, args).await,
        _ => Err(format!("unknown tool {name}")),
    }
}

/// Leases + queue lines, filtered to `repo` (plus repo-less leases) when given.
async fn claims_text(db: &sqlx::SqlitePool, team_id: &str, me: &str, repo: Option<&str>, now: i64) -> crate::error::AppResult<String> {
    let view = leases::list(db, team_id).await?;
    let rows = leases::active(db, team_id).await?;
    let relevant = |r: &str| repo.is_none_or(|mine| r.is_empty() || r == mine);
    let ls: Vec<&LeaseRow> = rows.iter().filter(|l| relevant(&l.repo)).collect();
    let qs: Vec<_> = view.queue.iter().filter(|w| relevant(&w.repo)).collect();
    let scope_desc = match repo {
        Some(r) => format!("{r} + shared"),
        None => "all repos".into(),
    };
    if ls.is_empty() && qs.is_empty() {
        return Ok(format!("No leases or queues ({scope_desc})."));
    }
    let mut lines = vec![format!("Leases ({scope_desc}):")];
    if ls.is_empty() {
        lines.push("  none".into());
    }
    lines.extend(ls.iter().map(|l| format!("  {}", format::lease_line(l, Some(me), now))));
    if !qs.is_empty() {
        lines.push("Queue:".into());
        lines.extend(qs.iter().map(|w| {
            let you = if w.waiter.agent_id.as_deref() == Some(me) { " (you)" } else { "" };
            format!("  {} — {}{you} #{} in line · since {}", w.scope, w.waiter.display, w.position, collab::ago(w.created_at, now))
        }));
    }
    Ok(lines.join("\n"))
}

async fn hello_text(ctx: &ToolCtx<'_>, out: &HelloOutcome, linked: bool) -> crate::error::AppResult<String> {
    let db = &ctx.state.db;
    let now = now_ms();
    let team_id = ctx.auth.team_id.as_str();
    let org = match crate::domain::accounts::get_team(db, team_id).await? {
        Some(t) => format!("org \"{}\" ({})", t.name, t.slug),
        None => "your org".to_string(),
    };
    let me = &out.agent;
    let loc = format::repo_label(me.repo.as_deref(), me.branch.as_deref());
    let mut lines = vec![format!(
        "You are {} ({}'s agent) on {org}{}.",
        me.display,
        me.handle,
        loc.as_ref().map(|l| format!(" · {l}")).unwrap_or_default()
    )];
    if let Some(req) = &out.requested {
        lines.push(format!("\"{req}\" is active in another session, so you are \"{}\".", me.name));
    }
    if out.adopted {
        lines.push("Resumed your earlier identity: its leases and pings are yours.".into());
    } else if let Some(prev) = &out.renamed_from {
        lines.push(format!("Renamed from {prev}."));
    }
    if me.repo.is_none() {
        lines.push("No repo set: pass repo (git remote URL) so path leases are scoped to your codebase.".into());
    }
    if linked {
        lines.push("Session key linked: pings also reach you between turns.".into());
    }
    if let Some(w) = me.working_on.as_deref().filter(|w| !w.is_empty()) {
        lines.push(format!("Working on: {w}"));
    }
    let mine = leases::agent_leases(db, &me.id).await?;
    if !mine.is_empty() {
        lines.push(format!("You hold: {}", mine.iter().map(|l| format!("{} (expires {})", l.scope, format::until(l.expires_at, now))).collect::<Vec<_>>().join(", ")));
    }
    let waits = leases::agent_waits(db, team_id, &me.id).await?;
    if !waits.is_empty() {
        lines.push(format!("You're queued for: {}", waits.iter().map(|(s, p)| format!("{s} (#{p})")).collect::<Vec<_>>().join(", ")));
    }

    let agents = collab::list_agents(db, team_id, true).await?;
    let others: Vec<String> = agents.iter().filter(|a| a.id != me.id).take(15).map(|a| format!("  {}", format::agent_line(a, None, now))).collect();
    if others.is_empty() {
        lines.push("Agents: nobody else active.".into());
    } else {
        lines.push(format!("Agents active ({}):", others.len()));
        lines.extend(others);
    }

    let claims = claims_text(db, team_id, &me.id, me.repo.as_deref(), now).await?;
    lines.extend(claims.lines().map(str::to_string));

    let open = collab::list_posts(
        db,
        team_id,
        &PostFilter { status: Some("open".into()), kind: Some("task".into()), claimed_by: Some("none".into()), limit: Some(200), ..Default::default() },
    )
    .await?;
    if open.is_empty() {
        lines.push("Open tasks: none.".into());
    } else {
        lines.push(format!("Open tasks ({}, claim \"#n\" to take one):", open.len()));
        lines.extend(open.iter().take(5).map(|p| format!("  {}", format::post_line(p, now))));
    }
    let recent = collab::list_posts(
        db,
        team_id,
        &PostFilter { kind: Some("discovery,decision,blocker,handoff,question".into()), limit: Some(30), ..Default::default() },
    )
    .await?;
    let recent: Vec<&Post> = recent.iter().filter(|p| p.updated_at > now - 86_400_000).take(10).collect();
    if !recent.is_empty() {
        lines.push("Last 24h (discoveries, decisions, blockers, handoffs, questions):".into());
        lines.extend(recent.iter().map(|p| format!("  {}", format::post_line(p, now))));
    }
    lines.push(PROTOCOL.into());
    Ok(lines.join("\n"))
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

async fn upload(state: &AppState, actor: &Actor, args: &Value) -> Result<String, String> {
    let a = Args::parse(args, &["filename", "base64", "url"])?;
    let filename = a.req_str("filename")?;
    let b64 = a.str("base64")?.filter(|s| !s.trim().is_empty());
    let url = a.str("url")?.filter(|s| !s.trim().is_empty());
    let saved = match (b64, url) {
        (Some(_), Some(_)) => return Err("pass base64 or url, not both".into()),
        (Some(b), None) => {
            let (data_ct, data) = match b.trim().strip_prefix("data:") {
                Some(rest) => match rest.split_once(',') {
                    Some((meta, data)) => (meta.split(';').next().map(str::to_string).filter(|s| !s.is_empty()), data.to_string()),
                    None => (None, b.clone()),
                },
                None => (None, b.clone()),
            };
            if data.len() > collab::MAX_FILE_BYTES / 3 * 4 + 1024 {
                return Err("file is over 10 MB; max is 10 MB".into());
            }
            let cleaned: String = data.chars().filter(|c| !c.is_whitespace()).collect();
            use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
            let bytes = STANDARD
                .decode(&cleaned)
                .or_else(|_| STANDARD_NO_PAD.decode(&cleaned))
                .or_else(|_| URL_SAFE.decode(&cleaned))
                .or_else(|_| URL_SAFE_NO_PAD.decode(&cleaned))
                .map_err(|e| format!("base64 did not decode ({e}). For files on disk, omit base64 and use the curl command."))?;
            collab::save_file(state, actor, &filename, data_ct.as_deref(), &bytes).await.map_err(err_text)?
        }
        (None, Some(u)) => {
            let (bytes, ct) = crate::files::fetch_public_url(u.trim()).await?;
            collab::save_file(state, actor, &filename, ct.as_deref(), &bytes).await.map_err(err_text)?
        }
        (None, None) => {
            let code = collab::create_upload_link(&state.db, actor, 60 * 60 * 1000).await.map_err(err_text)?;
            let safe = collab::sanitize_filename(&filename);
            return Ok(format!(
                "Upload the local file with:\ncurl -fsS -T {} '{}/u/{code}?filename={safe}'\n\
                 It prints JSON with a \"url\" to embed as ![{safe}](url). The link works once and expires in 1 hour.",
                shell_quote(&filename),
                state.cfg.public_api_url
            ));
        }
    };
    Ok(format!(
        "Uploaded {} ({}, {})\nURL: {}\nMarkdown: ![{}]({})",
        saved.filename,
        human_size(saved.size),
        saved.content_type,
        saved.url,
        saved.filename,
        saved.url
    ))
}
