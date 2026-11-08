//! Collaboration domain: boards, posts, claims, comments, pings, agents, files, activity.
//! OWNER: MCP subagent. The REST API (api/*) calls the pub fns below with the signatures
//! fixed here; extra fns (agent/session management for MCP + hooks) may be added freely.

pub mod leases;

use std::collections::HashSet;

use serde_json::{json, Value};
use sqlx::{FromRow, SqliteExecutor, SqlitePool};

use crate::{
    auth::RateLimiter,
    error::{AppError, AppResult},
    models::{
        Activity, Actor, Agent, AgentProfile, AgentStats, Author, Board, Comment, FileInfo, Lease, LeaseWait, MemberProfile,
        NewPost, Ping, PingSent, Post, PostDetail, PostFilter, PostPatch, AGENT_ACTIVE_MS, POST_KINDS, POST_STATUSES,
    },
    state::AppState,
    util::{new_id, now_ms, random_token},
};

/// Max stored file size.
pub const MAX_FILE_BYTES: usize = 10 * 1024 * 1024;
/// Uploads allowed per user per window.
const UPLOAD_LIMIT: usize = 60;
const UPLOAD_WINDOW_MS: i64 = 15 * 60 * 1000;
/// Statuses that count as "open" work (`status: "active"`).
pub const ACTIVE_STATUSES: &[&str] = &["open", "in_progress", "blocked"];
const MAX_TITLE: usize = 200;
const MAX_BODY: usize = 100_000;
const MAX_COMMENT: usize = 20_000;
const MAX_PING: usize = 4_000;
const MAX_MENTIONS: usize = 10;
const MAX_AGENT_NAME: usize = 24;

// ---------------------------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------------------------

fn bad(msg: impl Into<String>) -> AppError {
    AppError::BadRequest(msg.into())
}

fn post_not_found(number: i64) -> AppError {
    AppError::NotFound(format!("no post #{number}"))
}

pub fn author(user_id: String, handle: String, agent_id: Option<String>, agent_name: Option<String>) -> Author {
    let display = match &agent_name {
        Some(n) => format!("{handle}/{n}"),
        None => handle.clone(),
    };
    Author { user_id, handle, agent_id: agent_name.as_ref().and(agent_id), agent_name, display }
}

/// "just now", "42s ago", "3m ago", "5h ago", "2d ago".
pub fn ago(then_ms: i64, now: i64) -> String {
    let s = ((now - then_ms).max(0)) / 1000;
    match s {
        0..=9 => "just now".to_string(),
        10..=59 => format!("{s}s ago"),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86_399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}

/// Collapse whitespace and cut to `max` chars (adds "…").
pub fn excerpt(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&flat, max)
}

pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Lowercase slug `[a-z0-9-]`, spaces/underscores/dots become `-`, runs collapsed, trimmed.
pub fn slugify(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.trim().chars().flat_map(|c| c.to_lowercase()) {
        let c = if c.is_ascii_alphanumeric() {
            c
        } else if matches!(c, '-' | '_' | ' ' | '.' | '/') {
            '-'
        } else {
            continue;
        };
        if c == '-' && (out.is_empty() || out.ends_with('-')) {
            continue;
        }
        out.push(c);
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

pub fn normalize_board_key(raw: &str) -> AppResult<String> {
    let key = slugify(raw);
    if key.is_empty() || key.len() > 24 {
        return Err(bad("board key must be 1-24 chars of a-z, 0-9, -"));
    }
    Ok(key)
}

pub fn normalize_agent_name(raw: &str) -> AppResult<String> {
    let name = slugify(raw);
    if name.is_empty() {
        return Err(bad("name must be a short lowercase slug like \"ui-fixer\""));
    }
    if name.len() > MAX_AGENT_NAME {
        return Err(bad(format!("name is too long (max {MAX_AGENT_NAME} chars)")));
    }
    Ok(name)
}

fn validate_kind(kind: &str) -> AppResult<String> {
    let k = kind.trim().to_lowercase();
    if POST_KINDS.contains(&k.as_str()) {
        Ok(k)
    } else {
        Err(bad(format!("kind must be one of: {}", POST_KINDS.join(", "))))
    }
}

fn validate_status(status: &str) -> AppResult<String> {
    let s = status.trim().to_lowercase().replace([' ', '-'], "_");
    let s = if s == "active" || s == "wip" { "in_progress".to_string() } else { s };
    if POST_STATUSES.contains(&s.as_str()) {
        Ok(s)
    } else {
        Err(bad(format!("status must be one of: {}", POST_STATUSES.join(", "))))
    }
}

fn validate_priority(p: i64) -> AppResult<i64> {
    if (0..=4).contains(&p) {
        Ok(p)
    } else {
        Err(bad("priority must be 0 (none), 1 (urgent), 2 (high), 3 (medium) or 4 (low)"))
    }
}

fn validate_title(t: &str) -> AppResult<String> {
    let t = t.trim();
    if t.is_empty() {
        return Err(bad("title is required"));
    }
    if t.chars().count() > MAX_TITLE {
        return Err(bad(format!("title is too long (max {MAX_TITLE} chars)")));
    }
    Ok(t.to_string())
}

fn validate_body(b: &str) -> AppResult<()> {
    if b.chars().count() > MAX_BODY {
        return Err(bad(format!("body is too long (max {MAX_BODY} chars)")));
    }
    Ok(())
}

/// Comma-separated list → validated, normalized, comma-joined (for the `instr` filter trick).
fn status_list(raw: &str) -> AppResult<String> {
    let mut out = Vec::new();
    for part in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let p = part.to_lowercase();
        if p == "active" || p == "open_work" {
            out.extend(ACTIVE_STATUSES.iter().map(|s| s.to_string()));
        } else if p == "all" {
            out.extend(POST_STATUSES.iter().map(|s| s.to_string()));
        } else {
            out.push(validate_status(&p)?);
        }
    }
    Ok(out.join(","))
}

fn kind_list(raw: &str) -> AppResult<String> {
    let mut out = Vec::new();
    for part in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        out.push(validate_kind(part)?);
    }
    Ok(out.join(","))
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.is_unique_violation())
}

async fn record_event(
    ex: impl SqliteExecutor<'_>,
    actor: &Actor,
    kind: &str,
    post_id: Option<&str>,
    data: Value,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO events (team_id, at, actor_user_id, actor_agent_id, kind, post_id, data)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(actor.team_id.as_str())
    .bind(now_ms())
    .bind(actor.user_id.as_str())
    .bind(actor.agent_id.as_deref())
    .bind(kind)
    .bind(post_id)
    .bind(data.to_string())
    .execute(ex)
    .await?;
    Ok(())
}

/// Display of whoever performs `actor`'s actions ("paul/ui-fixer" or "paul").
pub async fn actor_display(db: &SqlitePool, actor: &Actor) -> AppResult<String> {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT u.handle, a.name FROM users u LEFT JOIN agents a ON a.id = ?2 WHERE u.id = ?1",
    )
    .bind(actor.user_id.as_str())
    .bind(actor.agent_id.as_deref())
    .fetch_optional(db)
    .await?;
    Ok(match row {
        Some((h, Some(n))) => format!("{h}/{n}"),
        Some((h, None)) => h,
        None => "someone".to_string(),
    })
}

// ---------------------------------------------------------------------------------------------
// Boards
// ---------------------------------------------------------------------------------------------

#[derive(FromRow)]
struct BoardRow {
    id: String,
    key: String,
    name: String,
    description: String,
    created_at: i64,
    open_count: i64,
    total_count: i64,
    last_activity_at: Option<i64>,
}

impl From<BoardRow> for Board {
    fn from(r: BoardRow) -> Self {
        Board {
            id: r.id,
            key: r.key,
            name: r.name,
            description: r.description,
            created_at: r.created_at,
            open_count: r.open_count,
            total_count: r.total_count,
            last_activity_at: r.last_activity_at,
        }
    }
}

macro_rules! board_select {
    () => {
        "SELECT b.id, b.key, b.name, b.description, b.created_at,
           (SELECT COUNT(*) FROM posts p WHERE p.board_id = b.id AND p.status IN ('open','in_progress','blocked')) AS open_count,
           (SELECT COUNT(*) FROM posts p WHERE p.board_id = b.id) AS total_count,
           (SELECT MAX(p.updated_at) FROM posts p WHERE p.board_id = b.id) AS last_activity_at
         FROM boards b "
    };
}

/// Boards with counts, ordered by key. Excludes archived.
pub async fn list_boards(db: &SqlitePool, team_id: &str) -> AppResult<Vec<Board>> {
    let rows: Vec<BoardRow> =
        sqlx::query_as(concat!(board_select!(), "WHERE b.team_id = ?1 AND b.archived_at IS NULL ORDER BY b.key"))
            .bind(team_id)
            .fetch_all(db)
            .await?;
    Ok(rows.into_iter().map(Board::from).collect())
}

/// `key` is normalized to lowercase slug [a-z0-9-]{1,24}; Conflict if it exists.
pub async fn create_board(db: &SqlitePool, actor: &Actor, key: &str, name: &str, description: &str) -> AppResult<Board> {
    let key = normalize_board_key(key)?;
    let name = name.trim();
    let name = if name.is_empty() { key.clone() } else { truncate_chars(name, 60) };
    let description = truncate_chars(description.trim(), 500);
    let id = new_id("brd");
    let res = sqlx::query(
        "INSERT INTO boards (id, team_id, key, name, description, created_by_user, created_by_agent, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(id.as_str())
    .bind(actor.team_id.as_str())
    .bind(key.as_str())
    .bind(name.as_str())
    .bind(description.as_str())
    .bind(actor.user_id.as_str())
    .bind(actor.agent_id.as_deref())
    .bind(now_ms())
    .execute(db)
    .await;
    match res {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => return Err(AppError::Conflict(format!("board \"{key}\" already exists"))),
        Err(e) => return Err(e.into()),
    }
    record_event(db, actor, "board_created", None, json!({ "key": key, "name": name })).await?;
    let row: BoardRow = sqlx::query_as(concat!(board_select!(), "WHERE b.id = ?1")).bind(id.as_str()).fetch_one(db).await?;
    Ok(row.into())
}

/// Topic (board) key for posting: created on first use. Returns the normalized key.
pub async fn ensure_board(db: &SqlitePool, actor: &Actor, raw: &str) -> AppResult<String> {
    let key = normalize_board_key(raw.trim().trim_start_matches('#'))?;
    match create_board(db, actor, &key, "", "").await {
        Ok(_) | Err(AppError::Conflict(_)) => Ok(key),
        Err(e) => Err(e),
    }
}

async fn board_id_by_key(db: &SqlitePool, team_id: &str, key: &str) -> AppResult<String> {
    let k = key.trim().to_lowercase();
    let k = k.trim_start_matches('#');
    let id: Option<String> =
        sqlx::query_scalar("SELECT id FROM boards WHERE team_id = ?1 AND key = ?2 AND archived_at IS NULL")
            .bind(team_id)
            .bind(k)
            .fetch_optional(db)
            .await?;
    match id {
        Some(id) => Ok(id),
        None => {
            let keys: Vec<String> =
                sqlx::query_scalar("SELECT key FROM boards WHERE team_id = ?1 AND archived_at IS NULL ORDER BY key")
                    .bind(team_id)
                    .fetch_all(db)
                    .await?;
            Err(AppError::NotFound(format!("no board \"{k}\" (boards: {})", keys.join(", "))))
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Posts
// ---------------------------------------------------------------------------------------------

#[derive(FromRow)]
struct PostRow {
    id: String,
    number: i64,
    board_key: String,
    kind: String,
    status: String,
    priority: i64,
    title: String,
    body: String,
    author_user_id: String,
    author_handle: String,
    author_agent_id: Option<String>,
    author_agent_name: Option<String>,
    claimed_user_id: Option<String>,
    claimed_user_handle: Option<String>,
    claimed_agent_id: Option<String>,
    claimed_agent_name: Option<String>,
    claimed_at: Option<i64>,
    comment_count: i64,
    created_at: i64,
    updated_at: i64,
}

impl PostRow {
    fn into_post(self, body_max: Option<usize>) -> Post {
        let claimed_by = match (self.claimed_user_id, self.claimed_user_handle) {
            (Some(uid), Some(h)) => Some(author(uid, h, self.claimed_agent_id, self.claimed_agent_name)),
            _ => None,
        };
        Post {
            id: self.id,
            number: self.number,
            board_key: self.board_key,
            kind: self.kind,
            status: self.status,
            priority: self.priority,
            title: self.title,
            body: match body_max {
                Some(n) => truncate_chars(&self.body, n),
                None => self.body,
            },
            author: author(self.author_user_id, self.author_handle, self.author_agent_id, self.author_agent_name),
            claimed_at: if claimed_by.is_some() { self.claimed_at } else { None },
            claimed_by,
            comment_count: self.comment_count,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

macro_rules! post_select {
    () => {
        "SELECT p.id, p.number, b.key AS board_key, p.kind, p.status, p.priority, p.title, p.body,
           p.author_user_id, au.handle AS author_handle, p.author_agent_id, aa.name AS author_agent_name,
           cu.id AS claimed_user_id, cu.handle AS claimed_user_handle,
           p.claimed_by_agent_id AS claimed_agent_id, ca.name AS claimed_agent_name, p.claimed_at,
           (SELECT COUNT(*) FROM comments c WHERE c.post_id = p.id) AS comment_count,
           p.created_at, p.updated_at
         FROM posts p
         JOIN boards b ON b.id = p.board_id
         JOIN users au ON au.id = p.author_user_id
         LEFT JOIN agents aa ON aa.id = p.author_agent_id
         LEFT JOIN agents ca ON ca.id = p.claimed_by_agent_id
         LEFT JOIN users cu ON cu.id = COALESCE(p.claimed_by_user_id, ca.user_id) "
    };
}

async fn post_by_number(db: &SqlitePool, team_id: &str, number: i64) -> AppResult<Option<Post>> {
    let row: Option<PostRow> = sqlx::query_as(concat!(post_select!(), "WHERE p.team_id = ?1 AND p.number = ?2"))
        .bind(team_id)
        .bind(number)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| r.into_post(None)))
}

async fn require_post(db: &SqlitePool, team_id: &str, number: i64) -> AppResult<Post> {
    post_by_number(db, team_id, number).await?.ok_or_else(|| post_not_found(number))
}

/// Who-matcher for the `claimed_by` / `author` filters: (mode, handle, name).
/// Modes: none | any | display (handle+name) | handle (anything by that human, incl. agents)
/// | human (the human themself) | either (agent name OR human handle).
fn who_filter(raw: Option<&str>) -> (Option<&'static str>, Option<String>, Option<String>) {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return (None, None, None);
    };
    let v = raw.to_lowercase();
    match v.as_str() {
        "none" | "nobody" | "unclaimed" | "-" => return (Some("none"), None, None),
        "any" | "anyone" | "claimed" | "*" => return (Some("any"), None, None),
        _ => {}
    }
    if let Some((h, n)) = v.trim_start_matches('@').split_once('/') {
        if n == "*" || n.is_empty() {
            return (Some("handle"), Some(h.to_string()), None);
        }
        return (Some("display"), Some(h.to_string()), Some(n.to_string()));
    }
    if let Some(h) = v.strip_prefix('@') {
        return (Some("human"), Some(h.to_string()), None);
    }
    (Some("either"), Some(v.clone()), Some(v))
}

pub async fn list_posts(db: &SqlitePool, team_id: &str, filter: &PostFilter) -> AppResult<Vec<Post>> {
    let board = filter.board.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| s.trim_start_matches('#').to_lowercase());
    let status = match filter.status.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Some(status_list(s)?),
        None => None,
    };
    let kind = match filter.kind.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(k) => Some(kind_list(k)?),
        None => None,
    };
    let (c_mode, c_handle, c_name) = who_filter(filter.claimed_by.as_deref());
    let (mut a_mode, a_handle, a_name) = who_filter(filter.author.as_deref());
    if a_mode == Some("either") {
        // Bare author word: that human's own posts; an agent name only if no member has that handle.
        a_mode = Some("bare");
    }
    let author_user = filter.author_user.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let q = filter.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let limit = filter.limit.unwrap_or(50).clamp(1, 200);
    let rows: Vec<PostRow> = sqlx::query_as(concat!(
        post_select!(),
        "WHERE p.team_id = ?1
           AND (?2 IS NULL OR b.key = ?2)
           AND (?3 IS NULL OR instr(',' || ?3 || ',', ',' || p.status || ',') > 0)
           AND (?4 IS NULL OR instr(',' || ?4 || ',', ',' || p.kind || ',') > 0)
           AND (?5 IS NULL
                OR (?5 = 'none' AND cu.id IS NULL)
                OR (?5 = 'any' AND cu.id IS NOT NULL)
                OR (?5 = 'display' AND cu.handle = ?6 AND ca.name = ?7)
                OR (?5 = 'handle' AND cu.handle = ?6)
                OR (?5 = 'human' AND cu.handle = ?6 AND p.claimed_by_agent_id IS NULL)
                OR (?5 = 'either' AND (cu.handle = ?6 OR ca.name = ?7)))
           AND (?8 IS NULL
                OR (?8 = 'none' AND 0)
                OR (?8 = 'any')
                OR (?8 = 'display' AND au.handle = ?9 AND aa.name = ?10)
                OR (?8 = 'handle' AND au.handle = ?9)
                OR (?8 = 'human' AND au.handle = ?9 AND p.author_agent_id IS NULL)
                OR (?8 = 'either' AND (au.handle = ?9 OR aa.name = ?10))
                OR (?8 = 'bare' AND ((au.handle = ?9 AND p.author_agent_id IS NULL)
                     OR (aa.name = ?10 AND NOT EXISTS (SELECT 1 FROM memberships m JOIN users mu ON mu.id = m.user_id
                                                       WHERE m.team_id = ?1 AND mu.handle = ?9)))))
           AND (?14 IS NULL OR p.author_user_id = ?14)
           AND (?11 IS NULL OR instr(lower(p.title), lower(?11)) > 0 OR instr(lower(p.body), lower(?11)) > 0)
           AND (?12 IS NULL OR p.number < ?12)
         ORDER BY p.number DESC
         LIMIT ?13"
    ))
    .bind(team_id)
    .bind(board)
    .bind(status)
    .bind(kind)
    .bind(c_mode)
    .bind(c_handle)
    .bind(c_name)
    .bind(a_mode)
    .bind(a_handle)
    .bind(a_name)
    .bind(q)
    .bind(filter.before)
    .bind(limit)
    .bind(author_user)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| r.into_post(Some(240))).collect())
}

/// Assigns the next per-team number atomically. Parses @mentions in the body and pings them.
pub async fn create_post(db: &SqlitePool, actor: &Actor, new: NewPost) -> AppResult<Post> {
    Ok(create_post_pinging(db, actor, new).await?.0)
}

/// `create_post`, also reporting who the body's @mentions reached.
pub async fn create_post_pinging(db: &SqlitePool, actor: &Actor, new: NewPost) -> AppResult<(Post, Mentioned)> {
    let title = validate_title(&new.title)?;
    validate_body(&new.body)?;
    let kind = validate_kind(new.kind.as_deref().unwrap_or("note"))?;
    let status = validate_status(new.status.as_deref().unwrap_or("open"))?;
    let priority = validate_priority(new.priority.unwrap_or(0))?;
    let board_id = board_id_by_key(db, &actor.team_id, &new.board).await?;
    let id = new_id("pst");
    let now = now_ms();

    let mut tx = db.begin().await?;
    // Write first so the transaction takes the write lock immediately (no upgrade deadlocks).
    let number: Option<i64> = sqlx::query_scalar(
        "UPDATE teams SET next_post_number = next_post_number + 1 WHERE id = ?1 RETURNING next_post_number - 1",
    )
    .bind(actor.team_id.as_str())
    .fetch_optional(&mut *tx)
    .await?;
    let number = number.ok_or_else(|| AppError::NotFound("no such team".into()))?;
    sqlx::query(
        "INSERT INTO posts (id, team_id, board_id, number, kind, status, priority, title, body,
                            author_user_id, author_agent_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
    )
    .bind(id.as_str())
    .bind(actor.team_id.as_str())
    .bind(board_id.as_str())
    .bind(number)
    .bind(kind.as_str())
    .bind(status.as_str())
    .bind(priority)
    .bind(title.as_str())
    .bind(new.body.as_str())
    .bind(actor.user_id.as_str())
    .bind(actor.agent_id.as_deref())
    .bind(now)
    .execute(&mut *tx)
    .await?;
    let board_key: String = sqlx::query_scalar("SELECT key FROM boards WHERE id = ?1")
        .bind(board_id.as_str())
        .fetch_one(&mut *tx)
        .await?;
    record_event(&mut *tx, actor, "post_created", Some(&id), json!({ "board": board_key, "kind": kind, "title": title }))
        .await?;
    tx.commit().await?;

    let mentioned = ping_mentions(db, actor, &new.body, &id, number, &title, &HashSet::new()).await?;
    Ok((require_post(db, &actor.team_id, number).await?, mentioned))
}

pub async fn get_post(db: &SqlitePool, team_id: &str, number: i64) -> AppResult<PostDetail> {
    let post = require_post(db, team_id, number).await?;
    let comments = comments_for(db, &post.id, number).await?;
    Ok(PostDetail { post, comments })
}

#[derive(FromRow)]
struct CommentRow {
    id: String,
    body: String,
    created_at: i64,
    author_user_id: String,
    author_handle: String,
    author_agent_id: Option<String>,
    author_agent_name: Option<String>,
}

impl CommentRow {
    fn into_comment(self, post_number: i64) -> Comment {
        Comment {
            id: self.id,
            post_number,
            body: self.body,
            author: author(self.author_user_id, self.author_handle, self.author_agent_id, self.author_agent_name),
            created_at: self.created_at,
        }
    }
}

macro_rules! comment_select {
    () => {
        "SELECT c.id, c.body, c.created_at, c.author_user_id, u.handle AS author_handle,
                c.author_agent_id, a.name AS author_agent_name
         FROM comments c
         JOIN users u ON u.id = c.author_user_id
         LEFT JOIN agents a ON a.id = c.author_agent_id "
    };
}

async fn comments_for(db: &SqlitePool, post_id: &str, number: i64) -> AppResult<Vec<Comment>> {
    let rows: Vec<CommentRow> = sqlx::query_as(concat!(comment_select!(), "WHERE c.post_id = ?1 ORDER BY c.created_at, c.rowid"))
        .bind(post_id)
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(|r| r.into_comment(number)).collect())
}

pub async fn update_post(db: &SqlitePool, actor: &Actor, number: i64, patch: PostPatch) -> AppResult<Post> {
    Ok(update_post_releasing(db, actor, number, patch).await?.0)
}

/// `update_post`; setting done/closed also releases the post's "#n" lease (granting it to the
/// next waiter), which is returned.
pub async fn update_post_releasing(
    db: &SqlitePool,
    actor: &Actor,
    number: i64,
    patch: PostPatch,
) -> AppResult<(Post, Option<leases::TaskLeaseFreed>)> {
    let finished = patch.status.as_deref().and_then(|s| validate_status(s).ok()).filter(|s| s == "done" || s == "closed");
    let post = apply_post_patch(db, actor, number, patch).await?;
    let freed = match finished {
        Some(status) => leases::release_task_lease(db, actor, number, &status).await?,
        None => None,
    };
    Ok((post, freed))
}

async fn apply_post_patch(db: &SqlitePool, actor: &Actor, number: i64, patch: PostPatch) -> AppResult<Post> {
    let before = require_post(db, &actor.team_id, number).await?;
    let title = patch.title.as_deref().map(validate_title).transpose()?;
    if let Some(b) = &patch.body {
        validate_body(b)?;
    }
    let status = patch.status.as_deref().map(validate_status).transpose()?;
    let kind = patch.kind.as_deref().map(validate_kind).transpose()?;
    let priority = patch.priority.map(validate_priority).transpose()?;
    let board_id = match patch.board.as_deref() {
        Some(k) => Some(board_id_by_key(db, &actor.team_id, k).await?),
        None => None,
    };

    let mut changed: Vec<&str> = Vec::new();
    if title.as_deref().is_some_and(|t| t != before.title) {
        changed.push("title");
    }
    if patch.body.as_deref().is_some_and(|b| b != before.body) {
        changed.push("body");
    }
    if kind.as_deref().is_some_and(|k| k != before.kind) {
        changed.push("kind");
    }
    if priority.is_some_and(|p| p != before.priority) {
        changed.push("priority");
    }
    if let (Some(k), Some(_)) = (patch.board.as_deref(), &board_id)
        && k.trim().to_lowercase() != before.board_key
    {
        changed.push("board");
    }
    let status_changed = status.as_deref().is_some_and(|s| s != before.status);
    if changed.is_empty() && !status_changed {
        return Ok(before);
    }

    sqlx::query(
        "UPDATE posts SET
           title = COALESCE(?3, title),
           body = COALESCE(?4, body),
           status = COALESCE(?5, status),
           kind = COALESCE(?6, kind),
           priority = COALESCE(?7, priority),
           board_id = COALESCE(?8, board_id),
           claimed_by_agent_id = CASE WHEN ?5 IN ('done','closed') THEN NULL ELSE claimed_by_agent_id END,
           claimed_by_user_id = CASE WHEN ?5 IN ('done','closed') THEN NULL ELSE claimed_by_user_id END,
           claimed_at = CASE WHEN ?5 IN ('done','closed') THEN NULL ELSE claimed_at END,
           updated_at = ?9
         WHERE team_id = ?1 AND number = ?2",
    )
    .bind(actor.team_id.as_str())
    .bind(number)
    .bind(title.as_deref())
    .bind(patch.body.as_deref())
    .bind(status.as_deref())
    .bind(kind.as_deref())
    .bind(priority)
    .bind(board_id.as_deref())
    .bind(now_ms())
    .execute(db)
    .await?;

    if status_changed {
        record_event(
            db,
            actor,
            "status_changed",
            Some(&before.id),
            json!({ "from": before.status, "to": status.as_deref().unwrap_or_default() }),
        )
        .await?;
    }
    if !changed.is_empty() {
        record_event(db, actor, "post_updated", Some(&before.id), json!({ "fields": changed })).await?;
    }
    if let Some(body) = &patch.body {
        let already: HashSet<String> = parse_mentions(&before.body).into_iter().collect();
        let t = title.as_deref().unwrap_or(&before.title).to_string();
        ping_mentions(db, actor, body, &before.id, number, &t, &already).await?;
    }
    require_post(db, &actor.team_id, number).await
}

/// Atomic claim. Conflict (with the holder's display name) if another agent/human holds it,
/// unless `force`. Sets status in_progress. Works for agents (actor.agent_id) and humans.
pub async fn claim_post(db: &SqlitePool, actor: &Actor, number: i64, force: bool) -> AppResult<Post> {
    let before = require_post(db, &actor.team_id, number).await?;
    let now = now_ms();
    let res = sqlx::query(
        "UPDATE posts SET
           claimed_at = CASE WHEN claimed_by_agent_id IS ?3 AND claimed_by_user_id IS ?4 THEN COALESCE(claimed_at, ?5) ELSE ?5 END,
           claimed_by_agent_id = ?3,
           claimed_by_user_id = ?4,
           status = 'in_progress',
           updated_at = ?5
         WHERE team_id = ?1 AND number = ?2
           AND (?6
                OR (claimed_by_agent_id IS NULL AND claimed_by_user_id IS NULL)
                OR (claimed_by_agent_id IS ?3 AND claimed_by_user_id IS ?4))",
    )
    .bind(actor.team_id.as_str())
    .bind(number)
    .bind(actor.agent_id.as_deref())
    .bind(actor.user_id.as_str())
    .bind(now)
    .bind(force)
    .execute(db)
    .await?;

    if res.rows_affected() == 0 {
        let current = require_post(db, &actor.team_id, number).await?;
        let (who, since) = match &current.claimed_by {
            Some(h) => (h.display.clone(), current.claimed_at.map(|t| ago(t, now)).unwrap_or_default()),
            None => ("someone".into(), String::new()),
        };
        return Err(AppError::Conflict(format!("#{number} is already claimed by {who} ({since})")));
    }

    let was_mine = before
        .claimed_by
        .as_ref()
        .is_some_and(|h| h.agent_id == actor.agent_id && h.user_id == actor.user_id);
    if !was_mine || before.status != "in_progress" {
        let took_from = before.claimed_by.as_ref().filter(|_| !was_mine).map(|h| h.display.clone());
        record_event(db, actor, "claimed", Some(&before.id), json!({ "force": force, "from": took_from })).await?;
        if let Some(prev) = before.claimed_by.as_ref().filter(|_| !was_mine) {
            // Tell the previous holder they lost it.
            let recipient = match &prev.agent_id {
                Some(aid) => Recipient::Agent { id: aid.clone(), display: prev.display.clone() },
                None => Recipient::Human { user_id: prev.user_id.clone(), display: prev.handle.clone() },
            };
            let me = actor_display(db, actor).await?;
            let body = format!("{me} took over #{number} \"{}\" from you (force claim). Stop work on it.", before.title);
            deliver_ping(db, actor, &prev.display, &body, Some(&before.id), &[recipient], false).await?;
        }
    }
    require_post(db, &actor.team_id, number).await
}

/// Clears the claim. `status` defaults to "open" (or pass "done"/"blocked"/…).
pub async fn release_post(db: &SqlitePool, actor: &Actor, number: i64, status: Option<&str>) -> AppResult<Post> {
    let status = validate_status(status.unwrap_or("open"))?;
    let before = require_post(db, &actor.team_id, number).await?;
    if let Some(holder) = &before.claimed_by {
        let mine = holder.agent_id == actor.agent_id && holder.user_id == actor.user_id;
        // Humans in the dashboard may release anything (e.g. a stuck agent's claim).
        if !mine && actor.agent_id.is_some() {
            return Err(AppError::Conflict(format!(
                "#{number} is claimed by {who}, not you; send(to:\"{who}\") to coordinate, or if they've been inactive \
                 {}+ minutes, claim(scope:\"#{number}\", force:true) to take it over",
                leases::FORCE_IDLE_MS / 60_000,
                who = holder.display
            )));
        }
    }
    let res = sqlx::query(
        "UPDATE posts SET claimed_by_agent_id = NULL, claimed_by_user_id = NULL, claimed_at = NULL,
                          status = ?3, updated_at = ?4
         WHERE team_id = ?1 AND number = ?2 AND claimed_by_agent_id IS ?5 AND claimed_at IS ?6",
    )
    .bind(actor.team_id.as_str())
    .bind(number)
    .bind(status.as_str())
    .bind(now_ms())
    .bind(before.claimed_by.as_ref().and_then(|h| h.agent_id.as_deref()))
    .bind(before.claimed_at)
    .execute(db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::Conflict(format!("#{number} changed while releasing; check it and retry")));
    }
    if let Some(holder) = &before.claimed_by {
        record_event(db, actor, "released", Some(&before.id), json!({ "status": status, "was": holder.display })).await?;
    } else if status != before.status {
        record_event(db, actor, "status_changed", Some(&before.id), json!({ "from": before.status, "to": status })).await?;
    }
    require_post(db, &actor.team_id, number).await
}

/// Parses @mentions in the body and pings them.
pub async fn add_comment(db: &SqlitePool, actor: &Actor, number: i64, body: &str) -> AppResult<Comment> {
    Ok(add_comment_pinging(db, actor, number, body).await?.0)
}

async fn add_comment_pinging(db: &SqlitePool, actor: &Actor, number: i64, body: &str) -> AppResult<(Comment, Mentioned)> {
    let body = body.trim();
    if body.is_empty() {
        return Err(bad("comment body is required"));
    }
    if body.chars().count() > MAX_COMMENT {
        return Err(bad(format!("comment is too long (max {MAX_COMMENT} chars)")));
    }
    let post = require_post(db, &actor.team_id, number).await?;
    let id = new_id("cmt");
    let now = now_ms();
    sqlx::query(
        "INSERT INTO comments (id, team_id, post_id, body, author_user_id, author_agent_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(id.as_str())
    .bind(actor.team_id.as_str())
    .bind(post.id.as_str())
    .bind(body)
    .bind(actor.user_id.as_str())
    .bind(actor.agent_id.as_deref())
    .bind(now)
    .execute(db)
    .await?;
    sqlx::query("UPDATE posts SET updated_at = ?2 WHERE id = ?1").bind(post.id.as_str()).bind(now).execute(db).await?;
    record_event(db, actor, "commented", Some(&post.id), json!({ "comment_id": id, "excerpt": excerpt(body, 140) })).await?;
    let mentioned = ping_mentions(db, actor, body, &post.id, number, &post.title, &HashSet::new()).await?;
    let row: CommentRow = sqlx::query_as(concat!(comment_select!(), "WHERE c.id = ?1")).bind(id.as_str()).fetch_one(db).await?;
    Ok((row.into_comment(number), mentioned))
}

/// Comment on a post and ping its author and claimer (unless they wrote it or were @mentioned).
/// Returns the comment and everyone it pinged (@mentions, author, claimer).
pub async fn reply(db: &SqlitePool, actor: &Actor, number: i64, body: &str) -> AppResult<(Comment, Mentioned)> {
    let (c, mut pinged) = add_comment_pinging(db, actor, number, body).await?;
    let post = require_post(db, &actor.team_id, number).await?;
    let mentioned: HashSet<String> = parse_mentions(&c.body).into_iter().collect();
    let mut recips: Vec<Recipient> = Vec::new();
    for who in [Some(&post.author), post.claimed_by.as_ref()].into_iter().flatten() {
        let r = match &who.agent_id {
            Some(id) => Recipient::Agent { id: id.clone(), display: who.display.clone() },
            None => Recipient::Human { user_id: who.user_id.clone(), display: who.handle.clone() },
        };
        let key = who.display.to_lowercase();
        if r.is_actor(actor) || mentioned.contains(&key) || recips.iter().any(|x| x.key() == r.key()) {
            continue;
        }
        recips.push(r);
    }
    if recips.is_empty() {
        return Ok((c, pinged));
    }
    let me = actor_display(db, actor).await?;
    let text = format!("{me} replied on #{number} \"{}\": {}", truncate_chars(&post.title, 80), excerpt(&c.body, 400));
    let target = recips.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join(",");
    let sent = deliver_ping(db, actor, &target, &text, Some(&post.id), &recips, false).await?;
    for r in sent.recipients {
        if !pinged.recipients.contains(&r) {
            pinged.recipients.push(r);
        }
    }
    Ok((c, pinged))
}

// ---------------------------------------------------------------------------------------------
// Mentions + pings
// ---------------------------------------------------------------------------------------------

/// Extract `@handle`, `@handle/agent` and `@handle/*` mentions (lowercased, deduped, in order).
/// Ignores emails (`a@b.c`), text inside backticks, and `@all` (broadcast only via the ping tool).
pub fn parse_mentions(body: &str) -> Vec<String> {
    let chars: Vec<char> = body.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut in_code = false;
    let is_name = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            in_code = !in_code;
            i += 1;
            continue;
        }
        if c != '@' || in_code {
            i += 1;
            continue;
        }
        let prev_ok = i == 0 || {
            let p = chars[i - 1];
            !(p.is_alphanumeric() || matches!(p, '.' | '_' | '-' | '/' | '@' | '+'))
        };
        i += 1;
        if !prev_ok {
            continue;
        }
        let start = i;
        while i < chars.len() && is_name(chars[i]) {
            i += 1;
        }
        let handle: String = chars[start..i].iter().collect::<String>().trim_end_matches(['-', '_']).to_lowercase();
        if handle.is_empty() {
            continue;
        }
        let mut token = handle.clone();
        if i + 1 < chars.len() && chars[i] == '/' {
            if chars[i + 1] == '*' {
                token = format!("{handle}/*");
                i += 2;
            } else if is_name(chars[i + 1]) {
                let s2 = i + 1;
                let mut j = s2;
                while j < chars.len() && is_name(chars[j]) {
                    j += 1;
                }
                let name: String = chars[s2..j].iter().collect::<String>().trim_end_matches(['-', '_']).to_lowercase();
                if !name.is_empty() {
                    token = format!("{handle}/{name}");
                }
                i = j;
            }
        }
        if matches!(handle.as_str(), "all" | "everyone" | "here") {
            continue;
        }
        if !out.contains(&token) {
            out.push(token);
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Recipient {
    Agent { id: String, display: String },
    Human { user_id: String, display: String },
}

impl Recipient {
    pub fn display(&self) -> &str {
        match self {
            Recipient::Agent { display, .. } | Recipient::Human { display, .. } => display,
        }
    }
    fn key(&self) -> String {
        match self {
            Recipient::Agent { id, .. } => format!("a:{id}"),
            Recipient::Human { user_id, .. } => format!("u:{user_id}"),
        }
    }
    fn is_actor(&self, actor: &Actor) -> bool {
        match self {
            Recipient::Agent { id, .. } => actor.agent_id.as_deref() == Some(id.as_str()),
            Recipient::Human { user_id, .. } => actor.agent_id.is_none() && *user_id == actor.user_id,
        }
    }
}

async fn member_by_handle(db: &SqlitePool, team_id: &str, handle: &str) -> AppResult<Option<(String, String)>> {
    Ok(sqlx::query_as(
        "SELECT u.id, u.handle FROM users u JOIN memberships m ON m.user_id = u.id
         WHERE m.team_id = ?1 AND u.handle = ?2",
    )
    .bind(team_id)
    .bind(handle)
    .fetch_optional(db)
    .await?)
}

/// (id, name, handle, last_seen_at)
type AgentLite = (String, String, String, i64);

async fn agents_named(db: &SqlitePool, team_id: &str, name: &str) -> AppResult<Vec<AgentLite>> {
    Ok(sqlx::query_as(
        "SELECT a.id, a.name, u.handle, a.last_seen_at FROM agents a
         JOIN users u ON u.id = a.user_id
         JOIN memberships m ON m.team_id = a.team_id AND m.user_id = a.user_id
         WHERE a.team_id = ?1 AND a.name = ?2 ORDER BY a.last_seen_at DESC",
    )
    .bind(team_id)
    .bind(name)
    .fetch_all(db)
    .await?)
}

async fn agents_of_user(db: &SqlitePool, team_id: &str, user_id: &str, active_since: i64) -> AppResult<Vec<AgentLite>> {
    Ok(sqlx::query_as(
        "SELECT a.id, a.name, u.handle, a.last_seen_at FROM agents a JOIN users u ON u.id = a.user_id
         WHERE a.team_id = ?1 AND a.user_id = ?2 AND a.last_seen_at > ?3 AND EXISTS (SELECT 1 FROM mcp_sessions s WHERE s.agent_id = a.id AND s.closed_at IS NULL) ORDER BY a.last_seen_at DESC",
    )
    .bind(team_id)
    .bind(user_id)
    .bind(active_since)
    .fetch_all(db)
    .await?)
}

fn agent_recipient(a: &AgentLite) -> Recipient {
    Recipient::Agent { id: a.0.clone(), display: format!("{}/{}", a.2, a.1) }
}

/// Resolve a ping target to recipients (sender excluded). Errors explain how to address correctly.
pub async fn resolve_target(db: &SqlitePool, actor: &Actor, raw: &str) -> AppResult<Vec<Recipient>> {
    let t = raw.trim().to_lowercase();
    if t.is_empty() {
        return Err(bad("`to` is required: \"handle/agent\", \"agent\", \"@handle\", \"handle/*\" or \"@all\""));
    }
    let now = now_ms();
    let active_since = now - AGENT_ACTIVE_MS;
    let mut out: Vec<Recipient> = Vec::new();

    if matches!(t.as_str(), "@all" | "*" | "@*" | "@everyone" | "@here") {
        let agents: Vec<AgentLite> = sqlx::query_as(
            "SELECT a.id, a.name, u.handle, a.last_seen_at FROM agents a
             JOIN users u ON u.id = a.user_id
             JOIN memberships m ON m.team_id = a.team_id AND m.user_id = a.user_id
             WHERE a.team_id = ?1 AND a.last_seen_at > ?2 AND EXISTS (SELECT 1 FROM mcp_sessions s WHERE s.agent_id = a.id AND s.closed_at IS NULL) ORDER BY u.handle, a.name",
        )
        .bind(actor.team_id.as_str())
        .bind(active_since)
        .fetch_all(db)
        .await?;
        out.extend(agents.iter().map(agent_recipient));
        let humans: Vec<(String, String)> = sqlx::query_as(
            "SELECT u.id, u.handle FROM memberships m JOIN users u ON u.id = m.user_id WHERE m.team_id = ?1 ORDER BY u.handle",
        )
        .bind(actor.team_id.as_str())
        .fetch_all(db)
        .await?;
        out.extend(humans.into_iter().map(|(id, h)| Recipient::Human { user_id: id, display: h }));
    } else {
        let had_at = t.starts_with('@');
        let s = t.trim_start_matches('@');
        if let Some((h, n)) = s.split_once('/') {
            let Some((uid, handle)) = member_by_handle(db, &actor.team_id, h).await? else {
                return Err(AppError::NotFound(format!("no teammate \"{h}\" on this team")));
            };
            if n == "*" || n.is_empty() {
                let agents = agents_of_user(db, &actor.team_id, &uid, active_since).await?;
                out.extend(agents.iter().map(agent_recipient));
                out.retain(|r| !r.is_actor(actor));
                if out.is_empty() {
                    return Err(AppError::NotFound(format!(
                        "{handle} has no other active agents; ping \"@{handle}\" to reach the human"
                    )));
                }
            } else {
                let name = slugify(n);
                let agent: Option<(String, String)> = sqlx::query_as(
                    "SELECT a.id, a.name FROM agents a WHERE a.team_id = ?1 AND a.user_id = ?2 AND a.name = ?3",
                )
                .bind(actor.team_id.as_str())
                .bind(uid.as_str())
                .bind(name.as_str())
                .fetch_optional(db)
                .await?;
                match agent {
                    Some((id, n)) => out.push(Recipient::Agent { id, display: format!("{handle}/{n}") }),
                    None => {
                        let names: Vec<String> = sqlx::query_scalar(
                            "SELECT name FROM agents WHERE team_id = ?1 AND user_id = ?2 ORDER BY last_seen_at DESC LIMIT 10",
                        )
                        .bind(actor.team_id.as_str())
                        .bind(uid.as_str())
                        .fetch_all(db)
                        .await?;
                        let hint = if names.is_empty() {
                            format!("{handle} has no agents yet; ping \"@{handle}\" for the human")
                        } else {
                            format!("{handle}'s agents: {}", names.join(", "))
                        };
                        return Err(AppError::NotFound(format!("no agent {handle}/{name} ({hint})")));
                    }
                }
            }
        } else if had_at {
            match member_by_handle(db, &actor.team_id, s).await? {
                Some((uid, handle)) => out.push(Recipient::Human { user_id: uid, display: handle }),
                None => {
                    // `@ui-fixer` written for an agent: accept if the agent name is unique.
                    let agents = agents_named(db, &actor.team_id, &slugify(s)).await?;
                    if agents.len() == 1 {
                        out.push(agent_recipient(&agents[0]));
                    } else {
                        return Err(AppError::NotFound(format!("no teammate \"@{s}\" on this team")));
                    }
                }
            }
        } else {
            let agents = agents_named(db, &actor.team_id, &slugify(s)).await?;
            let agents: Vec<AgentLite> =
                agents.into_iter().filter(|a| actor.agent_id.as_deref() != Some(a.0.as_str())).collect();
            match agents.len() {
                1 => out.push(agent_recipient(&agents[0])),
                0 => match member_by_handle(db, &actor.team_id, s).await? {
                    Some((uid, handle)) => out.push(Recipient::Human { user_id: uid, display: handle }),
                    None => {
                        let active: Vec<String> = sqlx::query_scalar(
                            "SELECT u.handle || '/' || a.name FROM agents a JOIN users u ON u.id = a.user_id
                             WHERE a.team_id = ?1 AND a.last_seen_at > ?2 AND EXISTS (SELECT 1 FROM mcp_sessions s WHERE s.agent_id = a.id AND s.closed_at IS NULL) ORDER BY a.last_seen_at DESC LIMIT 15",
                        )
                        .bind(actor.team_id.as_str())
                        .bind(active_since)
                        .fetch_all(db)
                        .await?;
                        let hint = if active.is_empty() { "no agents are active".to_string() } else { format!("active: {}", active.join(", ")) };
                        return Err(AppError::NotFound(format!("no agent or teammate named \"{s}\" ({hint})")));
                    }
                },
                _ => {
                    let opts: Vec<String> = agents.iter().map(|a| format!("{}/{}", a.2, a.1)).collect();
                    return Err(bad(format!("\"{s}\" is ambiguous: {}. Use handle/agent", opts.join(", "))));
                }
            }
        }
    }

    let mut seen = HashSet::new();
    let had_any = !out.is_empty();
    out.retain(|r| !r.is_actor(actor) && seen.insert(r.key()));
    if out.is_empty() {
        return Err(if had_any { bad("that's you") } else { AppError::NotFound("nobody to ping (no active agents)".into()) });
    }
    Ok(out)
}

/// Insert the ping and one inbox row per recipient.
async fn deliver_ping(
    db: &SqlitePool,
    actor: &Actor,
    target: &str,
    body: &str,
    post_id: Option<&str>,
    recipients: &[Recipient],
    log_event: bool,
) -> AppResult<PingSent> {
    let id = new_id("png");
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO pings (id, team_id, from_user_id, from_agent_id, target, post_id, body, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(id.as_str())
    .bind(actor.team_id.as_str())
    .bind(actor.user_id.as_str())
    .bind(actor.agent_id.as_deref())
    .bind(target)
    .bind(post_id)
    .bind(body)
    .bind(now_ms())
    .execute(&mut *tx)
    .await?;
    for r in recipients {
        let (agent_id, user_id) = match r {
            Recipient::Agent { id, .. } => (Some(id.as_str()), None),
            Recipient::Human { user_id, .. } => (None, Some(user_id.as_str())),
        };
        sqlx::query("INSERT INTO inbox (ping_id, team_id, agent_id, user_id) VALUES (?1, ?2, ?3, ?4)")
            .bind(id.as_str())
            .bind(actor.team_id.as_str())
            .bind(agent_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }
    let recipients: Vec<String> = recipients.iter().map(|r| r.display().to_string()).collect();
    if log_event {
        record_event(
            &mut *tx,
            actor,
            "pinged",
            post_id,
            json!({ "target": target, "recipients": recipients, "excerpt": excerpt(body, 140) }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(PingSent { ping_id: id, recipients })
}

/// Who a post/comment pinged: recipient displays ("paul", "bob/ui"), plus @mentions that matched
/// nobody (e.g. `@types/node` in prose; tokens without the @).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mentioned {
    pub recipients: Vec<String>,
    pub unknown: Vec<String>,
}

/// Ping everyone mentioned in `body` (except those already mentioned before, in `skip`).
async fn ping_mentions(
    db: &SqlitePool,
    actor: &Actor,
    body: &str,
    post_id: &str,
    number: i64,
    title: &str,
    skip: &HashSet<String>,
) -> AppResult<Mentioned> {
    let mut out = Mentioned::default();
    let mentions: Vec<String> =
        parse_mentions(body).into_iter().filter(|m| !skip.contains(m)).take(MAX_MENTIONS).collect();
    if mentions.is_empty() {
        return Ok(out);
    }
    let mut done: HashSet<String> = HashSet::new();
    let text = format!("mentioned you on #{number} \"{}\": {}", truncate_chars(title, 80), excerpt(body, 280));
    for m in mentions {
        // Unknown handles are skipped (and reported); mentioning yourself is just skipped.
        let recips = match resolve_target(db, actor, &format!("@{m}")).await {
            Ok(r) => r,
            Err(AppError::NotFound(_)) => {
                out.unknown.push(m);
                continue;
            }
            Err(_) => continue,
        };
        let recips: Vec<Recipient> = recips.into_iter().filter(|r| done.insert(r.key())).collect();
        if recips.is_empty() {
            continue;
        }
        let sent = deliver_ping(db, actor, &format!("@{m}"), &text, Some(post_id), &recips, false).await?;
        out.recipients.extend(sent.recipients);
    }
    Ok(out)
}

/// Targets: "name" / "handle/name" (one agent), "@handle" (that human's dashboard inbox),
/// "handle/*" (all that human's active agents), "@all" (every active agent + every human, minus sender).
pub async fn send_ping(db: &SqlitePool, actor: &Actor, target: &str, body: &str, post_number: Option<i64>) -> AppResult<PingSent> {
    let body = body.trim();
    if body.is_empty() {
        return Err(bad("message is required"));
    }
    if body.chars().count() > MAX_PING {
        return Err(bad(format!("message is too long (max {MAX_PING} chars)")));
    }
    let post_id = match post_number {
        Some(n) => Some(require_post(db, &actor.team_id, n).await?.id),
        None => None,
    };
    let recipients = resolve_target(db, actor, target).await?;
    deliver_ping(db, actor, target.trim(), body, post_id.as_deref(), &recipients, true).await
}

#[derive(FromRow)]
struct PingRow {
    id: String,
    target: String,
    body: String,
    created_at: i64,
    delivered_at: Option<i64>,
    read_at: Option<i64>,
    from_user_id: String,
    from_handle: String,
    from_agent_id: Option<String>,
    from_agent_name: Option<String>,
    post_number: Option<i64>,
}

impl From<PingRow> for Ping {
    fn from(r: PingRow) -> Self {
        Ping {
            id: r.id,
            from: author(r.from_user_id, r.from_handle, r.from_agent_id, r.from_agent_name),
            target: r.target,
            body: r.body,
            post_number: r.post_number,
            created_at: r.created_at,
            delivered_at: r.delivered_at,
            read_at: r.read_at,
        }
    }
}

macro_rules! ping_select {
    () => {
        "SELECT pg.id, pg.target, pg.body, pg.created_at, i.delivered_at, i.read_at,
                pg.from_user_id, u.handle AS from_handle, pg.from_agent_id, a.name AS from_agent_name,
                po.number AS post_number
         FROM inbox i
         JOIN pings pg ON pg.id = i.ping_id
         JOIN users u ON u.id = pg.from_user_id
         LEFT JOIN agents a ON a.id = pg.from_agent_id
         LEFT JOIN posts po ON po.id = pg.post_id "
    };
}

/// A human's dashboard inbox in a team, newest first.
pub async fn user_inbox(db: &SqlitePool, team_id: &str, user_id: &str, include_read: bool) -> AppResult<Vec<Ping>> {
    let rows: Vec<PingRow> = sqlx::query_as(concat!(
        ping_select!(),
        "WHERE i.team_id = ?1 AND i.user_id = ?2 AND (?3 OR i.read_at IS NULL) ORDER BY i.id DESC LIMIT 200"
    ))
    .bind(team_id)
    .bind(user_id)
    .bind(include_read)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(Ping::from).collect())
}

pub async fn mark_user_inbox_read(db: &SqlitePool, team_id: &str, user_id: &str) -> AppResult<()> {
    let now = now_ms();
    sqlx::query(
        "UPDATE inbox SET read_at = ?3, delivered_at = COALESCE(delivered_at, ?3)
         WHERE team_id = ?1 AND user_id = ?2 AND read_at IS NULL",
    )
    .bind(team_id)
    .bind(user_id)
    .bind(now)
    .execute(db)
    .await?;
    Ok(())
}

async fn pings_by_inbox_ids(db: &SqlitePool, ids: &[i64]) -> AppResult<Vec<Ping>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<PingRow> =
        sqlx::query_as(concat!(ping_select!(), "WHERE i.id IN (SELECT value FROM json_each(?1)) ORDER BY i.id"))
            .bind(serde_json::to_string(ids).unwrap_or_else(|_| "[]".into()))
            .fetch_all(db)
            .await?;
    Ok(rows.into_iter().map(Ping::from).collect())
}

/// Undelivered pings for an agent, atomically marked delivered (+read). Used by the tool-result
/// piggyback and the Claude Code hooks; each ping is handed out exactly once.
pub async fn take_agent_pings(db: &SqlitePool, agent_id: &str) -> AppResult<Vec<Ping>> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "UPDATE inbox SET delivered_at = ?2, read_at = COALESCE(read_at, ?2)
         WHERE agent_id = ?1 AND delivered_at IS NULL RETURNING id",
    )
    .bind(agent_id)
    .bind(now_ms())
    .fetch_all(db)
    .await?;
    pings_by_inbox_ids(db, &ids).await
}

/// The `inbox` tool: unread pings (marked read), or with `all` the 30 most recent.
pub async fn agent_inbox(db: &SqlitePool, agent_id: &str, all: bool) -> AppResult<Vec<Ping>> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "UPDATE inbox SET read_at = ?2, delivered_at = COALESCE(delivered_at, ?2)
         WHERE agent_id = ?1 AND read_at IS NULL RETURNING id",
    )
    .bind(agent_id)
    .bind(now_ms())
    .fetch_all(db)
    .await?;
    if !all {
        return pings_by_inbox_ids(db, &ids).await;
    }
    let rows: Vec<PingRow> = sqlx::query_as(concat!(ping_select!(), "WHERE i.agent_id = ?1 ORDER BY i.id DESC LIMIT 30"))
        .bind(agent_id)
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(Ping::from).collect())
}

// ---------------------------------------------------------------------------------------------
// Agents
// ---------------------------------------------------------------------------------------------

#[derive(FromRow)]
struct AgentListRow {
    id: String,
    name: String,
    user_id: String,
    handle: String,
    client: Option<String>,
    working_on: Option<String>,
    created_at: i64,
    last_seen_at: i64,
    active: bool,
    claimed: Option<String>,
    repo: Option<String>,
    branch: Option<String>,
    leases: Option<String>,
}

/// Agents in the team, most recently seen first. Active = seen within AGENT_ACTIVE_MS and still
/// holding an open MCP session (a client that sent DELETE /mcp is gone right away).
pub async fn list_agents(db: &SqlitePool, team_id: &str, active_only: bool) -> AppResult<Vec<Agent>> {
    query_agents(db, team_id, active_only, None, None).await
}

/// `list_agents`, optionally narrowed to one agent id and/or one owner.
async fn query_agents(
    db: &SqlitePool,
    team_id: &str,
    active_only: bool,
    agent_id: Option<&str>,
    user_id: Option<&str>,
) -> AppResult<Vec<Agent>> {
    let now = now_ms();
    let since = now - AGENT_ACTIVE_MS;
    let rows: Vec<AgentListRow> = sqlx::query_as(
        "SELECT a.id, a.name, a.user_id, u.handle, a.client, a.working_on, a.created_at, a.last_seen_at,
           (a.last_seen_at > ?3 AND EXISTS (SELECT 1 FROM mcp_sessions s WHERE s.agent_id = a.id AND s.closed_at IS NULL)) AS active,
           (SELECT group_concat(number) FROM
              (SELECT p.number FROM posts p WHERE p.claimed_by_agent_id = a.id AND p.status IN ('in_progress','blocked')
               ORDER BY p.number)) AS claimed,
           a.repo, a.branch,
           (SELECT group_concat(scope, char(31)) FROM (SELECT l.scope FROM leases l WHERE l.agent_id = a.id AND l.expires_at > ?4 ORDER BY l.created_at, l.rowid)) AS leases
         FROM agents a
         JOIN users u ON u.id = a.user_id
         JOIN memberships m ON m.team_id = a.team_id AND m.user_id = a.user_id
         WHERE a.team_id = ?1 AND (?2 = 0 OR (a.last_seen_at > ?3 AND EXISTS (SELECT 1 FROM mcp_sessions s WHERE s.agent_id = a.id AND s.closed_at IS NULL)))
           AND (?5 IS NULL OR a.id = ?5)
           AND (?6 IS NULL OR a.user_id = ?6)
         ORDER BY a.last_seen_at DESC
         LIMIT 200",
    )
    .bind(team_id)
    .bind(active_only)
    .bind(since)
    .bind(now)
    .bind(agent_id)
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Agent {
            display: format!("{}/{}", r.handle, r.name),
            id: r.id,
            name: r.name,
            user_id: r.user_id,
            owner_handle: r.handle,
            client: r.client,
            working_on: r.working_on,
            created_at: r.created_at,
            active: r.active,
            last_seen_at: r.last_seen_at,
            claimed: r
                .claimed
                .unwrap_or_default()
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect(),
            repo: r.repo,
            branch: r.branch,
            leases: r.leases.map(|l| l.split('\u{1f}').map(str::to_string).collect()).unwrap_or_default(),
        })
        .collect())
}

/// Persisted Streamable-HTTP MCP session.
#[derive(Debug, Clone, FromRow)]
#[allow(dead_code)] // token_id / client_version / protocol are kept for diagnostics
pub struct McpSession {
    pub id: String,
    pub team_id: String,
    pub user_id: String,
    pub token_id: Option<String>,
    pub agent_id: Option<String>,
    pub client_name: Option<String>,
    pub client_version: Option<String>,
    pub protocol: Option<String>,
    pub closed_at: Option<i64>,
}

/// A resolved agent identity.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRef {
    pub id: String,
    pub name: String,
    pub user_id: String,
    pub handle: String,
    pub display: String,
    pub working_on: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
}

#[derive(FromRow)]
struct AgentRow {
    id: String,
    user_id: String,
    name: String,
    handle: String,
    working_on: Option<String>,
    client_session_key: Option<String>,
    repo: Option<String>,
    branch: Option<String>,
}

impl From<AgentRow> for AgentRef {
    fn from(r: AgentRow) -> Self {
        AgentRef {
            display: format!("{}/{}", r.handle, r.name),
            id: r.id,
            name: r.name,
            user_id: r.user_id,
            handle: r.handle,
            working_on: r.working_on,
            repo: r.repo,
            branch: r.branch,
        }
    }
}

macro_rules! agent_select {
    () => {
        "SELECT a.id, a.user_id, a.name, u.handle, a.working_on, a.client_session_key, a.repo, a.branch
         FROM agents a JOIN users u ON u.id = a.user_id "
    };
}

pub async fn agent_by_id(db: &SqlitePool, agent_id: &str) -> AppResult<Option<AgentRef>> {
    let row: Option<AgentRow> =
        sqlx::query_as(concat!(agent_select!(), "WHERE a.id = ?1")).bind(agent_id).fetch_optional(db).await?;
    Ok(row.map(AgentRef::from))
}

/// The agent a Claude Code session key (hook `session_id`) was linked to via `hello(session=…)`.
pub async fn agent_by_session_key(db: &SqlitePool, team_id: &str, user_id: &str, key: &str) -> AppResult<Option<AgentRef>> {
    let row: Option<AgentRow> = sqlx::query_as(concat!(
        agent_select!(),
        "WHERE a.client_session_key = ?3 AND a.team_id = ?1 AND a.user_id = ?2 ORDER BY a.last_seen_at DESC LIMIT 1"
    ))
    .bind(team_id)
    .bind(user_id)
    .bind(key)
    .fetch_optional(db)
    .await?;
    Ok(row.map(AgentRef::from))
}

pub async fn touch_agent(db: &SqlitePool, agent_id: &str) -> AppResult<()> {
    sqlx::query("UPDATE agents SET last_seen_at = ?2 WHERE id = ?1").bind(agent_id).bind(now_ms()).execute(db).await?;
    Ok(())
}

pub async fn set_working_on(db: &SqlitePool, agent_id: &str, working_on: &str) -> AppResult<()> {
    let w = truncate_chars(working_on.trim(), 200);
    let w = if w.is_empty() { None } else { Some(w) };
    sqlx::query("UPDATE agents SET working_on = ?2, last_seen_at = ?3 WHERE id = ?1")
        .bind(agent_id)
        .bind(w)
        .bind(now_ms())
        .execute(db)
        .await?;
    Ok(())
}

/// Set where the agent works (repo is normalized; empty strings clear). None leaves a field as is.
pub async fn set_location(db: &SqlitePool, agent_id: &str, repo: Option<&str>, branch: Option<&str>) -> AppResult<()> {
    let repo = repo.map(|r| leases::normalize_repo(r).unwrap_or_default());
    let branch = branch.map(|b| truncate_chars(b.trim(), 100));
    sqlx::query(
        "UPDATE agents SET repo = CASE WHEN ?2 IS NULL THEN repo WHEN ?2 = '' THEN NULL ELSE ?2 END,
                           branch = CASE WHEN ?3 IS NULL THEN branch WHEN ?3 = '' THEN NULL ELSE ?3 END
         WHERE id = ?1",
    )
    .bind(agent_id)
    .bind(repo)
    .bind(branch)
    .execute(db)
    .await?;
    Ok(())
}

/// "claude-code" → "claude", "codex-mcp-client" → "codex", unknown → "agent".
pub fn client_short(client_name: Option<&str>) -> String {
    let raw = client_name.unwrap_or("").to_lowercase();
    let first = raw.split(|c: char| !c.is_ascii_alphanumeric()).find(|s| !s.is_empty()).unwrap_or("");
    let s: String = first.chars().take(12).collect();
    if s.is_empty() { "agent".to_string() } else { s }
}

pub async fn create_mcp_session(
    db: &SqlitePool,
    team_id: &str,
    user_id: &str,
    token_id: &str,
    client_name: Option<&str>,
    client_version: Option<&str>,
    protocol: &str,
) -> AppResult<String> {
    let id = format!("mcp_{}", random_token(32));
    let now = now_ms();
    sqlx::query(
        "INSERT INTO mcp_sessions (id, team_id, user_id, token_id, client_name, client_version, protocol, created_at, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
    )
    .bind(id.as_str())
    .bind(team_id)
    .bind(user_id)
    .bind(token_id)
    .bind(client_name.map(|s| truncate_chars(s, 80)))
    .bind(client_version.map(|s| truncate_chars(s, 40)))
    .bind(protocol)
    .bind(now)
    .execute(db)
    .await?;
    Ok(id)
}

pub async fn get_mcp_session(db: &SqlitePool, id: &str) -> AppResult<Option<McpSession>> {
    Ok(sqlx::query_as(
        "SELECT id, team_id, user_id, token_id, agent_id, client_name, client_version, protocol, closed_at
         FROM mcp_sessions WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?)
}

pub async fn touch_mcp_session(db: &SqlitePool, id: &str) -> AppResult<()> {
    sqlx::query("UPDATE mcp_sessions SET last_seen_at = ?2 WHERE id = ?1").bind(id).bind(now_ms()).execute(db).await?;
    Ok(())
}

pub async fn close_mcp_session(db: &SqlitePool, id: &str) -> AppResult<()> {
    sqlx::query("UPDATE mcp_sessions SET closed_at = COALESCE(closed_at, ?2) WHERE id = ?1")
        .bind(id)
        .bind(now_ms())
        .execute(db)
        .await?;
    Ok(())
}

/// Close every open MCP session of an agent (its client exited), so it shows inactive and its
/// name can be resumed by the next session right away.
pub async fn close_agent_sessions(db: &SqlitePool, agent_id: &str) -> AppResult<u64> {
    let r = sqlx::query("UPDATE mcp_sessions SET closed_at = ?2 WHERE agent_id = ?1 AND closed_at IS NULL")
        .bind(agent_id)
        .bind(now_ms())
        .execute(db)
        .await?;
    Ok(r.rows_affected())
}

async fn insert_agent(db: &SqlitePool, sess: &McpSession, name: &str) -> Result<String, sqlx::Error> {
    let id = new_id("agt");
    let now = now_ms();
    sqlx::query(
        "INSERT INTO agents (id, team_id, user_id, name, client, created_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
    )
    .bind(id.as_str())
    .bind(sess.team_id.as_str())
    .bind(sess.user_id.as_str())
    .bind(name)
    .bind(sess.client_name.as_deref())
    .bind(now)
    .execute(db)
    .await?;
    Ok(id)
}

/// The session's agent, creating a provisional `<client>-<4 chars>` one on first use.
pub async fn ensure_session_agent(db: &SqlitePool, sess: &mut McpSession) -> AppResult<AgentRef> {
    if let Some(id) = &sess.agent_id
        && let Some(a) = agent_by_id(db, id).await?
    {
        return Ok(a);
    }
    let short = client_short(sess.client_name.as_deref());
    let mut created = None;
    for _ in 0..8 {
        let name = format!("{short}-{}", random_token(4).to_lowercase());
        match insert_agent(db, sess, &name).await {
            Ok(id) => {
                created = Some(id);
                break;
            }
            Err(e) if is_unique_violation(&e) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    let id = created.ok_or_else(|| AppError::Internal(anyhow::anyhow!("could not allocate agent name")))?;
    // Concurrent first calls in one session: only one provisional agent wins.
    let res = sqlx::query("UPDATE mcp_sessions SET agent_id = ?2 WHERE id = ?1 AND agent_id IS NULL")
        .bind(sess.id.as_str())
        .bind(id.as_str())
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        sqlx::query("DELETE FROM agents WHERE id = ?1").bind(id.as_str()).execute(db).await?;
        if let Some(fresh) = get_mcp_session(db, &sess.id).await? {
            *sess = fresh;
        }
        if let Some(aid) = &sess.agent_id
            && let Some(a) = agent_by_id(db, aid).await?
        {
            return Ok(a);
        }
        return Err(AppError::Internal(anyhow::anyhow!("session agent race")));
    }
    sess.agent_id = Some(id.clone());
    agent_by_id(db, &id).await?.ok_or_else(|| AppError::Internal(anyhow::anyhow!("agent vanished")))
}

/// What `hello` did.
#[derive(Debug, Clone)]
pub struct HelloOutcome {
    pub agent: AgentRef,
    /// Previous name of this session's agent, if it was renamed.
    pub renamed_from: Option<String>,
    /// True if an existing (inactive) agent with this name was taken over.
    pub adopted: bool,
    /// The requested name was busy (active in another session), so a suffix was added.
    pub requested: Option<String>,
}

/// A same-named agent idle this long is resumed by a new session instead of getting a suffix:
/// clients often exit without closing their MCP session, and a restarted agent must get its
/// leases and inbox back.
const RESUME_IDLE_MS: i64 = 90 * 1000;

/// True if another open MCP session is using this agent right now (called within RESUME_IDLE_MS).
async fn agent_held_elsewhere(db: &SqlitePool, agent_id: &str, my_session: &str) -> AppResult<bool> {
    let since = now_ms() - RESUME_IDLE_MS;
    let held: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM mcp_sessions s JOIN agents a ON a.id = s.agent_id
                       WHERE s.agent_id = ?1 AND s.id != ?2 AND s.closed_at IS NULL AND a.last_seen_at > ?3)",
    )
    .bind(agent_id)
    .bind(my_session)
    .bind(since)
    .fetch_one(db)
    .await?;
    Ok(held != 0)
}

async fn authored_anything(db: &SqlitePool, agent_id: &str) -> AppResult<bool> {
    let n: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM posts WHERE author_agent_id = ?1)
              + (SELECT COUNT(*) FROM comments WHERE author_agent_id = ?1)
              + (SELECT COUNT(*) FROM pings WHERE from_agent_id = ?1)
              + (SELECT COUNT(*) FROM files WHERE uploader_agent_id = ?1)
              + (SELECT COUNT(*) FROM boards WHERE created_by_agent = ?1)",
    )
    .bind(agent_id)
    .fetch_one(db)
    .await?;
    Ok(n > 0)
}

/// Name this session's agent. Rules: rename the session's agent; or adopt the same human's
/// INACTIVE agent of that name (inheriting inbox + claims; the provisional agent is deleted if it
/// authored nothing); if that name is ACTIVE in another session, suffix -2, -3, ….
/// `session_key` (Claude Code session_id) links the agent for hook delivery and also proves
/// "same conversation", so a resumed session reclaims its name even if it still looks active.
pub async fn hello(
    db: &SqlitePool,
    sess: &mut McpSession,
    name: &str,
    working_on: Option<&str>,
    session_key: Option<&str>,
) -> AppResult<HelloOutcome> {
    let base = normalize_agent_name(name)?;
    let session_key = session_key.map(str::trim).filter(|s| !s.is_empty()).map(|s| truncate_chars(s, 200));
    let current: Option<AgentRef> = match &sess.agent_id {
        Some(id) => agent_by_id(db, id).await?,
        None => None,
    };

    let mut outcome: Option<HelloOutcome> = None;
    for i in 1..=50 {
        let candidate = if i == 1 {
            base.clone()
        } else {
            let suffix = format!("-{i}");
            let keep = MAX_AGENT_NAME.saturating_sub(suffix.len());
            format!("{}{suffix}", base.chars().take(keep).collect::<String>().trim_end_matches('-'))
        };
        let requested = if i == 1 { None } else { Some(base.clone()) };
        if current.as_ref().is_some_and(|c| c.name == candidate) {
            outcome = Some(HelloOutcome { agent: current.clone().unwrap(), renamed_from: None, adopted: false, requested });
            break;
        }
        let existing: Option<AgentRow> = sqlx::query_as(concat!(
            agent_select!(),
            "WHERE a.team_id = ?1 AND a.user_id = ?2 AND a.name = ?3"
        ))
        .bind(sess.team_id.as_str())
        .bind(sess.user_id.as_str())
        .bind(candidate.as_str())
        .fetch_optional(db)
        .await?;
        match existing {
            None => {
                let id = match &current {
                    Some(c) => {
                        let r = sqlx::query("UPDATE agents SET name = ?2 WHERE id = ?1")
                            .bind(c.id.as_str())
                            .bind(candidate.as_str())
                            .execute(db)
                            .await;
                        match r {
                            Ok(_) => c.id.clone(),
                            Err(e) if is_unique_violation(&e) => continue,
                            Err(e) => return Err(e.into()),
                        }
                    }
                    None => match insert_agent(db, sess, &candidate).await {
                        Ok(id) => id,
                        Err(e) if is_unique_violation(&e) => continue,
                        Err(e) => return Err(e.into()),
                    },
                };
                sqlx::query("UPDATE mcp_sessions SET agent_id = ?2 WHERE id = ?1")
                    .bind(sess.id.as_str())
                    .bind(id.as_str())
                    .execute(db)
                    .await?;
                sess.agent_id = Some(id.clone());
                let agent = agent_by_id(db, &id).await?.ok_or_else(|| AppError::Internal(anyhow::anyhow!("agent vanished")))?;
                outcome = Some(HelloOutcome {
                    agent,
                    renamed_from: current.as_ref().map(|c| c.name.clone()),
                    adopted: false,
                    requested,
                });
                break;
            }
            Some(ex) => {
                let same_conversation = session_key.is_some() && ex.client_session_key == session_key;
                if !same_conversation && agent_held_elsewhere(db, &ex.id, &sess.id).await? {
                    continue;
                }
                adopt_agent(db, sess, &ex.id, current.as_ref()).await?;
                let agent = AgentRef::from(ex);
                outcome = Some(HelloOutcome { agent, renamed_from: None, adopted: true, requested });
                break;
            }
        }
    }
    let mut outcome = outcome.ok_or_else(|| bad("too many agents with that name; pick another"))?;

    let now = now_ms();
    sqlx::query("UPDATE agents SET client = COALESCE(?2, client), last_seen_at = ?3 WHERE id = ?1")
        .bind(outcome.agent.id.as_str())
        .bind(sess.client_name.as_deref())
        .bind(now)
        .execute(db)
        .await?;
    if let Some(w) = working_on.filter(|w| !w.trim().is_empty()) {
        set_working_on(db, &outcome.agent.id, w).await?;
    }
    if let Some(key) = &session_key {
        sqlx::query("UPDATE agents SET client_session_key = NULL WHERE team_id = ?1 AND user_id = ?2 AND client_session_key = ?3 AND id != ?4")
            .bind(sess.team_id.as_str())
            .bind(sess.user_id.as_str())
            .bind(key.as_str())
            .bind(outcome.agent.id.as_str())
            .execute(db)
            .await?;
        sqlx::query("UPDATE agents SET client_session_key = ?2 WHERE id = ?1")
            .bind(outcome.agent.id.as_str())
            .bind(key.as_str())
            .execute(db)
            .await?;
    }
    if let Some(fresh) = agent_by_id(db, &outcome.agent.id).await? {
        outcome.agent = fresh;
    }
    let changed = outcome.adopted || outcome.renamed_from.is_some() || current.is_none();
    if changed {
        let actor = Actor { team_id: sess.team_id.clone(), user_id: sess.user_id.clone(), agent_id: Some(outcome.agent.id.clone()) };
        record_event(
            db,
            &actor,
            "agent_joined",
            None,
            json!({
                "name": outcome.agent.name,
                "display": outcome.agent.display,
                "client": sess.client_name,
                "adopted": outcome.adopted,
                "renamed_from": outcome.renamed_from,
                "working_on": outcome.agent.working_on,
            }),
        )
        .await?;
    }
    Ok(outcome)
}

async fn adopt_agent(db: &SqlitePool, sess: &mut McpSession, target_id: &str, current: Option<&AgentRef>) -> AppResult<()> {
    sqlx::query("UPDATE mcp_sessions SET agent_id = ?2 WHERE id = ?1").bind(sess.id.as_str()).bind(target_id).execute(db).await?;
    // Stale sessions that still point at the adopted agent get a fresh provisional one if they return.
    sqlx::query("UPDATE mcp_sessions SET agent_id = NULL WHERE agent_id = ?1 AND id != ?2")
        .bind(target_id)
        .bind(sess.id.as_str())
        .execute(db)
        .await?;
    sess.agent_id = Some(target_id.to_string());
    if let Some(prov) = current.filter(|c| c.id != target_id) {
        // Same session, new identity: carry over its claims and unseen pings.
        sqlx::query("UPDATE posts SET claimed_by_agent_id = ?2 WHERE claimed_by_agent_id = ?1")
            .bind(prov.id.as_str())
            .bind(target_id)
            .execute(db)
            .await?;
        for q in ["UPDATE leases SET agent_id = ?2 WHERE agent_id = ?1", "UPDATE lease_waits SET agent_id = ?2 WHERE agent_id = ?1"] {
            sqlx::query(q).bind(prov.id.as_str()).bind(target_id).execute(db).await?;
        }
        sqlx::query("UPDATE inbox SET agent_id = ?2 WHERE agent_id = ?1 AND delivered_at IS NULL")
            .bind(prov.id.as_str())
            .bind(target_id)
            .execute(db)
            .await?;
        let still_used: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mcp_sessions WHERE agent_id = ?1")
            .bind(prov.id.as_str())
            .fetch_one(db)
            .await?;
        if still_used == 0 && !authored_anything(db, &prov.id).await? {
            sqlx::query("UPDATE events SET actor_agent_id = ?2 WHERE actor_agent_id = ?1")
                .bind(prov.id.as_str())
                .bind(target_id)
                .execute(db)
                .await?;
            sqlx::query("DELETE FROM agents WHERE id = ?1").bind(prov.id.as_str()).execute(db).await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------------------------

/// Keep `[A-Za-z0-9._-]`, max 100 chars, no leading dots. Empty → "file".
pub fn sanitize_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let mut out = String::new();
    for c in base.chars() {
        let c = if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    let out = out.trim_start_matches(['.', '-']).trim_end_matches('-');
    let out: String = if out.chars().count() > 100 {
        // keep the extension
        match out.rsplit_once('.') {
            Some((stem, ext)) if ext.len() <= 10 => format!("{}.{}", stem.chars().take(99 - ext.len()).collect::<String>(), ext),
            _ => out.chars().take(100).collect(),
        }
    } else {
        out.to_string()
    };
    if out.is_empty() { "file".to_string() } else { out }
}

pub fn guess_content_type(filename: &str) -> &'static str {
    let ext = filename.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "log" | "md" | "markdown" | "diff" | "patch" | "rs" | "ts" | "tsx" | "js" | "py" | "toml" | "yaml" | "yml"
        | "sh" | "sql" | "go" => "text/plain; charset=utf-8",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

/// Validated, lowercased content type; falls back to a guess from the filename.
pub fn normalize_content_type(given: Option<&str>, filename: &str) -> String {
    let g = given.map(str::trim).unwrap_or("").to_ascii_lowercase();
    let essence = g.split(';').next().unwrap_or("").trim();
    let valid = !essence.is_empty()
        && essence.len() <= 100
        && essence.contains('/')
        && g.chars().all(|c| c.is_ascii_graphic() || c == ' ')
        && essence != "application/octet-stream"
        && essence != "application/x-www-form-urlencoded"
        && !essence.starts_with("multipart/");
    if valid {
        if essence == "text/plain" { "text/plain; charset=utf-8".to_string() } else { g }
    } else {
        guess_content_type(filename).to_string()
    }
}

/// Stores bytes via `state.storage` (disk or S3) under a random id; max 10 MB (BadRequest beyond). Returns absolute URL.
pub async fn save_file(state: &AppState, actor: &Actor, filename: &str, content_type: Option<&str>, bytes: &[u8]) -> AppResult<FileInfo> {
    if bytes.is_empty() {
        return Err(bad("file is empty"));
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err(bad(format!("file is {:.1} MB; max is 10 MB", bytes.len() as f64 / 1_048_576.0)));
    }
    check_upload_rate(&UPLOAD_LIMITER, &actor.user_id, now_ms())?;
    check_storage_quota(&state.db, &actor.team_id, bytes.len() as i64, state.storage.limits()).await?;
    let filename = sanitize_filename(filename);
    let content_type = normalize_content_type(content_type, &filename);
    let id = random_token(24);
    state.storage.put(&id, &content_type, bytes).await.map_err(AppError::Internal)?;
    sqlx::query(
        "INSERT INTO files (id, team_id, uploader_user_id, uploader_agent_id, filename, content_type, size, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(id.as_str())
    .bind(actor.team_id.as_str())
    .bind(actor.user_id.as_str())
    .bind(actor.agent_id.as_deref())
    .bind(filename.as_str())
    .bind(content_type.as_str())
    .bind(bytes.len() as i64)
    .bind(now_ms())
    .execute(&state.db)
    .await?;
    Ok(FileInfo {
        url: format!("{}/f/{}/{}", state.cfg.public_api_url, id, filename),
        id,
        filename,
        content_type,
        size: bytes.len() as i64,
    })
}

static UPLOAD_LIMITER: std::sync::LazyLock<RateLimiter> = std::sync::LazyLock::new(RateLimiter::new);

/// Per-user upload rate limit, shared by every upload path (REST, MCP, `PUT /u/<code>`).
fn check_upload_rate(limiter: &RateLimiter, user_id: &str, now: i64) -> AppResult<()> {
    if limiter.allow(&format!("upload:{user_id}"), UPLOAD_LIMIT, UPLOAD_WINDOW_MS, now) {
        Ok(())
    } else {
        Err(AppError::RateLimited("slow down: upload limit"))
    }
}

/// Rejects an upload of `new_bytes` that would push the team (or the whole server) over its storage cap.
/// `limits` is (per-team, server-wide) bytes, from `Storage::limits`.
async fn check_storage_quota(db: &SqlitePool, team_id: &str, new_bytes: i64, limits: (i64, i64)) -> AppResult<()> {
    let (team_cap, global_cap) = limits;
    let (team_used,): (i64,) =
        sqlx::query_as("SELECT COALESCE(SUM(size), 0) FROM files WHERE team_id = ?1").bind(team_id).fetch_one(db).await?;
    if team_used + new_bytes > team_cap {
        return Err(bad(format!("team storage limit ({}) reached", crate::storage::cap_label(team_cap))));
    }
    let (total_used,): (i64,) = sqlx::query_as("SELECT COALESCE(SUM(size), 0) FROM files").fetch_one(db).await?;
    if total_used + new_bytes > global_cap {
        return Err(bad("server storage is full; uploads are temporarily disabled"));
    }
    Ok(())
}

/// (filename, content_type, size) of a stored file.
pub async fn file_meta(db: &SqlitePool, id: &str) -> AppResult<Option<(String, String, i64)>> {
    Ok(sqlx::query_as("SELECT filename, content_type, size FROM files WHERE id = ?1").bind(id).fetch_optional(db).await?)
}

/// One-time upload code for `PUT /u/<code>`, valid for `ttl_ms`.
pub async fn create_upload_link(db: &SqlitePool, actor: &Actor, ttl_ms: i64) -> AppResult<String> {
    let code = random_token(32);
    sqlx::query("INSERT INTO upload_links (code, team_id, user_id, agent_id, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)")
        .bind(code.as_str())
        .bind(actor.team_id.as_str())
        .bind(actor.user_id.as_str())
        .bind(actor.agent_id.as_deref())
        .bind(now_ms() + ttl_ms)
        .execute(db)
        .await?;
    Ok(code)
}

/// Atomically burn an upload code. Returns who created it, or None if unknown/used/expired.
pub async fn consume_upload_link(db: &SqlitePool, code: &str) -> AppResult<Option<Actor>> {
    let row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "UPDATE upload_links SET used_at = ?2 WHERE code = ?1 AND used_at IS NULL AND expires_at > ?2
         RETURNING team_id, user_id, agent_id",
    )
    .bind(code)
    .bind(now_ms())
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(team_id, user_id, agent_id)| Actor { team_id, user_id, agent_id }))
}

/// True if an unused, unexpired upload code exists (checked before reading a large body).
pub async fn upload_link_valid(db: &SqlitePool, code: &str) -> AppResult<bool> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM upload_links WHERE code = ?1 AND used_at IS NULL AND expires_at > ?2")
        .bind(code)
        .bind(now_ms())
        .fetch_one(db)
        .await?;
    Ok(n > 0)
}

// ---------------------------------------------------------------------------------------------
// Activity
// ---------------------------------------------------------------------------------------------

#[derive(FromRow)]
struct EventRow {
    id: i64,
    at: i64,
    kind: String,
    data: String,
    actor_user_id: Option<String>,
    actor_handle: Option<String>,
    actor_agent_id: Option<String>,
    actor_agent_name: Option<String>,
    post_number: Option<i64>,
    post_title: Option<String>,
}

macro_rules! event_select {
    () => {
        "SELECT e.id, e.at, e.kind, e.data, e.actor_user_id, u.handle AS actor_handle,
                e.actor_agent_id, a.name AS actor_agent_name, p.number AS post_number, p.title AS post_title
         FROM events e
         LEFT JOIN users u ON u.id = e.actor_user_id
         LEFT JOIN agents a ON a.id = e.actor_agent_id
         LEFT JOIN posts p ON p.id = e.post_id "
    };
}

/// Events with id > `after`, oldest first, max `limit` (default 100).
/// Without `after`, returns the most recent `limit` events (still oldest first).
#[cfg(test)]
pub async fn activity(db: &SqlitePool, team_id: &str, after: Option<i64>, limit: Option<i64>) -> AppResult<Vec<Activity>> {
    activity_filtered(db, team_id, &ActivityFilter { after, limit, ..Default::default() }).await
}

/// Optional narrowing for the activity feed.
#[derive(Debug, Clone, Default)]
pub struct ActivityFilter {
    pub after: Option<i64>,
    pub limit: Option<i64>,
    /// Only events performed by this agent.
    pub agent_id: Option<String>,
    /// Only events by this human or any of their agents.
    pub user_id: Option<String>,
}

/// `activity` with optional agent/user filters (same ordering rules).
pub async fn activity_filtered(db: &SqlitePool, team_id: &str, f: &ActivityFilter) -> AppResult<Vec<Activity>> {
    let limit = f.limit.unwrap_or(100).clamp(1, 500);
    // Two statements so each ORDER BY can walk the events_team index.
    let sql = if f.after.is_some() {
        concat!(
            event_select!(),
            "WHERE e.team_id = ?1 AND e.id > ?2
               AND (?3 IS NULL OR e.actor_agent_id = ?3) AND (?4 IS NULL OR e.actor_user_id = ?4)
             ORDER BY e.id ASC LIMIT ?5"
        )
    } else {
        concat!(
            event_select!(),
            "WHERE e.team_id = ?1 AND ?2 IS NULL
               AND (?3 IS NULL OR e.actor_agent_id = ?3) AND (?4 IS NULL OR e.actor_user_id = ?4)
             ORDER BY e.id DESC LIMIT ?5"
        )
    };
    let mut rows: Vec<EventRow> = sqlx::query_as(sql)
        .bind(team_id)
        .bind(f.after)
        .bind(f.agent_id.as_deref())
        .bind(f.user_id.as_deref())
        .bind(limit)
        .fetch_all(db)
        .await?;
    if f.after.is_none() {
        rows.reverse();
    }
    Ok(rows
        .into_iter()
        .map(|r| Activity {
            id: r.id,
            at: r.at,
            kind: r.kind,
            actor: match (r.actor_user_id, r.actor_handle) {
                (Some(uid), Some(h)) => Some(author(uid, h, r.actor_agent_id, r.actor_agent_name)),
                _ => None,
            },
            post_number: r.post_number,
            post_title: r.post_title,
            data: serde_json::from_str(&r.data).unwrap_or_else(|_| json!({})),
        })
        .collect())
}

/// Most recent `n` events matching `f`, newest first (profile pages).
async fn recent_activity(db: &SqlitePool, team_id: &str, agent_id: Option<&str>, user_id: Option<&str>, n: i64) -> AppResult<Vec<Activity>> {
    let f = ActivityFilter { after: None, limit: Some(n), agent_id: agent_id.map(Into::into), user_id: user_id.map(Into::into) };
    let mut v = activity_filtered(db, team_id, &f).await?;
    v.reverse();
    Ok(v)
}

// ---------------------------------------------------------------------------------------------
// Profiles
// ---------------------------------------------------------------------------------------------

const PROFILE_RECENT: i64 = 50;

/// One agent's page: identity, live leases, queue positions, counters, recent events.
/// None if the agent isn't in this team (or its owner left it).
pub async fn agent_profile(db: &SqlitePool, team_id: &str, agent_id: &str) -> AppResult<Option<AgentProfile>> {
    let Some(agent) = query_agents(db, team_id, false, Some(agent_id), None).await?.into_iter().next() else {
        return Ok(None);
    };
    let claims = leases::list(db, team_id).await?;
    let leases: Vec<Lease> = claims.leases.into_iter().filter(|l| l.holder.agent_id.as_deref() == Some(agent_id)).collect();
    let queue: Vec<LeaseWait> = claims.queue.into_iter().filter(|w| w.waiter.agent_id.as_deref() == Some(agent_id)).collect();
    let (posts, replies, messages_sent, messages_received, sessions): (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM posts WHERE team_id = ?1 AND author_agent_id = ?2),
                (SELECT COUNT(*) FROM comments WHERE team_id = ?1 AND author_agent_id = ?2),
                (SELECT COUNT(*) FROM pings WHERE team_id = ?1 AND from_agent_id = ?2),
                (SELECT COUNT(*) FROM inbox WHERE team_id = ?1 AND agent_id = ?2),
                (SELECT COUNT(*) FROM mcp_sessions WHERE team_id = ?1 AND agent_id = ?2)",
    )
    .bind(team_id)
    .bind(agent_id)
    .fetch_one(db)
    .await?;
    let recent = recent_activity(db, team_id, Some(agent_id), None, PROFILE_RECENT).await?;
    Ok(Some(AgentProfile {
        agent,
        leases,
        queue,
        stats: AgentStats { posts, replies, messages_sent, messages_received, sessions },
        recent,
    }))
}

/// One human's page: membership, their agents in this team (active first), recent events by
/// them or their agents. None if they aren't a member.
pub async fn member_profile(db: &SqlitePool, team_id: &str, user_id: &str) -> AppResult<Option<MemberProfile>> {
    let Some(member) = crate::domain::accounts::get_member(db, team_id, user_id).await? else {
        return Ok(None);
    };
    let mut agents = query_agents(db, team_id, false, None, Some(user_id)).await?;
    agents.sort_by_key(|a| !a.active); // stable: keeps last-seen order within each group
    let recent = recent_activity(db, team_id, None, Some(user_id), PROFILE_RECENT).await?;
    Ok(Some(MemberProfile { member, agents, recent }))
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::test_pool;

    pub struct Fixture {
        pub db: SqlitePool,
        pub team: String,
        pub paul: String,
        pub bob: String,
    }

    pub async fn add_user(db: &SqlitePool, team: &str, handle: &str) -> String {
        let id = new_id("usr");
        sqlx::query("INSERT INTO users (id, email, handle, created_at) VALUES (?1, ?2, ?3, ?4)")
            .bind(id.as_str())
            .bind(format!("{handle}@example.com"))
            .bind(handle)
            .bind(now_ms())
            .execute(db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO memberships (team_id, user_id, role, joined_at) VALUES (?1, ?2, 'member', ?3)")
            .bind(team)
            .bind(id.as_str())
            .bind(now_ms())
            .execute(db)
            .await
            .unwrap();
        id
    }

    pub async fn fixture() -> Fixture {
        let db = test_pool().await;
        let owner = new_id("usr");
        sqlx::query("INSERT INTO users (id, email, handle, created_at) VALUES (?1, 'paul@example.com', 'paul', 0)")
            .bind(owner.as_str())
            .execute(&db)
            .await
            .unwrap();
        let team = new_id("tm");
        sqlx::query("INSERT INTO teams (id, name, slug, owner_id, created_at) VALUES (?1, 'paul''s board', 'paul', ?2, 0)")
            .bind(team.as_str())
            .bind(owner.as_str())
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO memberships (team_id, user_id, role, joined_at) VALUES (?1, ?2, 'owner', 0)")
            .bind(team.as_str())
            .bind(owner.as_str())
            .execute(&db)
            .await
            .unwrap();
        let bob = add_user(&db, &team, "bob").await;
        let human = Actor { team_id: team.clone(), user_id: owner.clone(), agent_id: None };
        for k in ["general", "ui", "backend"] {
            create_board(&db, &human, k, "", "").await.unwrap();
        }
        Fixture { db, team, paul: owner, bob }
    }

    pub async fn session(db: &SqlitePool, team: &str, user: &str, client: &str) -> McpSession {
        // Tests don't have api_tokens rows, so token_id stays NULL.
        let id = format!("mcp_{}", random_token(32));
        sqlx::query(
            "INSERT INTO mcp_sessions (id, team_id, user_id, client_name, protocol, created_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, '2025-06-18', ?5, ?5)",
        )
        .bind(id.as_str())
        .bind(team)
        .bind(user)
        .bind(client)
        .bind(now_ms())
        .execute(db)
        .await
        .unwrap();
        get_mcp_session(db, &id).await.unwrap().unwrap()
    }

    pub async fn named_agent(db: &SqlitePool, team: &str, user: &str, name: &str) -> (McpSession, Actor) {
        let mut s = session(db, team, user, "claude-code").await;
        let out = hello(db, &mut s, name, None, None).await.unwrap();
        let actor = Actor { team_id: team.into(), user_id: user.into(), agent_id: Some(out.agent.id) };
        (s, actor)
    }

    fn np(board: &str, title: &str, body: &str, kind: Option<&str>) -> NewPost {
        NewPost { board: board.into(), title: title.into(), body: body.into(), kind: kind.map(Into::into), status: None, priority: None }
    }

    #[tokio::test]
    async fn post_numbers_are_sequential_per_team() {
        let f = fixture().await;
        let a = Actor { team_id: f.team.clone(), user_id: f.paul.clone(), agent_id: None };
        let p1 = create_post(&f.db, &a, np("ui", "one", "", None)).await.unwrap();
        let p2 = create_post(&f.db, &a, np("backend", "two", "", Some("task"))).await.unwrap();
        assert_eq!((p1.number, p2.number), (1, 2));
        assert_eq!(p2.kind, "task");
        assert_eq!(p1.kind, "note");
        assert_eq!(p1.board_key, "ui");

        // Second team has its own sequence.
        let t2 = new_id("tm");
        sqlx::query("INSERT INTO teams (id, name, slug, owner_id, created_at) VALUES (?1, 't2', 't2', ?2, 0)")
            .bind(t2.as_str())
            .bind(f.paul.as_str())
            .execute(&f.db)
            .await
            .unwrap();
        let a2 = Actor { team_id: t2.clone(), user_id: f.paul.clone(), agent_id: None };
        create_board(&f.db, &a2, "general", "", "").await.unwrap();
        let q = create_post(&f.db, &a2, np("general", "x", "", None)).await.unwrap();
        assert_eq!(q.number, 1);

        assert!(matches!(create_post(&f.db, &a, np("nope", "x", "", None)).await, Err(AppError::NotFound(_))));
        let boards = list_boards(&f.db, &f.team).await.unwrap();
        let ui = boards.iter().find(|b| b.key == "ui").unwrap();
        assert_eq!((ui.open_count, ui.total_count), (1, 1));
        assert!(matches!(create_board(&f.db, &a, "UI", "", "").await, Err(AppError::Conflict(_))));
    }

    #[tokio::test]
    async fn claim_conflict_force_and_release() {
        let f = fixture().await;
        let (_s1, a1) = named_agent(&f.db, &f.team, &f.paul, "ui-fixer").await;
        let (_s2, a2) = named_agent(&f.db, &f.team, &f.bob, "api").await;
        let p = create_post(&f.db, &a1, np("ui", "Fix login redirect", "", Some("task"))).await.unwrap();

        let c = claim_post(&f.db, &a1, p.number, false).await.unwrap();
        assert_eq!(c.status, "in_progress");
        assert_eq!(c.claimed_by.as_ref().unwrap().display, "paul/ui-fixer");
        // Idempotent for the holder.
        claim_post(&f.db, &a1, p.number, false).await.unwrap();

        match claim_post(&f.db, &a2, p.number, false).await {
            Err(AppError::Conflict(m)) => assert!(m.contains("paul/ui-fixer"), "{m}"),
            other => panic!("expected conflict, got {other:?}"),
        }
        // Others can't release it either.
        assert!(matches!(release_post(&f.db, &a2, p.number, None).await, Err(AppError::Conflict(_))));

        // Force takes over and notifies the previous holder.
        let forced = claim_post(&f.db, &a2, p.number, true).await.unwrap();
        assert_eq!(forced.claimed_by.as_ref().unwrap().display, "bob/api");
        let pings = take_agent_pings(&f.db, a1.agent_id.as_deref().unwrap()).await.unwrap();
        assert_eq!(pings.len(), 1);
        assert!(pings[0].body.contains("took over #1"));

        let agents = list_agents(&f.db, &f.team, true).await.unwrap();
        let api = agents.iter().find(|a| a.display == "bob/api").unwrap();
        assert_eq!(api.claimed, vec![p.number]);

        let r = release_post(&f.db, &a2, p.number, Some("done")).await.unwrap();
        assert_eq!(r.status, "done");
        assert!(r.claimed_by.is_none());
        let r2 = release_post(&f.db, &a2, p.number, Some("bogus")).await;
        assert!(matches!(r2, Err(AppError::BadRequest(_))));

        // Humans may release an agent's claim from the dashboard.
        claim_post(&f.db, &a1, p.number, false).await.unwrap();
        let human = Actor { team_id: f.team.clone(), user_id: f.bob.clone(), agent_id: None };
        let r3 = release_post(&f.db, &human, p.number, None).await.unwrap();
        assert_eq!(r3.status, "open");

        // Human claims show the human.
        let hc = claim_post(&f.db, &human, p.number, false).await.unwrap();
        assert_eq!(hc.claimed_by.unwrap().display, "bob");

        let kinds: Vec<String> = activity(&f.db, &f.team, None, None).await.unwrap().into_iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&"claimed".to_string()) && kinds.contains(&"released".to_string()), "{kinds:?}");
    }

    #[tokio::test]
    async fn concurrent_claims_have_one_winner() {
        let f = fixture().await;
        let (_s1, a1) = named_agent(&f.db, &f.team, &f.paul, "a").await;
        let (_s2, a2) = named_agent(&f.db, &f.team, &f.bob, "b").await;
        let p = create_post(&f.db, &a1, np("ui", "race", "", Some("task"))).await.unwrap();
        let (r1, r2) = tokio::join!(claim_post(&f.db, &a1, p.number, false), claim_post(&f.db, &a2, p.number, false));
        assert!(r1.is_ok() != r2.is_ok());
    }

    #[test]
    fn mention_parsing() {
        assert_eq!(parse_mentions("hey @Paul and @bob/ui-fixer, see @bob/*."), vec!["paul", "bob/ui-fixer", "bob/*"]);
        assert!(parse_mentions("mail me at paul@example.com").is_empty());
        assert!(parse_mentions("`@paul` and ```\n@bob\n```").is_empty());
        assert!(parse_mentions("@all please").is_empty());
        assert_eq!(parse_mentions("(@paul) @paul @paul-"), vec!["paul"]);
        assert_eq!(parse_mentions("@paul/"), vec!["paul"]);
    }

    #[tokio::test]
    async fn ping_fan_out_for_each_target_form() {
        let f = fixture().await;
        let (_s1, ui) = named_agent(&f.db, &f.team, &f.paul, "ui-fixer").await;
        let (_s2, docs) = named_agent(&f.db, &f.team, &f.paul, "docs").await;
        let (_s3, api) = named_agent(&f.db, &f.team, &f.bob, "api").await;
        let human_paul = Actor { team_id: f.team.clone(), user_id: f.paul.clone(), agent_id: None };

        // handle/agent
        let s = send_ping(&f.db, &api, "paul/ui-fixer", "hi", None).await.unwrap();
        assert_eq!(s.recipients, vec!["paul/ui-fixer"]);
        // bare agent name
        let s = send_ping(&f.db, &api, "docs", "hi", None).await.unwrap();
        assert_eq!(s.recipients, vec!["paul/docs"]);
        // @handle → human inbox
        let s = send_ping(&f.db, &api, "@paul", "hi human", None).await.unwrap();
        assert_eq!(s.recipients, vec!["paul"]);
        assert_eq!(user_inbox(&f.db, &f.team, &f.paul, false).await.unwrap().len(), 1);
        // handle/* → all paul's active agents (excluding the sender if it is one of them)
        let mut s = send_ping(&f.db, &api, "paul/*", "both", None).await.unwrap();
        s.recipients.sort();
        assert_eq!(s.recipients, vec!["paul/docs", "paul/ui-fixer"]);
        let s = send_ping(&f.db, &ui, "paul/*", "sibling", None).await.unwrap();
        assert_eq!(s.recipients, vec!["paul/docs"]);
        // @all → every active agent + every human, minus sender
        let mut s = send_ping(&f.db, &ui, "@all", "everyone", None).await.unwrap();
        s.recipients.sort();
        assert_eq!(s.recipients, vec!["bob", "bob/api", "paul", "paul/docs"]);
        let s = send_ping(&f.db, &human_paul, "@all", "from human", None).await.unwrap();
        assert!(!s.recipients.contains(&"paul".to_string()) && s.recipients.contains(&"bob".to_string()));
        // errors
        assert!(send_ping(&f.db, &ui, "ui-fixer", "me", None).await.is_err());
        assert!(matches!(send_ping(&f.db, &ui, "paul/nope", "x", None).await, Err(AppError::NotFound(_))));
        assert!(matches!(send_ping(&f.db, &ui, "@zed", "x", None).await, Err(AppError::NotFound(_))));
        assert!(matches!(send_ping(&f.db, &ui, "bob", "", None).await, Err(AppError::BadRequest(_))));
        // ambiguity: bob also has a "docs"
        let (_s4, _bdocs) = named_agent(&f.db, &f.team, &f.bob, "docs").await;
        assert!(matches!(send_ping(&f.db, &api, "docs", "x", None).await, Err(AppError::BadRequest(_))));
        // bare human handle without @ falls back to the human
        let s = send_ping(&f.db, &ui, "bob", "hey", Some(999)).await;
        assert!(matches!(s, Err(AppError::NotFound(_))), "bad post number");
        let s = send_ping(&f.db, &ui, "bob", "hey", None).await.unwrap();
        assert_eq!(s.recipients, vec!["bob"]);

        // delivery marking: piggyback takes each ping once
        let got = take_agent_pings(&f.db, ui.agent_id.as_deref().unwrap()).await.unwrap();
        let bodies: Vec<&str> = got.iter().map(|p| p.body.as_str()).collect();
        assert_eq!(bodies, vec!["hi", "both", "from human"]);
        assert!(got.iter().all(|p| p.delivered_at.is_some()));
        assert_eq!(got[0].from.display, "bob/api");
        assert!(take_agent_pings(&f.db, ui.agent_id.as_deref().unwrap()).await.unwrap().is_empty());
        // inbox(): nothing unread; all=true shows history
        assert!(agent_inbox(&f.db, ui.agent_id.as_deref().unwrap(), false).await.unwrap().is_empty());
        assert_eq!(agent_inbox(&f.db, ui.agent_id.as_deref().unwrap(), true).await.unwrap().len(), 3);
        // docs hasn't been delivered: inbox() returns + marks read
        let d = agent_inbox(&f.db, docs.agent_id.as_deref().unwrap(), false).await.unwrap();
        assert_eq!(d.len(), 5);
        assert!(take_agent_pings(&f.db, docs.agent_id.as_deref().unwrap()).await.unwrap().is_empty());

        // A cleanly closed session makes its agent inactive immediately.
        close_mcp_session(&f.db, &_s2.id).await.unwrap();
        let s = send_ping(&f.db, &api, "paul/*", "who's left?", None).await.unwrap();
        assert_eq!(s.recipients, vec!["paul/ui-fixer"]);
        let active: Vec<String> = list_agents(&f.db, &f.team, true).await.unwrap().into_iter().map(|a| a.display).collect();
        assert!(!active.contains(&"paul/docs".to_string()), "{active:?}");
        assert!(list_agents(&f.db, &f.team, false).await.unwrap().iter().any(|a| a.display == "paul/docs" && !a.active));

        mark_user_inbox_read(&f.db, &f.team, &f.paul).await.unwrap();
        assert!(user_inbox(&f.db, &f.team, &f.paul, false).await.unwrap().is_empty());
        assert_eq!(user_inbox(&f.db, &f.team, &f.paul, true).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn mentions_in_posts_and_comments_ping() {
        let f = fixture().await;
        let (_s1, ui) = named_agent(&f.db, &f.team, &f.paul, "ui-fixer").await;
        let (_s2, api) = named_agent(&f.db, &f.team, &f.bob, "api").await;
        let p = create_post(&f.db, &api, np("ui", "Need help", "@paul/ui-fixer can you look? cc @paul @types/node", None))
            .await
            .unwrap();
        let got = take_agent_pings(&f.db, ui.agent_id.as_deref().unwrap()).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].post_number, Some(p.number));
        assert!(got[0].body.starts_with("mentioned you on #1"));
        assert_eq!(user_inbox(&f.db, &f.team, &f.paul, false).await.unwrap().len(), 1);

        let c = add_comment(&f.db, &ui, p.number, "on it @bob/api").await.unwrap();
        assert_eq!(c.author.display, "paul/ui-fixer");
        assert_eq!(take_agent_pings(&f.db, api.agent_id.as_deref().unwrap()).await.unwrap().len(), 1);
        // self-mention doesn't ping
        add_comment(&f.db, &ui, p.number, "note to @paul/ui-fixer").await.unwrap();
        assert!(take_agent_pings(&f.db, ui.agent_id.as_deref().unwrap()).await.unwrap().is_empty());
        // editing the body only pings NEW mentions
        update_post(&f.db, &api, p.number, PostPatch { body: Some("@paul/ui-fixer @bob".into()), ..Default::default() })
            .await
            .unwrap();
        assert!(take_agent_pings(&f.db, ui.agent_id.as_deref().unwrap()).await.unwrap().is_empty());
        assert_eq!(user_inbox(&f.db, &f.team, &f.bob, false).await.unwrap().len(), 1);

        let d = get_post(&f.db, &f.team, p.number).await.unwrap();
        assert_eq!(d.comments.len(), 2);
        assert_eq!(d.post.comment_count, 2);
    }

    #[tokio::test]
    async fn list_and_update_posts() {
        let f = fixture().await;
        let (_s1, ui) = named_agent(&f.db, &f.team, &f.paul, "ui-fixer").await;
        let (_s2, api) = named_agent(&f.db, &f.team, &f.bob, "api").await;
        let a = create_post(&f.db, &ui, np("ui", "Button colour", "make it blue", Some("task"))).await.unwrap();
        let b = create_post(&f.db, &api, np("backend", "Rate limits", "429s on login", Some("discovery"))).await.unwrap();
        create_post(&f.db, &api, np("backend", "Done thing", "", Some("task"))).await.unwrap();
        release_post(&f.db, &api, 3, Some("done")).await.unwrap();
        claim_post(&f.db, &api, b.number, false).await.unwrap();

        let all = list_posts(&f.db, &f.team, &PostFilter::default()).await.unwrap();
        assert_eq!(all.iter().map(|p| p.number).collect::<Vec<_>>(), vec![3, 2, 1]);
        let active = list_posts(&f.db, &f.team, &PostFilter { status: Some("active".into()), ..Default::default() }).await.unwrap();
        assert_eq!(active.len(), 2);
        let by = |v: &str| PostFilter { claimed_by: Some(v.into()), ..Default::default() };
        assert_eq!(list_posts(&f.db, &f.team, &by("bob/api")).await.unwrap().len(), 1);
        assert_eq!(list_posts(&f.db, &f.team, &by("api")).await.unwrap().len(), 1);
        assert_eq!(list_posts(&f.db, &f.team, &by("bob")).await.unwrap().len(), 1);
        assert_eq!(list_posts(&f.db, &f.team, &by("none")).await.unwrap().len(), 2);
        let au = PostFilter { author: Some("paul/ui-fixer".into()), ..Default::default() };
        assert_eq!(list_posts(&f.db, &f.team, &au).await.unwrap().len(), 1);
        let q = PostFilter { q: Some("BLUE".into()), ..Default::default() };
        assert_eq!(list_posts(&f.db, &f.team, &q).await.unwrap()[0].number, a.number);
        let k = PostFilter { kind: Some("discovery".into()), board: Some("backend".into()), ..Default::default() };
        assert_eq!(list_posts(&f.db, &f.team, &k).await.unwrap().len(), 1);
        let before = PostFilter { before: Some(3), limit: Some(1), ..Default::default() };
        assert_eq!(list_posts(&f.db, &f.team, &before).await.unwrap()[0].number, 2);
        assert!(list_posts(&f.db, &f.team, &PostFilter { status: Some("bogus".into()), ..Default::default() }).await.is_err());

        let u = update_post(
            &f.db,
            &ui,
            a.number,
            PostPatch { status: Some("blocked".into()), board: Some("backend".into()), priority: Some(2), ..Default::default() },
        )
        .await
        .unwrap();
        assert_eq!((u.status.as_str(), u.board_key.as_str(), u.priority), ("blocked", "backend", 2));
        // done clears the claim
        let d = update_post(&f.db, &ui, b.number, PostPatch { status: Some("done".into()), ..Default::default() }).await.unwrap();
        assert!(d.claimed_by.is_none());
        let ev = activity(&f.db, &f.team, None, Some(3)).await.unwrap();
        assert_eq!(ev.len(), 3);
        assert!(ev[0].id < ev[2].id);
        let after = activity(&f.db, &f.team, Some(ev[0].id), None).await.unwrap();
        assert_eq!(after.len(), 2);
        assert_eq!(ev.last().unwrap().kind, "status_changed");
        assert_eq!(ev.last().unwrap().actor.as_ref().unwrap().display, "paul/ui-fixer");
    }

    #[tokio::test]
    async fn hello_naming_adoption_and_suffixing() {
        let f = fixture().await;
        // Provisional agent from the client name.
        let mut s1 = session(&f.db, &f.team, &f.paul, "claude-code").await;
        let prov = ensure_session_agent(&f.db, &mut s1).await.unwrap();
        assert!(prov.name.starts_with("claude-") && prov.name.len() == 11, "{}", prov.name);
        assert_eq!(ensure_session_agent(&f.db, &mut s1).await.unwrap().id, prov.id);
        assert_eq!(client_short(Some("codex-mcp-client")), "codex");

        // hello renames the provisional agent.
        let o = hello(&f.db, &mut s1, "UI Fixer", Some("login page"), Some("sess-1")).await.unwrap();
        assert_eq!(o.agent.display, "paul/ui-fixer");
        assert_eq!(o.agent.id, prov.id);
        assert_eq!(o.renamed_from.as_deref(), Some(prov.name.as_str()));
        assert_eq!(o.agent.working_on.as_deref(), Some("login page"));

        // Same name in a second, concurrent session → suffixed.
        let mut s2 = session(&f.db, &f.team, &f.paul, "claude-code").await;
        let o2 = hello(&f.db, &mut s2, "ui-fixer", None, None).await.unwrap();
        assert_eq!(o2.agent.name, "ui-fixer-2");
        assert_eq!(o2.requested.as_deref(), Some("ui-fixer"));
        let mut s3 = session(&f.db, &f.team, &f.paul, "codex-mcp-client").await;
        assert_eq!(hello(&f.db, &mut s3, "ui-fixer", None, None).await.unwrap().agent.name, "ui-fixer-3");

        // Another human can use the same name freely.
        let mut sb = session(&f.db, &f.team, &f.bob, "claude-code").await;
        assert_eq!(hello(&f.db, &mut sb, "ui-fixer", None, None).await.unwrap().agent.display, "bob/ui-fixer");

        // Give ui-fixer a claim and an unread ping, then close its session → inactive.
        let ui = Actor { team_id: f.team.clone(), user_id: f.paul.clone(), agent_id: Some(prov.id.clone()) };
        let p = create_post(&f.db, &ui, np("ui", "t", "", Some("task"))).await.unwrap();
        claim_post(&f.db, &ui, p.number, false).await.unwrap();
        let bob_h = Actor { team_id: f.team.clone(), user_id: f.bob.clone(), agent_id: None };
        send_ping(&f.db, &bob_h, "paul/ui-fixer", "still there?", None).await.unwrap();
        close_mcp_session(&f.db, &s1.id).await.unwrap();

        // A new session: provisional first (it does something), then hello adopts ui-fixer.
        let mut s4 = session(&f.db, &f.team, &f.paul, "claude-code").await;
        let prov4 = ensure_session_agent(&f.db, &mut s4).await.unwrap();
        let o4 = hello(&f.db, &mut s4, "ui-fixer", None, None).await.unwrap();
        assert!(o4.adopted);
        assert_eq!(o4.agent.id, prov.id);
        assert_eq!(s4.agent_id.as_deref(), Some(prov.id.as_str()));
        assert!(agent_by_id(&f.db, &prov4.id).await.unwrap().is_none(), "unused provisional agent deleted");
        let inherited = take_agent_pings(&f.db, &prov.id).await.unwrap();
        assert_eq!(inherited.len(), 1);
        let post = get_post(&f.db, &f.team, p.number).await.unwrap().post;
        assert_eq!(post.claimed_by.unwrap().agent_id.as_deref(), Some(prov.id.as_str()));

        // Resumed conversation with the same session key reclaims even though s4 is active.
        let mut s5 = session(&f.db, &f.team, &f.paul, "claude-code").await;
        let o5 = hello(&f.db, &mut s5, "ui-fixer", None, Some("sess-1")).await.unwrap();
        assert!(o5.adopted && o5.agent.id == prov.id);
        let linked = agent_by_session_key(&f.db, &f.team, &f.paul, "sess-1").await.unwrap().unwrap();
        assert_eq!(linked.id, prov.id);
        assert!(agent_by_session_key(&f.db, &f.team, &f.bob, "sess-1").await.unwrap().is_none());

        // A provisional agent that authored something is kept (its post keeps its author).
        let mut s6 = session(&f.db, &f.team, &f.paul, "claude-code").await;
        let prov6 = ensure_session_agent(&f.db, &mut s6).await.unwrap();
        let a6 = Actor { team_id: f.team.clone(), user_id: f.paul.clone(), agent_id: Some(prov6.id.clone()) };
        create_post(&f.db, &a6, np("ui", "by provisional", "", None)).await.unwrap();
        close_mcp_session(&f.db, &s2.id).await.unwrap();
        let o6 = hello(&f.db, &mut s6, "ui-fixer-2", None, None).await.unwrap();
        assert!(o6.adopted);
        assert!(agent_by_id(&f.db, &prov6.id).await.unwrap().is_some());

        assert!(hello(&f.db, &mut s6, "!!!", None, None).await.is_err());
        let names: Vec<String> = list_agents(&f.db, &f.team, false).await.unwrap().into_iter().map(|a| a.display).collect();
        assert!(names.contains(&"paul/ui-fixer-3".to_string()), "{names:?}");
        let joined = activity(&f.db, &f.team, None, Some(500)).await.unwrap().into_iter().filter(|e| e.kind == "agent_joined").count();
        assert!(joined >= 5);
    }

    #[tokio::test]
    async fn upload_links_are_one_time() {
        let f = fixture().await;
        let a = Actor { team_id: f.team.clone(), user_id: f.paul.clone(), agent_id: None };
        let code = create_upload_link(&f.db, &a, 60_000).await.unwrap();
        assert!(upload_link_valid(&f.db, &code).await.unwrap());
        assert_eq!(consume_upload_link(&f.db, &code).await.unwrap().unwrap().user_id, f.paul);
        assert!(consume_upload_link(&f.db, &code).await.unwrap().is_none());
        let expired = create_upload_link(&f.db, &a, -1).await.unwrap();
        assert!(consume_upload_link(&f.db, &expired).await.unwrap().is_none());
    }

    #[test]
    fn filenames_and_types() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("my screenshot (1).PNG"), "my-screenshot-1-.PNG");
        assert_eq!(sanitize_filename(".env"), "env");
        assert_eq!(sanitize_filename(""), "file");
        assert_eq!(normalize_content_type(None, "a.png"), "image/png");
        assert_eq!(normalize_content_type(Some("application/octet-stream"), "a.pdf"), "application/pdf");
        assert_eq!(normalize_content_type(Some("Text/Plain"), "a"), "text/plain; charset=utf-8");
        assert_eq!(normalize_content_type(Some("image/svg+xml"), "a.png"), "image/svg+xml");
        assert_eq!(normalize_content_type(Some("bad\nheader"), "a.txt"), "text/plain; charset=utf-8");
        assert_eq!(slugify("  UI__Fixer!! "), "ui-fixer");
    }

    async fn insert_file(db: &SqlitePool, team: &str, user: &str, size: i64) {
        sqlx::query(
            "INSERT INTO files (id, team_id, uploader_user_id, uploader_agent_id, filename, content_type, size, created_at)
             VALUES (?1, ?2, ?3, NULL, 'f', 'text/plain', ?4, 0)",
        )
        .bind(random_token(24))
        .bind(team)
        .bind(user)
        .bind(size)
        .execute(db)
        .await
        .unwrap();
    }

    async fn add_team(db: &SqlitePool, owner: &str) -> String {
        let team = new_id("tm");
        sqlx::query("INSERT INTO teams (id, name, slug, owner_id, created_at) VALUES (?1, 'other', lower(substr(?1, 4)), ?2, 0)")
            .bind(team.as_str())
            .bind(owner)
            .execute(db)
            .await
            .unwrap();
        team
    }

    #[tokio::test]
    async fn storage_quota() {
        use crate::storage::{DISK_LIMITS, S3_LIMITS};
        let (team_cap, global_cap) = DISK_LIMITS;
        let f = fixture().await;
        let other = add_team(&f.db, &f.paul).await;
        let third = add_team(&f.db, &f.paul).await;
        check_storage_quota(&f.db, &f.team, 10, DISK_LIMITS).await.unwrap();
        insert_file(&f.db, &f.team, &f.paul, team_cap - 10).await;
        check_storage_quota(&f.db, &f.team, 10, DISK_LIMITS).await.expect("exactly at the cap is fine");
        let err = check_storage_quota(&f.db, &f.team, 11, DISK_LIMITS).await.unwrap_err();
        assert!(matches!(&err, AppError::BadRequest(m) if m == "team storage limit (200 MB) reached"), "{err}");
        // S3 mode has a 1 GB team cap.
        check_storage_quota(&f.db, &f.team, 11, S3_LIMITS).await.expect("S3 team cap is higher");
        // Other teams are unaffected until the global cap is hit.
        check_storage_quota(&f.db, &other, MAX_FILE_BYTES as i64, DISK_LIMITS).await.unwrap();
        insert_file(&f.db, &other, &f.paul, global_cap - team_cap + 10).await;
        let err = check_storage_quota(&f.db, &third, 1, DISK_LIMITS).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "global cap applies");
        check_storage_quota(&f.db, &third, 1, S3_LIMITS).await.expect("S3 global cap is higher");
        insert_file(&f.db, &third, &f.paul, S3_LIMITS.0).await;
        let err = check_storage_quota(&f.db, &third, 1, S3_LIMITS).await.unwrap_err();
        assert!(matches!(&err, AppError::BadRequest(m) if m == "team storage limit (1 GB) reached"), "{err}");
    }

    #[test]
    fn upload_rate_limit() {
        let rl = RateLimiter::new();
        for i in 0..UPLOAD_LIMIT as i64 {
            check_upload_rate(&rl, "usr_a", 1_000 + i).unwrap();
        }
        let err = check_upload_rate(&rl, "usr_a", 2_000).unwrap_err();
        assert_eq!(err.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(err.to_string(), "slow down: upload limit");
        check_upload_rate(&rl, "usr_b", 2_000).expect("per-user");
        check_upload_rate(&rl, "usr_a", 1_000 + UPLOAD_WINDOW_MS).expect("window slides");
    }
}
