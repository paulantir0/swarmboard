//! Accounts domain: users, teams, memberships, invites, magic-link tokens, sessions, API tokens.
//! OWNER: backend/auth subagent. `authenticate_token` signature is fixed (MCP + hook depend on it).
//!
//! Secrets (login links, session cookies, API tokens) are only ever stored as sha256 hex.
//! Writes that must not race (signup, joining a team, rotating invites) run in `BEGIN IMMEDIATE`
//! transactions, which take SQLite's write lock up front and so serialize against each other.

use serde::Serialize;
use sqlx::{SqliteConnection, SqliteExecutor, SqlitePool};

use crate::{
    config::{Config, MAX_TEAM_MEMBERS},
    error::{AppError, AppResult},
    models::TokenAuth,
    util::{new_id, now_ms, random_token, sha256_hex},
};

/// Magic links expire after 20 minutes.
pub const LOGIN_TOKEN_TTL_MS: i64 = 20 * 60 * 1000;
/// Browser sessions last 30 days.
pub const SESSION_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// Boards every new personal team starts with.
pub const DEFAULT_BOARDS: &[&str] = &["general", "ui", "backend", "ci"];
pub const HANDLE_MAX_LEN: usize = 20;
/// Handles that would collide with ping targets (`@all`) or read as the system.
pub const RESERVED_HANDLES: &[&str] = &["all", "admin", "api", "everyone", "here", "root", "swarmboard", "system"];
pub const API_TOKEN_PREFIX: &str = "sb_";
/// Plaintext token = "sb_" + this many base62 chars.
pub const API_TOKEN_RANDOM_LEN: usize = 40;
/// Chars of the plaintext kept for display ("sb_" + 8).
const API_TOKEN_DISPLAY_LEN: usize = 11;
/// `last_used_at` is written at most this often per token.
const TOKEN_TOUCH_INTERVAL_MS: i64 = 60 * 1000;
const MAX_ACTIVE_TOKENS: i64 = 25;
const TEAM_NAME_MAX_LEN: usize = 60;
const TOKEN_LABEL_MAX_LEN: usize = 60;

// ---------------------------------------------------------------------------------------------
// Types (JSON shapes for the dashboard)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub email: String,
    pub handle: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Member,
}

impl Role {
    fn parse(s: &str) -> Role {
        if s == "owner" { Role::Owner } else { Role::Member }
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TeamSummary {
    pub id: String,
    pub name: String,
    /// "owner" | "member"
    pub role: String,
    pub member_count: i64,
    pub max_members: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Member {
    pub user_id: String,
    pub email: String,
    pub handle: String,
    /// "owner" | "member"
    pub role: String,
    pub joined_at: i64,
}

/// Public view of an invite link (shown on the /join page before signing in).
#[derive(Debug, Clone, Serialize)]
pub struct InviteInfo {
    pub team_name: String,
    pub inviter_handle: String,
    pub member_count: i64,
    pub max_members: i64,
    pub full: bool,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ApiTokenInfo {
    pub id: String,
    pub prefix: String,
    pub label: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

/// A freshly created API token. `token` is the plaintext and is never retrievable again.
#[derive(Debug, Clone, Serialize)]
pub struct NewApiToken {
    pub id: String,
    pub prefix: String,
    pub label: String,
    pub token: String,
}

/// What a consumed magic link carried.
#[derive(Debug, Clone, PartialEq)]
pub struct LoginClaim {
    pub email: String,
    pub invite_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinOutcome {
    Joined,
    AlreadyMember,
}

// ---------------------------------------------------------------------------------------------
// Emails and handles
// ---------------------------------------------------------------------------------------------

/// Trimmed, lowercased email if it looks deliverable; None otherwise. Deliberately loose.
pub fn normalize_email(raw: &str) -> Option<String> {
    let e = raw.trim().to_lowercase();
    if e.len() < 6 || e.len() > 254 {
        return None;
    }
    if e.chars().any(|c| c.is_whitespace() || c.is_control() || "<>\"(),;:[]\\".contains(c)) {
        return None;
    }
    let (local, domain) = e.split_once('@')?;
    if local.is_empty() || local.len() > 64 || domain.contains('@') {
        return None;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|l| l.is_empty() || l.starts_with('-') || l.ends_with('-')) {
        return None;
    }
    Some(e)
}

/// Lowercase `[a-z0-9-]`, runs of anything else collapsed to one '-', trimmed, max 20 chars.
fn slugify_handle(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= HANDLE_MAX_LEN {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

/// Handle candidate from an email's local part ("Paul.Smith+x@…" -> "paul-smith"). Never empty.
pub fn handle_base_from_email(email: &str) -> String {
    let local = email.split('@').next().unwrap_or("");
    let local = local.split('+').next().unwrap_or(local);
    let base = slugify_handle(local);
    if base.is_empty() { "user".to_string() } else { base }
}

/// Validates a user-chosen handle (PATCH /api/me).
pub fn validate_handle(raw: &str) -> AppResult<String> {
    let h = raw.trim().to_ascii_lowercase();
    let ok_chars = h.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if h.is_empty() || h.len() > HANDLE_MAX_LEN || !ok_chars || h.starts_with('-') || h.ends_with('-') {
        return Err(AppError::BadRequest("handle must be 1-20 characters: a-z, 0-9, -".into()));
    }
    if RESERVED_HANDLES.contains(&h.as_str()) {
        return Err(AppError::BadRequest("that handle is reserved".into()));
    }
    Ok(h)
}

/// `base`, then `base2`, `base3`, … (truncating `base` so the result stays within 20 chars).
async fn unique_handle(conn: &mut SqliteConnection, base: &str) -> AppResult<String> {
    for n in 1..1000u32 {
        let candidate = if n == 1 {
            base.to_string()
        } else {
            let suffix = n.to_string();
            let keep = base.len().min(HANDLE_MAX_LEN - suffix.len());
            format!("{}{}", base[..keep].trim_end_matches('-'), suffix)
        };
        if RESERVED_HANDLES.contains(&candidate.as_str()) {
            continue;
        }
        let taken: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE handle = ?")
            .bind(&candidate)
            .fetch_optional(&mut *conn)
            .await?;
        if taken.is_none() {
            return Ok(candidate);
        }
    }
    Ok(format!("u{}", random_token(8).to_ascii_lowercase()))
}

// ---------------------------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------------------------

async fn user_by_email<'e>(ex: impl SqliteExecutor<'e>, email: &str) -> AppResult<Option<User>> {
    Ok(sqlx::query_as::<_, User>("SELECT id, email, handle, created_at FROM users WHERE email = ?")
        .bind(email)
        .fetch_optional(ex)
        .await?)
}

pub async fn get_user(db: &SqlitePool, user_id: &str) -> AppResult<Option<User>> {
    Ok(sqlx::query_as::<_, User>("SELECT id, email, handle, created_at FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(db)
        .await?)
}

/// Finds the user for `email`, or creates them together with their personal team.
/// Returns `(user, created)`.
pub async fn find_or_create_user(db: &SqlitePool, email: &str) -> AppResult<(User, bool)> {
    let email = normalize_email(email).ok_or_else(|| AppError::BadRequest("invalid email".into()))?;
    if let Some(user) = user_by_email(db, &email).await? {
        return Ok((user, false));
    }
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    if let Some(user) = user_by_email(&mut *tx, &email).await? {
        tx.commit().await?;
        return Ok((user, false));
    }
    let handle = unique_handle(&mut tx, &handle_base_from_email(&email)).await?;
    let user = User { id: new_id("usr"), email, handle, created_at: now_ms() };
    sqlx::query("INSERT INTO users (id, email, handle, created_at) VALUES (?, ?, ?, ?)")
        .bind(&user.id)
        .bind(&user.email)
        .bind(&user.handle)
        .bind(user.created_at)
        .execute(&mut *tx)
        .await?;
    create_personal_team(&mut tx, &user).await?;
    tx.commit().await?;
    tracing::info!(user_id = %user.id, handle = %user.handle, "user created");
    Ok((user, true))
}

/// "<handle>'s board" with the default boards; `user` becomes owner. Returns the team id.
async fn create_personal_team(conn: &mut SqliteConnection, user: &User) -> AppResult<String> {
    let now = now_ms();
    let team_id = new_id("tm");
    sqlx::query("INSERT INTO teams (id, name, owner_id, created_at) VALUES (?, ?, ?, ?)")
        .bind(&team_id)
        .bind(format!("{}'s board", user.handle))
        .bind(&user.id)
        .bind(now)
        .execute(&mut *conn)
        .await?;
    sqlx::query("INSERT INTO memberships (team_id, user_id, role, joined_at) VALUES (?, ?, 'owner', ?)")
        .bind(&team_id)
        .bind(&user.id)
        .bind(now)
        .execute(&mut *conn)
        .await?;
    for key in DEFAULT_BOARDS {
        sqlx::query(
            "INSERT INTO boards (id, team_id, key, name, description, created_by_user, created_at) \
             VALUES (?, ?, ?, ?, '', ?, ?)",
        )
        .bind(new_id("brd"))
        .bind(&team_id)
        .bind(*key)
        .bind(*key)
        .bind(&user.id)
        .bind(now)
        .execute(&mut *conn)
        .await?;
    }
    Ok(team_id)
}

/// Changes the user's handle. Conflict if someone else has it.
pub async fn update_handle(db: &SqlitePool, user_id: &str, raw: &str) -> AppResult<User> {
    let handle = validate_handle(raw)?;
    let res = sqlx::query("UPDATE users SET handle = ? WHERE id = ?").bind(&handle).bind(user_id).execute(db).await;
    match res {
        Ok(r) if r.rows_affected() == 0 => return Err(AppError::NotFound("user not found".into())),
        Ok(_) => {}
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(AppError::Conflict("that handle is taken".into()));
        }
        Err(e) => return Err(e.into()),
    }
    get_user(db, user_id).await?.ok_or_else(|| AppError::NotFound("user not found".into()))
}

// ---------------------------------------------------------------------------------------------
// Teams and memberships
// ---------------------------------------------------------------------------------------------

/// Teams the user belongs to: own team(s) first, then joined ones in join order.
pub async fn list_user_teams(db: &SqlitePool, user_id: &str) -> AppResult<Vec<TeamSummary>> {
    Ok(sqlx::query_as::<_, TeamSummary>(
        "SELECT t.id, t.name, m.role, \
                (SELECT COUNT(*) FROM memberships m2 WHERE m2.team_id = t.id) AS member_count, \
                ? AS max_members \
         FROM memberships m JOIN teams t ON t.id = m.team_id \
         WHERE m.user_id = ? \
         ORDER BY (m.role = 'owner') DESC, m.joined_at, t.id",
    )
    .bind(MAX_TEAM_MEMBERS)
    .bind(user_id)
    .fetch_all(db)
    .await?)
}

pub async fn member_role(db: &SqlitePool, team_id: &str, user_id: &str) -> AppResult<Option<Role>> {
    let role: Option<String> = sqlx::query_scalar("SELECT role FROM memberships WHERE team_id = ? AND user_id = ?")
        .bind(team_id)
        .bind(user_id)
        .fetch_optional(db)
        .await?;
    Ok(role.map(|r| Role::parse(&r)))
}

pub async fn get_team(db: &SqlitePool, team_id: &str) -> AppResult<Option<Team>> {
    Ok(sqlx::query_as::<_, Team>("SELECT id, name, owner_id, created_at FROM teams WHERE id = ?")
        .bind(team_id)
        .fetch_optional(db)
        .await?)
}

/// Owner first, then by join time.
pub async fn list_members(db: &SqlitePool, team_id: &str) -> AppResult<Vec<Member>> {
    Ok(sqlx::query_as::<_, Member>(
        "SELECT m.user_id, u.email, u.handle, m.role, m.joined_at \
         FROM memberships m JOIN users u ON u.id = m.user_id \
         WHERE m.team_id = ? ORDER BY (m.role = 'owner') DESC, m.joined_at, u.handle",
    )
    .bind(team_id)
    .fetch_all(db)
    .await?)
}

pub async fn rename_team(db: &SqlitePool, team_id: &str, raw_name: &str) -> AppResult<Team> {
    let name = raw_name.trim();
    if name.is_empty() || name.chars().count() > TEAM_NAME_MAX_LEN {
        return Err(AppError::BadRequest(format!("name must be 1-{TEAM_NAME_MAX_LEN} characters")));
    }
    sqlx::query("UPDATE teams SET name = ? WHERE id = ?").bind(name).bind(team_id).execute(db).await?;
    get_team(db, team_id).await?.ok_or_else(|| AppError::NotFound("team not found".into()))
}

/// Owner removes anyone else; members may remove themselves; the owner can't leave.
/// The removed member's API tokens for this team are revoked.
pub async fn remove_member(db: &SqlitePool, team_id: &str, actor_id: &str, target_id: &str) -> AppResult<()> {
    let actor_role = member_role(db, team_id, actor_id)
        .await?
        .ok_or_else(|| AppError::Forbidden("not a member of this team".into()))?;
    let target_role = member_role(db, team_id, target_id)
        .await?
        .ok_or_else(|| AppError::NotFound("not a member of this team".into()))?;
    if target_role == Role::Owner {
        return Err(AppError::Forbidden("the owner can't leave their own team".into()));
    }
    if actor_id != target_id && actor_role != Role::Owner {
        return Err(AppError::Forbidden("only the owner can remove members".into()));
    }
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM memberships WHERE team_id = ? AND user_id = ?")
        .bind(team_id)
        .bind(target_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE api_tokens SET revoked_at = ? WHERE team_id = ? AND user_id = ? AND revoked_at IS NULL")
        .bind(now_ms())
        .bind(team_id)
        .bind(target_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Invites
// ---------------------------------------------------------------------------------------------

pub fn invite_url(cfg: &Config, code: &str) -> String {
    format!("{}/join/{}", cfg.web_url, code)
}

async fn insert_invite(conn: &mut SqliteConnection, team_id: &str, user_id: &str) -> AppResult<String> {
    let code = random_token(12);
    sqlx::query("INSERT INTO invites (code, team_id, created_by, created_at) VALUES (?, ?, ?, ?)")
        .bind(&code)
        .bind(team_id)
        .bind(user_id)
        .bind(now_ms())
        .execute(&mut *conn)
        .await?;
    Ok(code)
}

/// The team's current invite code, creating one (attributed to `user_id`) if none is active.
pub async fn current_invite(db: &SqlitePool, team_id: &str, user_id: &str) -> AppResult<String> {
    const Q: &str = "SELECT code FROM invites WHERE team_id = ? AND revoked_at IS NULL ORDER BY created_at DESC LIMIT 1";
    if let Some(code) = sqlx::query_scalar::<_, String>(Q).bind(team_id).fetch_optional(db).await? {
        return Ok(code);
    }
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let code = match sqlx::query_scalar::<_, String>(Q).bind(team_id).fetch_optional(&mut *tx).await? {
        Some(code) => code,
        None => insert_invite(&mut tx, team_id, user_id).await?,
    };
    tx.commit().await?;
    Ok(code)
}

/// Revokes every active invite for the team and issues a new one.
pub async fn rotate_invite(db: &SqlitePool, team_id: &str, user_id: &str) -> AppResult<String> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query("UPDATE invites SET revoked_at = ? WHERE team_id = ? AND revoked_at IS NULL")
        .bind(now_ms())
        .bind(team_id)
        .execute(&mut *tx)
        .await?;
    let code = insert_invite(&mut tx, team_id, user_id).await?;
    tx.commit().await?;
    Ok(code)
}

/// None if the code is unknown or revoked.
pub async fn invite_info(db: &SqlitePool, code: &str) -> AppResult<Option<InviteInfo>> {
    let row: Option<(String, String, i64)> = sqlx::query_as(
        "SELECT t.name, u.handle, (SELECT COUNT(*) FROM memberships m WHERE m.team_id = t.id) \
         FROM invites i JOIN teams t ON t.id = i.team_id JOIN users u ON u.id = i.created_by \
         WHERE i.code = ? AND i.revoked_at IS NULL",
    )
    .bind(code)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(team_name, inviter_handle, member_count)| InviteInfo {
        team_name,
        inviter_handle,
        member_count,
        max_members: MAX_TEAM_MEMBERS,
        full: member_count >= MAX_TEAM_MEMBERS,
    }))
}

/// Joins `user_id` to the invite's team. Idempotent for existing members.
/// NotFound for unknown/revoked codes, Conflict when the team already has 5 humans.
/// The cap check and insert happen under SQLite's write lock, so concurrent joins can't overshoot.
pub async fn join_via_invite(db: &SqlitePool, code: &str, user_id: &str) -> AppResult<(String, JoinOutcome)> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let team_id: String = sqlx::query_scalar("SELECT team_id FROM invites WHERE code = ? AND revoked_at IS NULL")
        .bind(code)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound("this invite link is no longer valid".into()))?;
    let existing: Option<i64> = sqlx::query_scalar("SELECT 1 FROM memberships WHERE team_id = ? AND user_id = ?")
        .bind(&team_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
    if existing.is_some() {
        tx.commit().await?;
        return Ok((team_id, JoinOutcome::AlreadyMember));
    }
    let now = now_ms();
    // Conditional insert: belt and braces on top of the write lock.
    let inserted = sqlx::query(
        "INSERT INTO memberships (team_id, user_id, role, joined_at) \
         SELECT ?1, ?2, 'member', ?3 \
         WHERE (SELECT COUNT(*) FROM memberships WHERE team_id = ?1) < ?4",
    )
    .bind(&team_id)
    .bind(user_id)
    .bind(now)
    .bind(MAX_TEAM_MEMBERS)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        tx.rollback().await?;
        return Err(AppError::Conflict(format!("this team is full ({MAX_TEAM_MEMBERS} people)")));
    }
    let handle: String = sqlx::query_scalar("SELECT handle FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO events (team_id, at, actor_user_id, kind, data) VALUES (?, ?, ?, 'member_joined', ?)")
        .bind(&team_id)
        .bind(now)
        .bind(user_id)
        .bind(serde_json::json!({ "handle": handle }).to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(%team_id, %user_id, "member joined");
    Ok((team_id, JoinOutcome::Joined))
}

// ---------------------------------------------------------------------------------------------
// Magic-link login tokens
// ---------------------------------------------------------------------------------------------

/// Creates a single-use login token for `email` and returns the plaintext secret (32 base62).
pub async fn create_login_token(db: &SqlitePool, email: &str, invite_code: Option<&str>) -> AppResult<String> {
    let now = now_ms();
    // Housekeeping: drop tokens that expired over a day ago.
    sqlx::query("DELETE FROM login_tokens WHERE expires_at < ?").bind(now - 24 * 60 * 60 * 1000).execute(db).await?;
    let secret = random_token(32);
    sqlx::query(
        "INSERT INTO login_tokens (token_hash, email, invite_code, created_at, expires_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(sha256_hex(&secret))
    .bind(email)
    .bind(invite_code)
    .bind(now)
    .bind(now + LOGIN_TOKEN_TTL_MS)
    .execute(db)
    .await?;
    Ok(secret)
}

/// Atomically marks the token used. None if unknown, expired, or already used.
pub async fn consume_login_token(db: &SqlitePool, secret: &str) -> AppResult<Option<LoginClaim>> {
    let secret = secret.trim();
    if secret.is_empty() || secret.len() > 128 {
        return Ok(None);
    }
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "UPDATE login_tokens SET used_at = ?1 \
         WHERE token_hash = ?2 AND used_at IS NULL AND expires_at > ?1 \
         RETURNING email, invite_code",
    )
    .bind(now_ms())
    .bind(sha256_hex(secret))
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(email, invite_code)| LoginClaim { email, invite_code }))
}

// ---------------------------------------------------------------------------------------------
// Browser sessions
// ---------------------------------------------------------------------------------------------

/// Creates a 30-day session and returns the cookie secret.
pub async fn create_session(db: &SqlitePool, user_id: &str) -> AppResult<String> {
    let now = now_ms();
    sqlx::query("DELETE FROM sessions WHERE expires_at < ?").bind(now).execute(db).await?;
    let secret = random_token(40);
    sqlx::query("INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)")
        .bind(sha256_hex(&secret))
        .bind(user_id)
        .bind(now)
        .bind(now + SESSION_TTL_MS)
        .execute(db)
        .await?;
    Ok(secret)
}

/// The signed-in user for a session cookie secret; None if unknown or expired.
pub async fn session_user(db: &SqlitePool, secret: &str) -> AppResult<Option<User>> {
    if secret.is_empty() || secret.len() > 128 {
        return Ok(None);
    }
    Ok(sqlx::query_as::<_, User>(
        "SELECT u.id, u.email, u.handle, u.created_at \
         FROM sessions s JOIN users u ON u.id = s.user_id \
         WHERE s.token_hash = ? AND s.expires_at > ?",
    )
    .bind(sha256_hex(secret))
    .bind(now_ms())
    .fetch_optional(db)
    .await?)
}

pub async fn delete_session(db: &SqlitePool, secret: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?").bind(sha256_hex(secret)).execute(db).await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// API tokens (MCP / hooks)
// ---------------------------------------------------------------------------------------------

/// Creates a bearer token for (user, team). The plaintext is only returned here.
pub async fn create_api_token(db: &SqlitePool, user_id: &str, team_id: &str, label: Option<&str>) -> AppResult<NewApiToken> {
    let label = label.map(str::trim).filter(|l| !l.is_empty()).unwrap_or("token");
    if label.chars().count() > TOKEN_LABEL_MAX_LEN {
        return Err(AppError::BadRequest(format!("label must be at most {TOKEN_LABEL_MAX_LEN} characters")));
    }
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM api_tokens WHERE user_id = ? AND team_id = ? AND revoked_at IS NULL",
    )
    .bind(user_id)
    .bind(team_id)
    .fetch_one(db)
    .await?;
    if active >= MAX_ACTIVE_TOKENS {
        return Err(AppError::Conflict(format!("at most {MAX_ACTIVE_TOKENS} tokens; revoke one first")));
    }
    let token = format!("{API_TOKEN_PREFIX}{}", random_token(API_TOKEN_RANDOM_LEN));
    let created = NewApiToken {
        id: new_id("tok"),
        prefix: token[..API_TOKEN_DISPLAY_LEN].to_string(),
        label: label.to_string(),
        token,
    };
    sqlx::query(
        "INSERT INTO api_tokens (id, token_hash, prefix, user_id, team_id, label, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&created.id)
    .bind(sha256_hex(&created.token))
    .bind(&created.prefix)
    .bind(user_id)
    .bind(team_id)
    .bind(&created.label)
    .bind(now_ms())
    .execute(db)
    .await?;
    Ok(created)
}

/// The user's active tokens in a team, newest first.
pub async fn list_api_tokens(db: &SqlitePool, user_id: &str, team_id: &str) -> AppResult<Vec<ApiTokenInfo>> {
    Ok(sqlx::query_as::<_, ApiTokenInfo>(
        "SELECT id, prefix, label, created_at, last_used_at FROM api_tokens \
         WHERE user_id = ? AND team_id = ? AND revoked_at IS NULL ORDER BY created_at DESC, id",
    )
    .bind(user_id)
    .bind(team_id)
    .fetch_all(db)
    .await?)
}

/// Revokes one of the user's own tokens. NotFound if it isn't theirs or is already revoked.
pub async fn revoke_api_token(db: &SqlitePool, user_id: &str, token_id: &str) -> AppResult<()> {
    let n = sqlx::query("UPDATE api_tokens SET revoked_at = ? WHERE id = ? AND user_id = ? AND revoked_at IS NULL")
        .bind(now_ms())
        .bind(token_id)
        .bind(user_id)
        .execute(db)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound("token not found".into()));
    }
    Ok(())
}

/// Look up a bearer token (plaintext as sent, e.g. "sb_…"). None if unknown or revoked.
/// Updates last_used_at (cheaply; at most once a minute is fine).
pub async fn authenticate_token(db: &SqlitePool, plaintext: &str) -> AppResult<Option<TokenAuth>> {
    let t = plaintext.trim();
    if !t.starts_with(API_TOKEN_PREFIX) || t.len() > 128 {
        return Ok(None);
    }
    // Joining memberships means a token stops working as soon as its human leaves the team.
    let row: Option<(String, String, String, String, Option<i64>)> = sqlx::query_as(
        "SELECT t.id, t.user_id, t.team_id, u.handle, t.last_used_at \
         FROM api_tokens t \
         JOIN users u ON u.id = t.user_id \
         JOIN memberships m ON m.team_id = t.team_id AND m.user_id = t.user_id \
         WHERE t.token_hash = ? AND t.revoked_at IS NULL",
    )
    .bind(sha256_hex(t))
    .fetch_optional(db)
    .await?;
    let Some((token_id, user_id, team_id, handle, last_used_at)) = row else {
        return Ok(None);
    };
    let now = now_ms();
    if last_used_at.is_none_or(|t| now - t >= TOKEN_TOUCH_INTERVAL_MS) {
        // Best effort: a busy database must not fail authentication.
        if let Err(e) = sqlx::query("UPDATE api_tokens SET last_used_at = ? WHERE id = ?")
            .bind(now)
            .bind(&token_id)
            .execute(db)
            .await
        {
            tracing::warn!(error = %e, %token_id, "failed to touch api token");
        }
    }
    Ok(Some(TokenAuth { token_id, user_id, team_id, handle }))
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_pool;

    async fn signup(db: &SqlitePool, email: &str) -> User {
        find_or_create_user(db, email).await.unwrap().0
    }

    async fn personal_team(db: &SqlitePool, user: &User) -> String {
        let teams = list_user_teams(db, &user.id).await.unwrap();
        teams.into_iter().find(|t| t.role == "owner").unwrap().id
    }

    #[test]
    fn email_normalization() {
        assert_eq!(normalize_email("  Paul@Example.COM "), Some("paul@example.com".into()));
        assert_eq!(normalize_email("a.b+c@sub.example.io"), Some("a.b+c@sub.example.io".into()));
        for bad in ["", "paul", "paul@", "@example.com", "paul@example", "pa ul@example.com", "a@b@c.com", "x@.com", "x@a..com"] {
            assert_eq!(normalize_email(bad), None, "{bad:?} should be rejected");
        }
    }

    #[test]
    fn handle_derivation() {
        assert_eq!(handle_base_from_email("Paul@x.com"), "paul");
        assert_eq!(handle_base_from_email("paul.smith+work@x.com"), "paul-smith");
        assert_eq!(handle_base_from_email("__weird__.._name@x.com"), "weird-name");
        assert_eq!(handle_base_from_email("abcdefghijklmnopqrstuvwxyz@x.com"), "abcdefghijklmnopqrst");
        assert_eq!(handle_base_from_email("josé@x.com"), "jos");
        assert_eq!(handle_base_from_email("+++@x.com"), "user");
        assert_eq!(handle_base_from_email("___@x.com"), "user");
    }

    #[test]
    fn handle_validation() {
        assert_eq!(validate_handle(" Paul-2 ").unwrap(), "paul-2");
        for bad in ["", "-paul", "paul-", "pa ul", "paul/x", "@paul", "abcdefghijklmnopqrstu", "all"] {
            assert!(validate_handle(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[tokio::test]
    async fn signup_creates_team_and_default_boards() {
        let db = test_pool().await;
        let (user, created) = find_or_create_user(&db, "Paul@Example.com").await.unwrap();
        assert!(created);
        assert_eq!(user.email, "paul@example.com");
        assert_eq!(user.handle, "paul");
        assert!(user.id.starts_with("usr_"));

        let teams = list_user_teams(&db, &user.id).await.unwrap();
        assert_eq!(teams.len(), 1);
        assert_eq!(teams[0].name, "paul's board");
        assert_eq!(teams[0].role, "owner");
        assert_eq!(teams[0].member_count, 1);
        assert_eq!(teams[0].max_members, 5);

        let mut keys: Vec<String> = sqlx::query_scalar("SELECT key FROM boards WHERE team_id = ? ORDER BY key")
            .bind(&teams[0].id)
            .fetch_all(&db)
            .await
            .unwrap();
        keys.sort();
        assert_eq!(keys, vec!["backend", "ci", "general", "ui"]);

        // Same email (any case) finds the same user without a second team.
        let (again, created) = find_or_create_user(&db, " PAUL@example.com").await.unwrap();
        assert!(!created);
        assert_eq!(again, user);
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM teams").fetch_one(&db).await.unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn handle_dedupe() {
        let db = test_pool().await;
        assert_eq!(signup(&db, "paul@a.com").await.handle, "paul");
        assert_eq!(signup(&db, "paul@b.com").await.handle, "paul2");
        assert_eq!(signup(&db, "Paul+x@c.com").await.handle, "paul3");
        // Reserved handles are skipped.
        assert_eq!(signup(&db, "all@a.com").await.handle, "all2");
        // Long bases are truncated to leave room for the suffix.
        assert_eq!(signup(&db, "abcdefghijklmnopqrstuvwxyz@a.com").await.handle, "abcdefghijklmnopqrst");
        assert_eq!(signup(&db, "abcdefghijklmnopqrstuvwxyz@b.com").await.handle, "abcdefghijklmnopqrs2");

        // Renaming: taken handles conflict, free ones work.
        let u = signup(&db, "zed@a.com").await;
        assert!(matches!(update_handle(&db, &u.id, "PAUL").await, Err(AppError::Conflict(_))));
        assert_eq!(update_handle(&db, &u.id, "zed-two").await.unwrap().handle, "zed-two");
        // A new "zed" signup can now have the plain handle.
        assert_eq!(signup(&db, "zed@b.com").await.handle, "zed");
    }

    #[tokio::test]
    async fn login_token_single_use_and_expiry() {
        let db = test_pool().await;
        let secret = create_login_token(&db, "paul@example.com", Some("INV123")).await.unwrap();
        assert_eq!(secret.len(), 32);
        // Only the hash is stored.
        let stored: String = sqlx::query_scalar("SELECT token_hash FROM login_tokens").fetch_one(&db).await.unwrap();
        assert_eq!(stored, sha256_hex(&secret));
        assert_ne!(stored, secret);

        let claim = consume_login_token(&db, &secret).await.unwrap().unwrap();
        assert_eq!(claim, LoginClaim { email: "paul@example.com".into(), invite_code: Some("INV123".into()) });
        assert_eq!(consume_login_token(&db, &secret).await.unwrap(), None, "second use must fail");
        assert_eq!(consume_login_token(&db, "nope").await.unwrap(), None);
        assert_eq!(consume_login_token(&db, "").await.unwrap(), None);

        let expired = create_login_token(&db, "paul@example.com", None).await.unwrap();
        sqlx::query("UPDATE login_tokens SET expires_at = ? WHERE token_hash = ?")
            .bind(now_ms() - 1)
            .bind(sha256_hex(&expired))
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(consume_login_token(&db, &expired).await.unwrap(), None, "expired token must fail");

        let fresh = create_login_token(&db, "x@example.com", None).await.unwrap();
        let expires: i64 = sqlx::query_scalar("SELECT expires_at - created_at FROM login_tokens WHERE token_hash = ?")
            .bind(sha256_hex(&fresh))
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(expires, 20 * 60 * 1000);
    }

    #[tokio::test]
    async fn session_lookup_and_expiry() {
        let db = test_pool().await;
        let user = signup(&db, "paul@example.com").await;
        let secret = create_session(&db, &user.id).await.unwrap();
        assert_eq!(session_user(&db, &secret).await.unwrap(), Some(user.clone()));
        assert_eq!(session_user(&db, "bogus").await.unwrap(), None);
        assert_eq!(session_user(&db, "").await.unwrap(), None);

        let ttl: i64 = sqlx::query_scalar("SELECT expires_at - created_at FROM sessions WHERE token_hash = ?")
            .bind(sha256_hex(&secret))
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(ttl, 30 * 24 * 60 * 60 * 1000);

        sqlx::query("UPDATE sessions SET expires_at = ?").bind(now_ms() - 1).execute(&db).await.unwrap();
        assert_eq!(session_user(&db, &secret).await.unwrap(), None, "expired session");

        let s2 = create_session(&db, &user.id).await.unwrap();
        delete_session(&db, &s2).await.unwrap();
        assert_eq!(session_user(&db, &s2).await.unwrap(), None, "logged out");
    }

    #[tokio::test]
    async fn invite_cap_rotation_and_idempotency() {
        let db = test_pool().await;
        let owner = signup(&db, "owner@example.com").await;
        let team = personal_team(&db, &owner).await;
        let code = current_invite(&db, &team, &owner.id).await.unwrap();
        assert_eq!(current_invite(&db, &team, &owner.id).await.unwrap(), code, "stable until rotated");

        let info = invite_info(&db, &code).await.unwrap().unwrap();
        assert_eq!((info.team_name.as_str(), info.inviter_handle.as_str(), info.member_count, info.full), ("owner's board", "owner", 1, false));

        let mut members = vec![];
        for i in 1..=4 {
            let u = signup(&db, &format!("m{i}@example.com")).await;
            let (tid, outcome) = join_via_invite(&db, &code, &u.id).await.unwrap();
            assert_eq!((tid.as_str(), outcome), (team.as_str(), JoinOutcome::Joined));
            members.push(u);
        }
        // Already a member: idempotent, even when full.
        assert_eq!(join_via_invite(&db, &code, &members[0].id).await.unwrap().1, JoinOutcome::AlreadyMember);
        assert_eq!(join_via_invite(&db, &code, &owner.id).await.unwrap().1, JoinOutcome::AlreadyMember);

        let sixth = signup(&db, "m5@example.com").await;
        assert!(matches!(join_via_invite(&db, &code, &sixth.id).await, Err(AppError::Conflict(_))));
        assert!(invite_info(&db, &code).await.unwrap().unwrap().full);
        assert_eq!(list_members(&db, &team).await.unwrap().len(), 5);

        let joined: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE team_id = ? AND kind = 'member_joined'")
            .bind(&team)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(joined, 4);

        // Rotation revokes the old code.
        let new_code = rotate_invite(&db, &team, &members[1].id).await.unwrap();
        assert_ne!(new_code, code);
        assert!(invite_info(&db, &code).await.unwrap().is_none());
        assert!(matches!(join_via_invite(&db, &code, &sixth.id).await, Err(AppError::NotFound(_))));
        assert_eq!(current_invite(&db, &team, &owner.id).await.unwrap(), new_code);
        assert_eq!(invite_info(&db, &new_code).await.unwrap().unwrap().inviter_handle, "m2");

        // A member leaves; now there's room.
        remove_member(&db, &team, &members[3].id, &members[3].id).await.unwrap();
        assert_eq!(join_via_invite(&db, &new_code, &sixth.id).await.unwrap().1, JoinOutcome::Joined);
    }

    /// Concurrent joins against a real file database with several connections must never exceed 5.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn invite_cap_holds_under_concurrency() {
        let dir = std::env::temp_dir().join(format!("swarmboard-test-{}", random_token(8)));
        let cfg = Config {
            bind: "127.0.0.1:0".into(),
            database_url: format!("sqlite://{}/test.db", dir.display()),
            data_dir: dir.clone(),
            web_url: "http://localhost:5191".into(),
            public_api_url: "http://localhost:8191".into(),
            resend_api_key: None,
            email_from: "x <x@example.com>".into(),
            cookie_secure: false,
        };
        let db = crate::db::connect(&cfg).await.unwrap();
        let owner = signup(&db, "owner@example.com").await;
        let team = personal_team(&db, &owner).await;
        let code = current_invite(&db, &team, &owner.id).await.unwrap();
        let mut users = vec![];
        for i in 0..12 {
            users.push(signup(&db, &format!("u{i}@example.com")).await);
        }
        let handles: Vec<_> = users
            .into_iter()
            .map(|u| {
                let (db, code) = (db.clone(), code.clone());
                tokio::spawn(async move { join_via_invite(&db, &code, &u.id).await })
            })
            .collect();
        let mut ok = 0;
        let mut full = 0;
        for h in handles {
            match h.await.unwrap() {
                Ok((_, JoinOutcome::Joined)) => ok += 1,
                Err(AppError::Conflict(_)) => full += 1,
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!((ok, full), (4, 8));
        assert_eq!(list_members(&db, &team).await.unwrap().len(), 5);
        db.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn member_removal_rules() {
        let db = test_pool().await;
        let owner = signup(&db, "owner@example.com").await;
        let team = personal_team(&db, &owner).await;
        let code = current_invite(&db, &team, &owner.id).await.unwrap();
        let a = signup(&db, "a@example.com").await;
        let b = signup(&db, "b@example.com").await;
        join_via_invite(&db, &code, &a.id).await.unwrap();
        join_via_invite(&db, &code, &b.id).await.unwrap();
        let tok = create_api_token(&db, &b.id, &team, Some("laptop")).await.unwrap();
        assert!(authenticate_token(&db, &tok.token).await.unwrap().is_some());

        assert!(matches!(remove_member(&db, &team, &a.id, &b.id).await, Err(AppError::Forbidden(_))), "member can't remove others");
        assert!(matches!(remove_member(&db, &team, &owner.id, &owner.id).await, Err(AppError::Forbidden(_))), "owner can't leave");
        assert!(matches!(remove_member(&db, &team, &a.id, &owner.id).await, Err(AppError::Forbidden(_))));
        remove_member(&db, &team, &owner.id, &b.id).await.unwrap();
        assert_eq!(member_role(&db, &team, &b.id).await.unwrap(), None);
        assert!(authenticate_token(&db, &tok.token).await.unwrap().is_none(), "removed member's token stops working");
        assert!(matches!(remove_member(&db, &team, &owner.id, &b.id).await, Err(AppError::NotFound(_))));
        remove_member(&db, &team, &a.id, &a.id).await.unwrap();
        assert_eq!(list_members(&db, &team).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn api_tokens_and_authenticate() {
        let db = test_pool().await;
        let user = signup(&db, "paul@example.com").await;
        let team = personal_team(&db, &user).await;

        let tok = create_api_token(&db, &user.id, &team, Some("  laptop ")).await.unwrap();
        assert!(tok.token.starts_with("sb_"));
        assert_eq!(tok.token.len(), 43);
        assert!(tok.token[3..].chars().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(tok.prefix, &tok.token[..11]);
        assert_eq!(tok.label, "laptop");
        assert!(tok.id.starts_with("tok_"));
        let stored: String = sqlx::query_scalar("SELECT token_hash FROM api_tokens WHERE id = ?")
            .bind(&tok.id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(stored, sha256_hex(&tok.token));

        assert_eq!(create_api_token(&db, &user.id, &team, None).await.unwrap().label, "token");

        let auth = authenticate_token(&db, &tok.token).await.unwrap().unwrap();
        assert_eq!((auth.token_id.as_str(), auth.user_id.as_str(), auth.team_id.as_str(), auth.handle.as_str()), (tok.id.as_str(), user.id.as_str(), team.as_str(), "paul"));
        assert!(authenticate_token(&db, &format!(" {} ", tok.token)).await.unwrap().is_some(), "surrounding whitespace ok");
        assert!(authenticate_token(&db, "sb_wrong").await.unwrap().is_none());
        assert!(authenticate_token(&db, "nope").await.unwrap().is_none());
        assert!(authenticate_token(&db, "").await.unwrap().is_none());

        // last_used_at is set on first use and then throttled.
        let lu1: Option<i64> = sqlx::query_scalar("SELECT last_used_at FROM api_tokens WHERE id = ?").bind(&tok.id).fetch_one(&db).await.unwrap();
        assert!(lu1.is_some());
        sqlx::query("UPDATE api_tokens SET last_used_at = 5 WHERE id = ?").bind(&tok.id).execute(&db).await.unwrap();
        authenticate_token(&db, &tok.token).await.unwrap();
        let lu2: Option<i64> = sqlx::query_scalar("SELECT last_used_at FROM api_tokens WHERE id = ?").bind(&tok.id).fetch_one(&db).await.unwrap();
        assert!(lu2.unwrap() > 5, "stale last_used_at is refreshed");
        authenticate_token(&db, &tok.token).await.unwrap();
        let lu3: Option<i64> = sqlx::query_scalar("SELECT last_used_at FROM api_tokens WHERE id = ?").bind(&tok.id).fetch_one(&db).await.unwrap();
        assert_eq!(lu3, lu2, "fresh last_used_at is not rewritten");

        let listed = list_api_tokens(&db, &user.id, &team).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|t| t.id == tok.id && t.prefix == tok.prefix));

        // Only the owner can revoke; revoked tokens stop authenticating and disappear from the list.
        let other = signup(&db, "other@example.com").await;
        assert!(matches!(revoke_api_token(&db, &other.id, &tok.id).await, Err(AppError::NotFound(_))));
        revoke_api_token(&db, &user.id, &tok.id).await.unwrap();
        assert!(authenticate_token(&db, &tok.token).await.unwrap().is_none());
        assert_eq!(list_api_tokens(&db, &user.id, &team).await.unwrap().len(), 1);
        assert!(matches!(revoke_api_token(&db, &user.id, &tok.id).await, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn rename_team_validation() {
        let db = test_pool().await;
        let user = signup(&db, "paul@example.com").await;
        let team = personal_team(&db, &user).await;
        assert_eq!(rename_team(&db, &team, "  swarm  ").await.unwrap().name, "swarm");
        assert!(matches!(rename_team(&db, &team, "   ").await, Err(AppError::BadRequest(_))));
        assert!(matches!(rename_team(&db, &team, &"x".repeat(61)).await, Err(AppError::BadRequest(_))));
    }
}
