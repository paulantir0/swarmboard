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
    domain::collab::leases,
    error::{AppError, AppResult},
    models::TokenAuth,
    util::{new_id, now_ms, random_token, sha256_hex},
};

/// Magic links expire after 20 minutes.
pub const LOGIN_TOKEN_TTL_MS: i64 = 20 * 60 * 1000;
/// Browser sessions last 30 days.
pub const SESSION_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// Boards every new personal team starts with.
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
/// Org slugs: swarmboard.app/<slug>.
pub const SLUG_MIN_LEN: usize = 2;
pub const SLUG_MAX_LEN: usize = 32;
/// Shown in "swarmboard.app/<slug> is taken".
pub const ORG_URL_HOST: &str = "swarmboard.app";
/// Paths the web app and API use at the top level, so they can't be org slugs. "slug" also
/// keeps GET /api/orgs/slug (the availability check) from shadowing an org.
pub const RESERVED_SLUGS: &[&str] = &[
    "app", "api", "auth", "join", "new", "orgs", "org", "f", "u", "p", "mcp", "healthz", "assets", "brand", "fonts", "crew",
    "login", "logout", "signup", "settings", "admin", "www", "docs", "help", "about", "terms", "privacy", "static", "invite",
    "connect", "github", "slug",
];
/// Orgs one user may own.
pub const MAX_OWNED_ORGS: i64 = 5;
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
    pub slug: String,
    pub owner_id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Admin,
    Member,
}

impl Role {
    fn parse(s: &str) -> Role {
        match s {
            "owner" => Role::Owner,
            "admin" => Role::Admin,
            _ => Role::Member,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }

    /// Owner or admin: may rename the team, rotate the invite, remove members, force-release leases.
    pub fn is_admin(self) -> bool {
        matches!(self, Role::Owner | Role::Admin)
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TeamSummary {
    pub id: String,
    pub name: String,
    pub slug: String,
    /// "owner" | "admin" | "member"
    pub role: String,
    pub member_count: i64,
    pub max_members: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Member {
    pub user_id: String,
    pub email: String,
    pub handle: String,
    /// "owner" | "admin" | "member"
    pub role: String,
    pub joined_at: i64,
}

/// Public view of an invite link (shown on the /join page before signing in).
#[derive(Debug, Clone, Serialize)]
pub struct InviteInfo {
    pub team_name: String,
    pub team_slug: String,
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

/// Finds the user for `email`, or creates them (with no orgs: they name one, or join via invite).
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
    tx.commit().await?;
    tracing::info!(user_id = %user.id, handle = %user.handle, "user created");
    Ok((user, true))
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
        "SELECT t.id, t.name, t.slug, m.role, \
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
    role_in(db, team_id, user_id).await
}

pub async fn get_team(db: &SqlitePool, team_id: &str) -> AppResult<Option<Team>> {
    Ok(sqlx::query_as::<_, Team>("SELECT id, name, slug, owner_id, created_at FROM teams WHERE id = ?")
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

fn validate_team_name(raw: &str) -> AppResult<String> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > TEAM_NAME_MAX_LEN {
        return Err(AppError::BadRequest(format!("name must be 1-{TEAM_NAME_MAX_LEN} characters")));
    }
    Ok(name.to_string())
}

/// Owner/admin may rename; only the owner may change the slug (a no-op slug passes for anyone).
/// Records `org_renamed` with whichever of `name` / `slug` actually changed.
pub async fn update_team(
    db: &SqlitePool,
    team_id: &str,
    actor_id: &str,
    raw_name: Option<&str>,
    raw_slug: Option<&str>,
) -> AppResult<Team> {
    let name = raw_name.map(validate_team_name).transpose()?;
    let slug = raw_slug.map(validate_slug).transpose()?;
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let role = role_in(&mut *tx, team_id, actor_id)
        .await?
        .ok_or_else(|| AppError::Forbidden("not a member of this org".into()))?;
    let (cur_name, cur_slug): (String, String) = sqlx::query_as("SELECT name, slug FROM teams WHERE id = ?")
        .bind(team_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound("org not found".into()))?;
    let name = name.filter(|n| *n != cur_name);
    let slug = slug.filter(|s| *s != cur_slug);
    if (name.is_some() || slug.is_some()) && !role.is_admin() {
        return Err(AppError::Forbidden("only the org owner or an admin can do that".into()));
    }
    if slug.is_some() && role != Role::Owner {
        return Err(AppError::Forbidden("only the org owner can change its URL".into()));
    }
    if let Some(n) = &name {
        sqlx::query("UPDATE teams SET name = ? WHERE id = ?").bind(n).bind(team_id).execute(&mut *tx).await?;
    }
    if let Some(sl) = &slug {
        let res = sqlx::query("UPDATE teams SET slug = ? WHERE id = ?").bind(sl).bind(team_id).execute(&mut *tx).await;
        match res {
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => return Err(slug_taken(sl)),
            other => {
                other?;
            }
        }
    }
    if name.is_some() || slug.is_some() {
        let mut data = serde_json::Map::new();
        if let Some(n) = &name {
            data.insert("name".into(), n.clone().into());
        }
        if let Some(sl) = &slug {
            data.insert("slug".into(), sl.clone().into());
        }
        insert_event(&mut *tx, team_id, actor_id, "org_renamed", data.into()).await?;
    }
    tx.commit().await?;
    get_team(db, team_id).await?.ok_or_else(|| AppError::NotFound("org not found".into()))
}

// ---------------------------------------------------------------------------------------------
// Orgs (user-facing name for teams): slugs, creation, lookup by slug
// ---------------------------------------------------------------------------------------------

/// Lenient form of what someone typed: ASCII-lowercased, runs of anything outside `[a-z0-9]`
/// collapsed to one '-', trimmed of '-'. Not length-checked ("Acme Corp!" -> "acme-corp").
pub fn normalize_slug(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Checks an already-normalized slug: 2-32 of `[a-z0-9-]`, no leading/trailing/double '-', not reserved.
pub fn check_slug(slug: &str) -> AppResult<()> {
    let ok_chars = slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if slug.len() < SLUG_MIN_LEN
        || slug.len() > SLUG_MAX_LEN
        || !ok_chars
        || slug.starts_with('-')
        || slug.ends_with('-')
        || slug.contains("--")
    {
        return Err(AppError::BadRequest(format!("URL must be {SLUG_MIN_LEN}-{SLUG_MAX_LEN} characters: a-z, 0-9, -")));
    }
    if RESERVED_SLUGS.contains(&slug) {
        return Err(AppError::BadRequest(format!("{ORG_URL_HOST}/{slug} is reserved")));
    }
    Ok(())
}

/// `normalize_slug` then `check_slug`; the normalized slug on success.
pub fn validate_slug(raw: &str) -> AppResult<String> {
    let slug = normalize_slug(raw);
    check_slug(&slug)?;
    Ok(slug)
}

/// Slug derived from an org name: normalized and cut to 32 chars on a clean boundary.
pub fn slug_from_name(name: &str) -> String {
    let slug = normalize_slug(name);
    slug[..slug.len().min(SLUG_MAX_LEN)].trim_end_matches('-').to_string()
}

fn slug_taken(slug: &str) -> AppError {
    AppError::Conflict(format!("{ORG_URL_HOST}/{slug} is taken"))
}

/// Live availability for the "name your org" form.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SlugAvailability {
    pub slug: String,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub async fn slug_availability(db: &SqlitePool, raw: &str) -> AppResult<SlugAvailability> {
    let slug = normalize_slug(raw);
    let reason = match check_slug(&slug) {
        Err(e) => Some(e.to_string()),
        Ok(()) => {
            let taken: Option<i64> = sqlx::query_scalar("SELECT 1 FROM teams WHERE slug = ?").bind(&slug).fetch_optional(db).await?;
            taken.map(|_| slug_taken(&slug).to_string())
        }
    };
    Ok(SlugAvailability { available: reason.is_none(), slug, reason })
}

/// The user's view of one org they belong to; None if they aren't a member.
pub async fn team_summary(db: &SqlitePool, team_id: &str, user_id: &str) -> AppResult<Option<TeamSummary>> {
    Ok(sqlx::query_as::<_, TeamSummary>(
        "SELECT t.id, t.name, t.slug, m.role, \
                (SELECT COUNT(*) FROM memberships m2 WHERE m2.team_id = t.id) AS member_count, \
                ? AS max_members \
         FROM memberships m JOIN teams t ON t.id = m.team_id WHERE t.id = ? AND m.user_id = ?",
    )
    .bind(MAX_TEAM_MEMBERS)
    .bind(team_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?)
}

/// Like `team_summary`, by slug (any case). None for unknown slugs and non-members alike.
pub async fn org_by_slug(db: &SqlitePool, raw_slug: &str, user_id: &str) -> AppResult<Option<TeamSummary>> {
    let slug = raw_slug.trim();
    if slug.is_empty() || slug.len() > SLUG_MAX_LEN {
        return Ok(None);
    }
    let id: Option<String> = sqlx::query_scalar("SELECT id FROM teams WHERE slug = ?").bind(slug).fetch_optional(db).await?;
    match id {
        Some(id) => team_summary(db, &id, user_id).await,
        None => Ok(None),
    }
}

/// Creates an org owned by `user_id` (slug from `raw_slug`, else derived from the name).
/// BadRequest for bad names/slugs or past `MAX_OWNED_ORGS`; Conflict when the slug is taken.
pub async fn create_org(db: &SqlitePool, user_id: &str, raw_name: &str, raw_slug: Option<&str>) -> AppResult<TeamSummary> {
    let name = validate_team_name(raw_name)?;
    let slug = match raw_slug.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => validate_slug(raw)?,
        None => {
            let slug = slug_from_name(&name);
            check_slug(&slug).map_err(|e| AppError::BadRequest(format!("choose a URL for this org ({e})")))?;
            slug
        }
    };
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let owned: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memberships WHERE user_id = ? AND role = 'owner'")
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
    if owned >= MAX_OWNED_ORGS {
        return Err(AppError::BadRequest(format!("you can own at most {MAX_OWNED_ORGS} orgs")));
    }
    let now = now_ms();
    let team_id = new_id("tm");
    let res = sqlx::query("INSERT INTO teams (id, name, slug, owner_id, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&team_id)
        .bind(&name)
        .bind(&slug)
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await;
    match res {
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => return Err(slug_taken(&slug)),
        other => {
            other?;
        }
    }
    sqlx::query("INSERT INTO memberships (team_id, user_id, role, joined_at) VALUES (?, ?, 'owner', ?)")
        .bind(&team_id)
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    insert_event(&mut *tx, &team_id, user_id, "org_created", serde_json::json!({ "name": name, "slug": slug })).await?;
    tx.commit().await?;
    tracing::info!(%team_id, %slug, %user_id, "org created");
    team_summary(db, &team_id, user_id).await?.ok_or_else(|| AppError::NotFound("org not found".into()))
}

async fn handle_of<'e>(ex: impl SqliteExecutor<'e>, user_id: &str) -> AppResult<String> {
    Ok(sqlx::query_scalar("SELECT handle FROM users WHERE id = ?").bind(user_id).fetch_optional(ex).await?.unwrap_or_default())
}

async fn role_in<'e>(ex: impl SqliteExecutor<'e>, team_id: &str, user_id: &str) -> AppResult<Option<Role>> {
    let role: Option<String> = sqlx::query_scalar("SELECT role FROM memberships WHERE team_id = ? AND user_id = ?")
        .bind(team_id)
        .bind(user_id)
        .fetch_optional(ex)
        .await?;
    Ok(role.map(|r| Role::parse(&r)))
}

async fn insert_event<'e>(ex: impl SqliteExecutor<'e>, team_id: &str, actor_id: &str, kind: &str, data: serde_json::Value) -> AppResult<()> {
    sqlx::query("INSERT INTO events (team_id, at, actor_user_id, kind, data) VALUES (?, ?, ?, ?, ?)")
        .bind(team_id)
        .bind(now_ms())
        .bind(actor_id)
        .bind(kind)
        .bind(data.to_string())
        .execute(ex)
        .await?;
    Ok(())
}

pub async fn get_member(db: &SqlitePool, team_id: &str, user_id: &str) -> AppResult<Option<Member>> {
    Ok(sqlx::query_as::<_, Member>(
        "SELECT m.user_id, u.email, u.handle, m.role, m.joined_at \
         FROM memberships m JOIN users u ON u.id = m.user_id WHERE m.team_id = ? AND m.user_id = ?",
    )
    .bind(team_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?)
}

/// Owner only: make a member an admin or back. The owner's own role changes only via transfer.
pub async fn set_role(db: &SqlitePool, team_id: &str, actor_id: &str, target_id: &str, raw_role: &str) -> AppResult<Member> {
    let role = match raw_role.trim() {
        "admin" => Role::Admin,
        "member" => Role::Member,
        "owner" => return Err(AppError::BadRequest("use transfer to make someone the owner".into())),
        _ => return Err(AppError::BadRequest("role must be \"admin\" or \"member\"".into())),
    };
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    match role_in(&mut *tx, team_id, actor_id).await? {
        Some(Role::Owner) => {}
        Some(_) => return Err(AppError::Forbidden("only the team owner can change roles".into())),
        None => return Err(AppError::Forbidden("not a member of this team".into())),
    }
    let current = role_in(&mut *tx, team_id, target_id)
        .await?
        .ok_or_else(|| AppError::NotFound("not a member of this team".into()))?;
    if current == Role::Owner {
        return Err(AppError::BadRequest("the owner's role can't be changed; transfer ownership instead".into()));
    }
    if current != role {
        sqlx::query("UPDATE memberships SET role = ? WHERE team_id = ? AND user_id = ?")
            .bind(role.as_str())
            .bind(team_id)
            .bind(target_id)
            .execute(&mut *tx)
            .await?;
        let handle = handle_of(&mut *tx, target_id).await?;
        let data = serde_json::json!({ "user": handle, "user_id": target_id, "role": role.as_str() });
        insert_event(&mut *tx, team_id, actor_id, "role_changed", data).await?;
    }
    tx.commit().await?;
    get_member(db, team_id, target_id).await?.ok_or_else(|| AppError::NotFound("not a member of this team".into()))
}

/// Owner only: `target_id` (a member) becomes owner, the previous owner becomes admin, and
/// teams.owner_id follows. One transaction under the write lock.
pub async fn transfer_ownership(db: &SqlitePool, team_id: &str, actor_id: &str, target_id: &str) -> AppResult<()> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    match role_in(&mut *tx, team_id, actor_id).await? {
        Some(Role::Owner) => {}
        Some(_) => return Err(AppError::Forbidden("only the team owner can transfer ownership".into())),
        None => return Err(AppError::Forbidden("not a member of this team".into())),
    }
    if target_id == actor_id {
        return Err(AppError::BadRequest("you already own this team".into()));
    }
    if role_in(&mut *tx, team_id, target_id).await?.is_none() {
        return Err(AppError::BadRequest("the new owner must be a member of this team".into()));
    }
    // Demote first: at most one owner row per team (unique index).
    sqlx::query("UPDATE memberships SET role = 'admin' WHERE team_id = ? AND user_id = ?")
        .bind(team_id)
        .bind(actor_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE memberships SET role = 'owner' WHERE team_id = ? AND user_id = ?")
        .bind(team_id)
        .bind(target_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE teams SET owner_id = ? WHERE id = ?").bind(target_id).bind(team_id).execute(&mut *tx).await?;
    let from = handle_of(&mut *tx, actor_id).await?;
    let to = handle_of(&mut *tx, target_id).await?;
    let data = serde_json::json!({ "from": from, "to": to, "from_id": actor_id, "to_id": target_id });
    insert_event(&mut *tx, team_id, actor_id, "ownership_transferred", data).await?;
    tx.commit().await?;
    tracing::info!(%team_id, from = %actor_id, to = %target_id, "ownership transferred");
    Ok(())
}

/// Who may remove whom: the owner removes anyone else; an admin removes plain members; anyone
/// may leave, except the owner (who must transfer ownership first).
fn check_removal(actor: Role, target: Role, is_self: bool) -> AppResult<()> {
    if is_self {
        return match actor {
            Role::Owner => Err(AppError::BadRequest("transfer ownership first".into())),
            _ => Ok(()),
        };
    }
    match (actor, target) {
        (Role::Owner, _) => Ok(()),
        (Role::Admin, Role::Member) => Ok(()),
        (Role::Admin, _) => Err(AppError::Forbidden("admins can only remove members, not admins or the owner".into())),
        (Role::Member, _) => Err(AppError::Forbidden("only the owner or an admin can remove members".into())),
    }
}

/// Removes `target_id` from the team (see `check_removal`), revokes their API tokens for it and
/// frees their agents' leases there (granting to waiters).
pub async fn remove_member(db: &SqlitePool, team_id: &str, actor_id: &str, target_id: &str) -> AppResult<()> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let actor_role = role_in(&mut *tx, team_id, actor_id)
        .await?
        .ok_or_else(|| AppError::Forbidden("not a member of this team".into()))?;
    let target_role = role_in(&mut *tx, team_id, target_id)
        .await?
        .ok_or_else(|| AppError::NotFound("not a member of this team".into()))?;
    check_removal(actor_role, target_role, actor_id == target_id)?;
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
    let user = handle_of(&mut *tx, target_id).await?;
    let by = handle_of(&mut *tx, actor_id).await?;
    let data = serde_json::json!({ "user": user, "by": by, "user_id": target_id, "by_id": actor_id });
    insert_event(&mut *tx, team_id, actor_id, "member_removed", data).await?;
    tx.commit().await?;
    // The membership is gone, so their tokens no longer authenticate and their agents can't re-claim.
    let actor = crate::models::Actor { team_id: team_id.into(), user_id: actor_id.into(), agent_id: None };
    leases::release_user(db, team_id, target_id, &actor, &by).await?;
    tracing::info!(%team_id, %target_id, by = %actor_id, "member removed");
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
    let row: Option<(String, String, String, i64)> = sqlx::query_as(
        "SELECT t.name, t.slug, u.handle, (SELECT COUNT(*) FROM memberships m WHERE m.team_id = t.id) \
         FROM invites i JOIN teams t ON t.id = i.team_id JOIN users u ON u.id = i.created_by \
         WHERE i.code = ? AND i.revoked_at IS NULL",
    )
    .bind(code)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(team_name, team_slug, inviter_handle, member_count)| InviteInfo {
        team_name,
        team_slug,
        inviter_handle,
        member_count,
        max_members: MAX_TEAM_MEMBERS,
        full: member_count >= MAX_TEAM_MEMBERS,
    }))
}

/// Before emailing an invite to `email` (normalized): Conflict if that address already belongs
/// to a member or the team is full.
pub async fn check_email_invite(db: &SqlitePool, team_id: &str, email: &str) -> AppResult<()> {
    let (member_count, already): (i64, bool) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(MAX(u.email = ?2), 0) \
         FROM memberships m JOIN users u ON u.id = m.user_id WHERE m.team_id = ?1",
    )
    .bind(team_id)
    .bind(email)
    .fetch_one(db)
    .await?;
    if already {
        return Err(AppError::Conflict("already a member".into()));
    }
    if member_count >= MAX_TEAM_MEMBERS {
        return Err(AppError::Conflict(format!("this team is full ({MAX_TEAM_MEMBERS} people)")));
    }
    Ok(())
}

/// Records `invite_sent` with only the invitee's email domain, never the full address.
pub async fn record_invite_sent(db: &SqlitePool, team_id: &str, actor: &User, email: &str) -> AppResult<()> {
    let domain = email.rsplit_once('@').map(|(_, d)| d).unwrap_or("");
    let data = serde_json::json!({ "by": actor.handle, "by_id": actor.id, "email_domain": domain });
    insert_event(db, team_id, &actor.id, "invite_sent", data).await
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

    /// An org "<handle>'s board" at /<handle>, owned by `user`.
    async fn personal_team(db: &SqlitePool, user: &User) -> String {
        create_org(db, &user.id, &format!("{}'s board", user.handle), Some(&user.handle)).await.unwrap().id
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
    async fn signup_creates_user_without_org() {
        let db = test_pool().await;
        let (user, created) = find_or_create_user(&db, "Paul@Example.com").await.unwrap();
        assert!(created);
        assert_eq!(user.email, "paul@example.com");
        assert_eq!(user.handle, "paul");
        assert!(user.id.starts_with("usr_"));
        assert!(list_user_teams(&db, &user.id).await.unwrap().is_empty(), "a new user has zero orgs");

        // Same email (any case) finds the same user.
        let (again, created) = find_or_create_user(&db, " PAUL@example.com").await.unwrap();
        assert!(!created);
        assert_eq!(again, user);
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM teams").fetch_one(&db).await.unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn slug_normalization_and_validation() {
        assert_eq!(normalize_slug("  Acme Corp! "), "acme-corp");
        assert_eq!(normalize_slug("--a__b--c--"), "a-b-c");
        assert_eq!(normalize_slug("Zoë's Board"), "zo-s-board");
        assert_eq!(normalize_slug("!!!"), "");
        assert_eq!(validate_slug("Acme").unwrap(), "acme");
        assert_eq!(validate_slug("my-org-2").unwrap(), "my-org-2");
        assert_eq!(validate_slug(&"a".repeat(32)).unwrap().len(), 32);
        for bad in ["", "a", "-", "!!", &"a".repeat(33)] {
            assert!(matches!(validate_slug(bad), Err(AppError::BadRequest(_))), "{bad:?} should be rejected");
        }
        // The strict check rejects what normalization would have fixed.
        for bad in ["-ab", "ab-", "a--b", "Ab", "a b", "a_b"] {
            assert!(check_slug(bad).is_err(), "{bad:?} should fail check_slug");
        }
        for r in RESERVED_SLUGS {
            match validate_slug(&r.to_uppercase()) {
                Err(AppError::BadRequest(m)) if r.len() >= 2 => assert_eq!(m, format!("swarmboard.app/{r} is reserved")),
                Err(AppError::BadRequest(_)) => {}
                other => panic!("{r}: {other:?}"),
            }
        }
        assert_eq!(slug_from_name("Paul's Board"), "paul-s-board");
        assert_eq!(slug_from_name(&format!("{}-{}", "a".repeat(31), "bcd")), "a".repeat(31), "cut on a clean boundary");
    }

    #[tokio::test]
    async fn create_org_slugs_cap_and_events() {
        let db = test_pool().await;
        let paul = signup(&db, "paul@example.com").await;
        let zed = signup(&db, "zed@example.com").await;

        let org = create_org(&db, &paul.id, "  Acme Corp ", None).await.unwrap();
        assert_eq!((org.name.as_str(), org.slug.as_str(), org.role.as_str(), org.member_count, org.max_members), ("Acme Corp", "acme-corp", "owner", 1, 5));
        assert_eq!(list_user_teams(&db, &paul.id).await.unwrap()[0].slug, "acme-corp");
        let ev = event_data(&db, &org.id, "org_created").await;
        assert_eq!(ev, vec![serde_json::json!({ "name": "Acme Corp", "slug": "acme-corp" })]);

        // Taken, ignoring case, whether explicit or derived.
        for (name, slug) in [("Other", Some("ACME-corp")), ("acme corp", None)] {
            match create_org(&db, &zed.id, name, slug).await {
                Err(AppError::Conflict(m)) => assert_eq!(m, "swarmboard.app/acme-corp is taken"),
                other => panic!("{other:?}"),
            }
        }
        assert!(sqlx::query("INSERT INTO teams (id, name, slug, owner_id, created_at) VALUES ('tm_x', 'x', 'acme-corp', ?, 0)")
            .bind(&zed.id)
            .execute(&db)
            .await
            .is_err(), "unique index");
        assert!(sqlx::query("UPDATE teams SET slug = 'Acme-Corp'").execute(&db).await.is_err(), "stored slugs are lowercase");
        assert!(sqlx::query("INSERT INTO teams (id, name, owner_id, created_at) VALUES ('tm_y', 'y', ?, 0)")
            .bind(&zed.id)
            .execute(&db)
            .await
            .is_err(), "slug is required");

        // Bad names and slugs.
        assert!(matches!(create_org(&db, &zed.id, "  ", None).await, Err(AppError::BadRequest(_))));
        assert!(matches!(create_org(&db, &zed.id, &"x".repeat(61), Some("xx")).await, Err(AppError::BadRequest(_))));
        assert!(matches!(create_org(&db, &zed.id, "API", None).await, Err(AppError::BadRequest(_))), "derived slug reserved");
        assert!(matches!(create_org(&db, &zed.id, "Zed", Some("join")).await, Err(AppError::BadRequest(_))));
        assert!(matches!(create_org(&db, &zed.id, "Z", None).await, Err(AppError::BadRequest(_))), "derived slug too short");
        assert_eq!(create_org(&db, &zed.id, "Z", Some("zz")).await.unwrap().slug, "zz");

        // At most 5 owned orgs (paul has 1).
        for i in 2..=5 {
            create_org(&db, &paul.id, &format!("Org {i}"), None).await.unwrap();
        }
        match create_org(&db, &paul.id, "Org 6", None).await {
            Err(AppError::BadRequest(m)) => assert_eq!(m, "you can own at most 5 orgs"),
            other => panic!("{other:?}"),
        }
        // Joining other orgs doesn't count toward the cap.
        let zz = org_by_slug(&db, "zz", &zed.id).await.unwrap().unwrap();
        join_via_invite(&db, &current_invite(&db, &zz.id, &zed.id).await.unwrap(), &paul.id).await.unwrap();
        assert_eq!(list_user_teams(&db, &paul.id).await.unwrap().len(), 6);
    }

    #[tokio::test]
    async fn slug_availability_and_lookup() {
        let db = test_pool().await;
        let paul = signup(&db, "paul@example.com").await;
        let zed = signup(&db, "zed@example.com").await;
        let org = create_org(&db, &paul.id, "Acme", None).await.unwrap();

        let a = slug_availability(&db, " New Thing ").await.unwrap();
        assert_eq!(a, SlugAvailability { slug: "new-thing".into(), available: true, reason: None });
        assert_eq!(serde_json::to_value(&a).unwrap(), serde_json::json!({ "slug": "new-thing", "available": true }));
        let a = slug_availability(&db, "ACME").await.unwrap();
        assert_eq!((a.slug.as_str(), a.available, a.reason.as_deref()), ("acme", false, Some("swarmboard.app/acme is taken")));
        let a = slug_availability(&db, "Admin").await.unwrap();
        assert_eq!((a.available, a.reason.as_deref()), (false, Some("swarmboard.app/admin is reserved")));
        let a = slug_availability(&db, "x").await.unwrap();
        assert!(!a.available && a.reason.unwrap().contains("2-32"));

        // Members see it (any case); non-members and unknown slugs look the same.
        let got = org_by_slug(&db, "ACME", &paul.id).await.unwrap().unwrap();
        assert_eq!((got.id.as_str(), got.slug.as_str(), got.role.as_str()), (org.id.as_str(), "acme", "owner"));
        assert!(org_by_slug(&db, "acme", &zed.id).await.unwrap().is_none());
        assert!(org_by_slug(&db, "nope", &zed.id).await.unwrap().is_none());
        join_via_invite(&db, &current_invite(&db, &org.id, &paul.id).await.unwrap(), &zed.id).await.unwrap();
        assert_eq!(org_by_slug(&db, "acme", &zed.id).await.unwrap().unwrap().role, "member");
        assert_eq!(invite_info(&db, &current_invite(&db, &org.id, &paul.id).await.unwrap()).await.unwrap().unwrap().team_slug, "acme");
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
            s3: None,
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

    /// A team with owner, an admin and two plain members.
    async fn team_with_roles(db: &SqlitePool) -> (String, User, User, User, User) {
        let owner = signup(db, "owner@example.com").await;
        let team = personal_team(db, &owner).await;
        let code = current_invite(db, &team, &owner.id).await.unwrap();
        let mut users = vec![];
        for h in ["adam", "mia", "max"] {
            let u = signup(db, &format!("{h}@example.com")).await;
            join_via_invite(db, &code, &u.id).await.unwrap();
            users.push(u);
        }
        let max = users.pop().unwrap();
        let mia = users.pop().unwrap();
        let adam = users.pop().unwrap();
        set_role(db, &team, &owner.id, &adam.id, "admin").await.unwrap();
        (team, owner, adam, mia, max)
    }

    async fn event_data(db: &SqlitePool, team: &str, kind: &str) -> Vec<serde_json::Value> {
        let rows: Vec<String> = sqlx::query_scalar("SELECT data FROM events WHERE team_id = ? AND kind = ? ORDER BY id")
            .bind(team)
            .bind(kind)
            .fetch_all(db)
            .await
            .unwrap();
        rows.iter().map(|d| serde_json::from_str(d).unwrap()).collect()
    }

    #[test]
    fn removal_matrix() {
        use Role::*;
        // (actor, target, self?) -> allowed
        let ok = |a, t, me| check_removal(a, t, me).is_ok();
        assert!(ok(Owner, Admin, false) && ok(Owner, Member, false));
        assert!(ok(Admin, Member, false));
        assert!(!ok(Admin, Admin, false) && !ok(Admin, Owner, false));
        assert!(!ok(Member, Member, false) && !ok(Member, Admin, false) && !ok(Member, Owner, false));
        assert!(ok(Admin, Admin, true) && ok(Member, Member, true));
        match check_removal(Owner, Owner, true) {
            Err(AppError::BadRequest(m)) => assert_eq!(m, "transfer ownership first"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(check_removal(Admin, Owner, false), Err(AppError::Forbidden(_))));
    }

    #[tokio::test]
    async fn role_changes() {
        let db = test_pool().await;
        let (team, owner, adam, mia, max) = team_with_roles(&db).await;
        assert_eq!(member_role(&db, &team, &adam.id).await.unwrap(), Some(Role::Admin));
        let mine = list_user_teams(&db, &adam.id).await.unwrap();
        assert_eq!(mine.iter().find(|t| t.id == team).unwrap().role, "admin");
        let roles: Vec<(String, String)> = list_members(&db, &team).await.unwrap().into_iter().map(|m| (m.handle, m.role)).collect();
        assert_eq!(roles[0], ("owner".into(), "owner".into()));
        assert!(roles.contains(&("adam".into(), "admin".into())));

        // Only the owner changes roles; admins and members get 403.
        assert!(matches!(set_role(&db, &team, &adam.id, &mia.id, "admin").await, Err(AppError::Forbidden(_))));
        assert!(matches!(set_role(&db, &team, &mia.id, &max.id, "admin").await, Err(AppError::Forbidden(_))));
        let outsider = signup(&db, "out@example.com").await;
        assert!(matches!(set_role(&db, &team, &outsider.id, &mia.id, "admin").await, Err(AppError::Forbidden(_))));
        // The owner's role can't be changed here; bad roles and non-members are rejected.
        assert!(matches!(set_role(&db, &team, &owner.id, &owner.id, "member").await, Err(AppError::BadRequest(_))));
        assert!(matches!(set_role(&db, &team, &owner.id, &mia.id, "owner").await, Err(AppError::BadRequest(_))));
        assert!(matches!(set_role(&db, &team, &owner.id, &mia.id, "god").await, Err(AppError::BadRequest(_))));
        assert!(matches!(set_role(&db, &team, &owner.id, &outsider.id, "admin").await, Err(AppError::NotFound(_))));

        let m = set_role(&db, &team, &owner.id, &mia.id, "admin").await.unwrap();
        assert_eq!((m.user_id.as_str(), m.handle.as_str(), m.role.as_str()), (mia.id.as_str(), "mia", "admin"));
        assert_eq!(set_role(&db, &team, &owner.id, &mia.id, "member").await.unwrap().role, "member");
        let ev = event_data(&db, &team, "role_changed").await;
        assert_eq!(ev.len(), 3, "{ev:?}");
        assert_eq!((ev[2]["user"].as_str(), ev[2]["role"].as_str()), (Some("mia"), Some("member")));
        // Setting the same role again is a no-op without an event.
        set_role(&db, &team, &owner.id, &mia.id, "member").await.unwrap();
        assert_eq!(event_data(&db, &team, "role_changed").await.len(), 3);
    }

    #[tokio::test]
    async fn member_removal_rules() {
        let db = test_pool().await;
        let (team, owner, adam, mia, max) = team_with_roles(&db).await;
        let eve = signup(&db, "eve@example.com").await;
        join_via_invite(&db, &current_invite(&db, &team, &owner.id).await.unwrap(), &eve.id).await.unwrap();
        set_role(&db, &team, &owner.id, &eve.id, "admin").await.unwrap();

        let forbidden = |r: AppResult<()>| matches!(r, Err(AppError::Forbidden(_)));
        assert!(forbidden(remove_member(&db, &team, &mia.id, &max.id).await), "member can't remove others");
        assert!(forbidden(remove_member(&db, &team, &mia.id, &owner.id).await));
        assert!(forbidden(remove_member(&db, &team, &adam.id, &eve.id).await), "admin can't remove an admin");
        assert!(forbidden(remove_member(&db, &team, &adam.id, &owner.id).await), "admin can't remove the owner");
        match remove_member(&db, &team, &owner.id, &owner.id).await {
            Err(AppError::BadRequest(m)) => assert_eq!(m, "transfer ownership first"),
            other => panic!("{other:?}"),
        }
        let outsider = signup(&db, "out@example.com").await;
        assert!(forbidden(remove_member(&db, &team, &outsider.id, &max.id).await));
        assert!(matches!(remove_member(&db, &team, &owner.id, &outsider.id).await, Err(AppError::NotFound(_))));

        remove_member(&db, &team, &adam.id, &max.id).await.unwrap(); // admin removes a member
        remove_member(&db, &team, &owner.id, &eve.id).await.unwrap(); // owner removes an admin
        remove_member(&db, &team, &mia.id, &mia.id).await.unwrap(); // a member leaves
        remove_member(&db, &team, &adam.id, &adam.id).await.unwrap(); // an admin leaves
        assert_eq!(list_members(&db, &team).await.unwrap().len(), 1);
        let ev = event_data(&db, &team, "member_removed").await;
        assert_eq!(ev.len(), 4);
        assert_eq!((ev[0]["user"].as_str(), ev[0]["by"].as_str()), (Some("max"), Some("adam")));
        assert_eq!((ev[2]["user"].as_str(), ev[2]["by"].as_str()), (Some("mia"), Some("mia")));
    }

    #[tokio::test]
    async fn removal_revokes_tokens_and_frees_leases() {
        use crate::domain::collab::{self, leases, tests::named_agent};
        let db = test_pool().await;
        let (team, owner, adam, mia, _max) = team_with_roles(&db).await;
        let tok = create_api_token(&db, &mia.id, &team, Some("laptop")).await.unwrap();
        let tok2 = create_api_token(&db, &mia.id, &team, None).await.unwrap();
        assert!(authenticate_token(&db, &tok.token).await.unwrap().is_some());
        // Mia's agent holds ci and waits on deploy; the owner's agent waits on ci.
        let (_s1, mia_bot) = named_agent(&db, &team, &mia.id, "bot").await;
        let (_s2, own_bot) = named_agent(&db, &team, &owner.id, "api").await;
        leases::claim(&db, &mia_bot, None, "ci", None, None, false).await.unwrap();
        leases::claim(&db, &own_bot, None, "deploy", None, None, false).await.unwrap();
        leases::claim(&db, &mia_bot, None, "deploy", None, None, true).await.unwrap();
        leases::claim(&db, &own_bot, None, "ci", None, None, true).await.unwrap();

        remove_member(&db, &team, &adam.id, &mia.id).await.unwrap();
        assert!(authenticate_token(&db, &tok.token).await.unwrap().is_none(), "removed member's token stops working");
        let live: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_tokens WHERE user_id = ? AND team_id = ? AND revoked_at IS NULL")
            .bind(&mia.id)
            .bind(&team)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(live, 0, "both tokens revoked ({})", tok2.id);

        let v = leases::list(&db, &team).await.unwrap();
        let held: Vec<(String, String)> = v.leases.iter().map(|l| (l.scope.clone(), l.holder.display.clone())).collect();
        assert_eq!(held, vec![("ci".into(), "owner/api".into()), ("deploy".into(), "owner/api".into())]);
        assert!(v.queue.is_empty(), "removed member's queue entries are dropped");
        let pings = collab::take_agent_pings(&db, own_bot.agent_id.as_deref().unwrap()).await.unwrap();
        assert!(pings.iter().any(|p| p.body.contains("you now hold ci")), "{pings:?}");
        let ev = event_data(&db, &team, "lease_force_released").await;
        assert_eq!((ev[0]["scope"].as_str(), ev[0]["by"].as_str()), (Some("ci"), Some("adam")));
    }

    #[tokio::test]
    async fn transfer_ownership_rules_and_atomicity() {
        let db = test_pool().await;
        let (team, owner, adam, mia, _max) = team_with_roles(&db).await;
        let outsider = signup(&db, "out@example.com").await;
        async fn snapshot(db: &SqlitePool, team: &str) -> (Vec<(String, String)>, String) {
            let roles: Vec<(String, String)> =
                sqlx::query_as("SELECT user_id, role FROM memberships WHERE team_id = ? ORDER BY user_id").bind(team).fetch_all(db).await.unwrap();
            let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM teams WHERE id = ?").bind(team).fetch_one(db).await.unwrap();
            (roles, owner_id)
        }
        let before = snapshot(&db, &team).await;

        assert!(matches!(transfer_ownership(&db, &team, &adam.id, &mia.id).await, Err(AppError::Forbidden(_))), "admins can't");
        assert!(matches!(transfer_ownership(&db, &team, &mia.id, &mia.id).await, Err(AppError::Forbidden(_))));
        assert!(matches!(transfer_ownership(&db, &team, &owner.id, &outsider.id).await, Err(AppError::BadRequest(_))));
        assert!(matches!(transfer_ownership(&db, &team, &owner.id, &owner.id).await, Err(AppError::BadRequest(_))));
        assert_eq!(snapshot(&db, &team).await, before);

        // A failure after the role swap (here: updating teams.owner_id) rolls everything back.
        sqlx::query("CREATE TRIGGER fail_owner BEFORE UPDATE OF owner_id ON teams BEGIN SELECT RAISE(ABORT, 'boom'); END")
            .execute(&db)
            .await
            .unwrap();
        assert!(matches!(transfer_ownership(&db, &team, &owner.id, &mia.id).await, Err(AppError::Internal(_))));
        assert_eq!(snapshot(&db, &team).await, before, "no partial transfer");
        sqlx::query("DROP TRIGGER fail_owner").execute(&db).await.unwrap();

        transfer_ownership(&db, &team, &owner.id, &mia.id).await.unwrap();
        assert_eq!(member_role(&db, &team, &mia.id).await.unwrap(), Some(Role::Owner));
        assert_eq!(member_role(&db, &team, &owner.id).await.unwrap(), Some(Role::Admin));
        assert_eq!(get_team(&db, &team).await.unwrap().unwrap().owner_id, mia.id);
        let owners: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memberships WHERE team_id = ? AND role = 'owner'")
            .bind(&team)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(owners, 1);
        let ev = event_data(&db, &team, "ownership_transferred").await;
        assert_eq!((ev[0]["from"].as_str(), ev[0]["to"].as_str()), (Some("owner"), Some("mia")));

        // The old owner is now an admin: can't transfer back, but can leave.
        assert!(matches!(transfer_ownership(&db, &team, &owner.id, &owner.id).await, Err(AppError::Forbidden(_))));
        // A second owner row is impossible.
        let dup = sqlx::query("UPDATE memberships SET role = 'owner' WHERE team_id = ? AND user_id = ?")
            .bind(&team)
            .bind(&adam.id)
            .execute(&db)
            .await;
        assert!(dup.is_err(), "unique owner index");
        remove_member(&db, &team, &owner.id, &owner.id).await.unwrap();
    }

    /// Migration 0006 on a database built by 0001-0005 with data in it.
    #[tokio::test]
    async fn roles_migration_on_old_data() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(true);
        let db = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await.unwrap();
        let full = sqlx::migrate!("./migrations");
        let mut old = sqlx::migrate!("./migrations");
        old.migrations = old.migrations.iter().filter(|m| m.version < 6).cloned().collect::<Vec<_>>().into();
        old.run(&db).await.unwrap();
        for (id, h) in [("usr_o", "o"), ("usr_m", "m"), ("usr_x", "x")] {
            sqlx::query("INSERT INTO users (id, email, handle, created_at) VALUES (?, ?, ?, 1)").bind(id).bind(format!("{h}@x.com")).bind(h).execute(&db).await.unwrap();
        }
        sqlx::query("INSERT INTO teams (id, name, owner_id, created_at) VALUES ('tm_1', 't', 'usr_o', 1), ('tm_2', 'u', 'usr_x', 1)").execute(&db).await.unwrap();
        sqlx::query(
            "INSERT INTO memberships (team_id, user_id, role, joined_at) VALUES \
             ('tm_1', 'usr_o', 'owner', 10), ('tm_1', 'usr_m', 'member', 20), ('tm_2', 'usr_x', 'owner', 30), ('tm_2', 'usr_m', 'member', 40)",
        )
        .execute(&db)
        .await
        .unwrap();
        assert!(sqlx::query("UPDATE memberships SET role = 'admin' WHERE user_id = 'usr_m'").execute(&db).await.is_err(), "old CHECK");
        sqlx::query("INSERT INTO api_tokens (id, token_hash, prefix, user_id, team_id, label, created_at) VALUES ('tok_1', 'h', 'sb_', 'usr_m', 'tm_1', 'l', 1)")
            .execute(&db)
            .await
            .unwrap();

        full.run(&db).await.unwrap();
        let rows: Vec<(String, String, String, i64)> =
            sqlx::query_as("SELECT team_id, user_id, role, joined_at FROM memberships ORDER BY joined_at").fetch_all(&db).await.unwrap();
        assert_eq!(
            rows,
            vec![
                ("tm_1".into(), "usr_o".into(), "owner".into(), 10),
                ("tm_1".into(), "usr_m".into(), "member".into(), 20),
                ("tm_2".into(), "usr_x".into(), "owner".into(), 30),
                ("tm_2".into(), "usr_m".into(), "member".into(), 40),
            ]
        );
        let idx: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'memberships' AND sql IS NOT NULL ORDER BY name")
            .fetch_all(&db)
            .await
            .unwrap();
        assert_eq!(idx, vec!["memberships_one_owner", "memberships_user"]);
        let fk_problems: Vec<(String,)> = sqlx::query_as("SELECT \"table\" FROM pragma_foreign_key_check").fetch_all(&db).await.unwrap();
        assert!(fk_problems.is_empty(), "{fk_problems:?}");
        // New CHECK accepts admin, still rejects junk; the app reads the migrated rows.
        sqlx::query("UPDATE memberships SET role = 'admin' WHERE team_id = 'tm_1' AND user_id = 'usr_m'").execute(&db).await.unwrap();
        assert!(sqlx::query("UPDATE memberships SET role = 'boss' WHERE user_id = 'usr_m'").execute(&db).await.is_err());
        assert_eq!(member_role(&db, "tm_1", "usr_m").await.unwrap(), Some(Role::Admin));
        assert_eq!(list_user_teams(&db, "usr_m").await.unwrap().len(), 2);
        // FKs survived the rebuild: deleting a team cascades to its memberships.
        sqlx::query("DELETE FROM api_tokens").execute(&db).await.unwrap();
        sqlx::query("DELETE FROM teams WHERE id = 'tm_2'").execute(&db).await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memberships WHERE team_id = 'tm_2'").fetch_one(&db).await.unwrap();
        assert_eq!(n, 0);
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
    async fn update_team_name_and_slug_permissions() {
        let db = test_pool().await;
        let (team, owner, adam, mia, _max) = team_with_roles(&db).await;
        let other = signup(&db, "other@example.com").await;
        create_org(&db, &other.id, "Taken", None).await.unwrap();

        // Name: owner or admin; members get 403. Validation as before.
        assert_eq!(update_team(&db, &team, &adam.id, Some("  swarm  "), None).await.unwrap().name, "swarm");
        assert!(matches!(update_team(&db, &team, &mia.id, Some("x"), None).await, Err(AppError::Forbidden(_))));
        assert!(matches!(update_team(&db, &team, &owner.id, Some("   "), None).await, Err(AppError::BadRequest(_))));
        assert!(matches!(update_team(&db, &team, &owner.id, Some(&"x".repeat(61)), None).await, Err(AppError::BadRequest(_))));
        assert!(matches!(update_team(&db, &team, &other.id, Some("x"), None).await, Err(AppError::Forbidden(_))));

        // Slug: owner only; unchanged slugs pass for anyone (forms send every field).
        match update_team(&db, &team, &adam.id, None, Some("squad")).await {
            Err(AppError::Forbidden(m)) => assert_eq!(m, "only the org owner can change its URL"),
            other => panic!("{other:?}"),
        }
        assert_eq!(update_team(&db, &team, &adam.id, Some("swarm"), Some("Owner")).await.unwrap().slug, "owner");
        assert!(matches!(update_team(&db, &team, &owner.id, None, Some("TAKEN")).await, Err(AppError::Conflict(m)) if m == "swarmboard.app/taken is taken"));
        assert!(matches!(update_team(&db, &team, &owner.id, None, Some("static")).await, Err(AppError::BadRequest(_))));
        let t = update_team(&db, &team, &owner.id, Some("Crew HQ"), Some("Crew HQ")).await.unwrap();
        assert_eq!((t.name.as_str(), t.slug.as_str()), ("Crew HQ", "crew-hq"));
        assert_eq!(org_by_slug(&db, "crew-hq", &mia.id).await.unwrap().unwrap().id, team);
        assert!(org_by_slug(&db, "owner", &mia.id).await.unwrap().is_none(), "old slug is free");

        let ev = event_data(&db, &team, "org_renamed").await;
        assert_eq!(
            ev,
            vec![
                serde_json::json!({ "name": "swarm" }),
                serde_json::json!({ "name": "Crew HQ", "slug": "crew-hq" }),
            ]
        );
    }

    /// Migration 0007 on a database built by 0001-0006 with data in it.
    #[tokio::test]
    async fn orgs_migration_on_old_data() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(true);
        let db = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await.unwrap();
        let full = sqlx::migrate!("./migrations");
        let mut old = sqlx::migrate!("./migrations");
        old.migrations = old.migrations.iter().filter(|m| m.version < 7).cloned().collect::<Vec<_>>().into();
        old.run(&db).await.unwrap();
        sqlx::query("INSERT INTO users (id, email, handle, created_at) VALUES ('usr_o', 'o@x.com', 'o', 1)").execute(&db).await.unwrap();
        let teams = [
            ("tm_aaaaaaaa1", "paul's board", 1),
            ("tm_bbbbbbbb2", "Paul's Board!", 2),
            ("tm_cccccccc3", "paul-s-board", 3),
            ("tm_dddddddd4", "API", 4),
            ("tm_eeeeeeee5", "日本", 5),
            ("tm_ffffffff6", "x", 6),
            ("tm_gggggggg7", "A very long team name that goes past thirty-two chars", 7),
            ("tm_hhhhhhhh8", "ab-2", 8),
            ("tm_iiiiiiii9", "ab", 9),
            ("tm_jjjjjjjj0", "AB", 10),
        ];
        for (id, name, at) in teams {
            sqlx::query("INSERT INTO teams (id, name, owner_id, created_at) VALUES (?, ?, 'usr_o', ?)")
                .bind(id)
                .bind(name)
                .bind(at)
                .execute(&db)
                .await
                .unwrap();
            sqlx::query("INSERT INTO memberships (team_id, user_id, role, joined_at) VALUES (?, 'usr_o', 'owner', ?)")
                .bind(id)
                .bind(at)
                .execute(&db)
                .await
                .unwrap();
        }

        full.run(&db).await.unwrap();
        let slugs: Vec<(String, String)> = sqlx::query_as("SELECT id, slug FROM teams ORDER BY created_at").fetch_all(&db).await.unwrap();
        let got: Vec<&str> = slugs.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(
            got,
            vec![
                "paul-s-board",
                "paul-s-board-2",
                "paul-s-board-3",
                "api-org",
                "team",
                "team-2",
                "a-very-long-team-name-that-g",
                "ab-2",
                "ab",
                "ab-jjjjjjjj", // "ab-2" was taken by an older team
            ]
        );
        for s in &got {
            check_slug(s).unwrap_or_else(|e| panic!("{s}: {e}"));
        }
        // The app reads the backfilled rows; new rows need a slug, unique ignoring case.
        let mine = list_user_teams(&db, "usr_o").await.unwrap();
        assert_eq!(mine.len(), 10);
        assert!(mine.iter().any(|t| t.slug == "api-org"));
        assert!(sqlx::query("INSERT INTO teams (id, name, owner_id, created_at) VALUES ('tm_n', 'n', 'usr_o', 1)").execute(&db).await.is_err());
        assert!(sqlx::query("INSERT INTO teams (id, name, slug, owner_id, created_at) VALUES ('tm_n', 'n', 'team', 'usr_o', 1)").execute(&db).await.is_err());
        assert!(sqlx::query("UPDATE teams SET slug = NULL WHERE id = 'tm_iiiiiiii9'").execute(&db).await.is_err());
        let fk_problems: Vec<(String,)> = sqlx::query_as("SELECT \"table\" FROM pragma_foreign_key_check").fetch_all(&db).await.unwrap();
        assert!(fk_problems.is_empty(), "{fk_problems:?}");
    }
}
