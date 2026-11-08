//! Leases: exclusive, auto-renewing locks so agents on different machines don't step on each other.
//!
//! Scopes are file paths/globs ("server/src/auth/**"), named shared resources ("ci",
//! "deploy:staging", "port:3000") or tasks ("#12"). Path leases are per repo; resources and tasks
//! are team-wide. A lease lasts `minutes` and is pushed forward by every tool call (and Claude Code
//! hook) of its holder, so an agent that goes quiet loses it. Expiry is evaluated lazily (`tick`)
//! and a freed scope goes to the first compatible waiter in the FIFO queue, who gets a ping.
//! All writes run under `BEGIN IMMEDIATE`, so check-then-insert is atomic across connections.

use serde_json::json;
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

use super::{add_comment, author, bad, claim_post, deliver_ping, record_event, release_post, require_post, truncate_chars, Recipient};
use crate::{
    error::{AppError, AppResult},
    models::{Actor, ClaimsView, Lease, LeaseWait, Post},
    util::{new_id, now_ms},
};

pub const DEFAULT_MINUTES: i64 = 30;
pub const MAX_MINUTES: i64 = 480;
/// Queue entries of agents unseen for this long are dropped.
const STALE_WAIT_MS: i64 = 60 * 60 * 1000;
const MAX_NOTE: usize = 300;

// ---------------------------------------------------------------------------------------------
// Scopes
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    Path,
    Resource,
    Task,
}

impl ScopeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ScopeKind::Path => "path",
            ScopeKind::Resource => "resource",
            ScopeKind::Task => "task",
        }
    }
    fn parse(s: &str) -> ScopeKind {
        match s {
            "path" => ScopeKind::Path,
            "task" => ScopeKind::Task,
            _ => ScopeKind::Resource,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub kind: ScopeKind,
    pub text: String,
}

impl Scope {
    pub fn task_number(&self) -> Option<i64> {
        if self.kind == ScopeKind::Task { self.text.trim_start_matches('#').parse().ok() } else { None }
    }
}

/// "https://github.com/Acme/App.git", "git@github.com:acme/app.git" → "github.com/acme/app";
/// a bare project name is just lowercased.
pub fn normalize_repo(raw: &str) -> Option<String> {
    let mut s = raw.trim().to_lowercase();
    if let Some(i) = s.find("://") {
        s = s[i + 3..].to_string();
    }
    let first_slash = s.find('/').unwrap_or(s.len());
    if let Some(at) = s[..first_slash].find('@') {
        s = s[at + 1..].to_string();
    }
    let first_slash = s.find('/').unwrap_or(s.len());
    if let Some(c) = s[..first_slash].find(':') {
        let (host, rest) = (s[..c].to_string(), s[c + 1..].to_string());
        // host:22/path (port) vs host:org/repo (scp-like)
        let rest = match rest.split_once('/') {
            Some((port, path)) if port.chars().all(|ch| ch.is_ascii_digit()) => path.to_string(),
            _ => rest,
        };
        s = format!("{host}/{rest}");
    }
    let mut s = s.trim_matches('/').to_string();
    if let Some(stripped) = s.strip_suffix(".git") {
        s = stripped.to_string();
    }
    while s.contains("//") {
        s = s.replace("//", "/");
    }
    let s = truncate_chars(s.trim_matches('/'), 200);
    if s.is_empty() { None } else { Some(s) }
}

fn is_glob(p: &str) -> bool {
    p.contains(['*', '?', '['])
}

/// The literal prefix of a pattern, up to the first `*`, `?` or `[`.
fn literal(p: &str) -> &str {
    p.find(['*', '?', '[']).map(|i| &p[..i]).unwrap_or(p)
}

fn under(path: &str, dir: &str) -> bool {
    path.len() > dir.len() && path.starts_with(dir) && path.as_bytes()[dir.len()] == b'/'
}

/// Conservative overlap of two path patterns (false positives are fine, false negatives are not).
pub fn paths_overlap(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (is_glob(a), is_glob(b)) {
        // A literal is a file or a directory.
        (false, false) => under(a, b) || under(b, a),
        (true, false) => b.starts_with(literal(a)) || under(literal(a), b),
        (false, true) => a.starts_with(literal(b)) || under(literal(b), a),
        (true, true) => literal(a).starts_with(literal(b)) || literal(b).starts_with(literal(a)),
    }
}

/// Classify and normalize. "#12" is a task; anything with a ':' or without any of `/ . * ? [`
/// is a named resource ("ci", "deploy:staging", "release"); everything else is a repo path.
/// "path:README" forces a path.
pub fn parse_scope(raw: &str) -> AppResult<Scope> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(bad("scope is required: a path/glob (\"web/src/**\"), a resource (\"ci\", \"deploy:staging\") or a task (\"#12\")"));
    }
    if t.chars().count() > 300 {
        return Err(bad("scope is too long (max 300 chars)"));
    }
    if let Some(n) = t.strip_prefix('#')
        && let Ok(n) = n.trim().parse::<i64>()
        && n > 0
    {
        return Ok(Scope { kind: ScopeKind::Task, text: format!("#{n}") });
    }
    let (forced_path, t) = match t.strip_prefix("path:").or_else(|| t.strip_prefix("file:")) {
        Some(rest) => (true, rest.trim()),
        None => (false, t),
    };
    if !forced_path && (t.contains(':') || !t.contains(['/', '.', '*', '?', '['])) {
        let r = t.split_whitespace().collect::<Vec<_>>().join("-").to_lowercase();
        return Ok(Scope { kind: ScopeKind::Resource, text: r });
    }
    let mut p = t.replace('\\', "/");
    while p.contains("//") {
        p = p.replace("//", "/");
    }
    while let Some(rest) = p.strip_prefix("./") {
        p = rest.to_string();
    }
    let mut p = p.trim_start_matches('/').to_string();
    if p.is_empty() || p == "." {
        p = "**".into();
    } else if p.ends_with('/') {
        p.push_str("**");
    }
    Ok(Scope { kind: ScopeKind::Path, text: p })
}

fn repos_match(a: &str, b: &str) -> bool {
    a.is_empty() || b.is_empty() || a == b
}

pub fn conflicts(a_kind: ScopeKind, a_repo: &str, a: &str, b_kind: ScopeKind, b_repo: &str, b: &str) -> bool {
    if a_kind != b_kind {
        return false;
    }
    match a_kind {
        ScopeKind::Path => repos_match(a_repo, b_repo) && paths_overlap(a, b),
        _ => a == b,
    }
}

// ---------------------------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, FromRow)]
pub struct LeaseRow {
    pub id: String,
    pub team_id: String,
    pub repo: String,
    pub scope: String,
    pub kind: String,
    pub agent_id: String,
    pub user_id: String,
    pub note: String,
    pub minutes: i64,
    pub created_at: i64,
    pub expires_at: i64,
    pub handle: String,
    pub agent_name: String,
}

impl LeaseRow {
    fn kind(&self) -> ScopeKind {
        ScopeKind::parse(&self.kind)
    }
    pub fn display(&self) -> String {
        format!("{}/{}", self.handle, self.agent_name)
    }
    fn actor(&self) -> Actor {
        Actor { team_id: self.team_id.clone(), user_id: self.user_id.clone(), agent_id: Some(self.agent_id.clone()) }
    }
    pub fn to_lease(&self) -> Lease {
        Lease {
            scope: self.scope.clone(),
            kind: self.kind.clone(),
            repo: self.repo.clone(),
            holder: author(self.user_id.clone(), self.handle.clone(), Some(self.agent_id.clone()), Some(self.agent_name.clone())),
            note: self.note.clone(),
            expires_at: self.expires_at,
            created_at: self.created_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct WaitRow {
    id: i64,
    team_id: String,
    repo: String,
    scope: String,
    kind: String,
    agent_id: String,
    user_id: String,
    note: String,
    minutes: i64,
    created_at: i64,
    handle: String,
    agent_name: String,
}

impl WaitRow {
    fn kind(&self) -> ScopeKind {
        ScopeKind::parse(&self.kind)
    }
    fn display(&self) -> String {
        format!("{}/{}", self.handle, self.agent_name)
    }
    fn actor(&self) -> Actor {
        Actor { team_id: self.team_id.clone(), user_id: self.user_id.clone(), agent_id: Some(self.agent_id.clone()) }
    }
}

macro_rules! lease_select {
    () => {
        "SELECT l.id, l.team_id, l.repo, l.scope, l.kind, l.agent_id, l.user_id, l.note, l.minutes,
        l.created_at, l.expires_at, u.handle, a.name AS agent_name
   FROM leases l JOIN users u ON u.id = l.user_id JOIN agents a ON a.id = l.agent_id "
    };
}
macro_rules! wait_select {
    () => {
        "SELECT w.id, w.team_id, w.repo, w.scope, w.kind, w.agent_id, w.user_id, w.note, w.minutes,
        w.created_at, u.handle, a.name AS agent_name
   FROM lease_waits w JOIN users u ON u.id = w.user_id JOIN agents a ON a.id = w.agent_id "
    };
}

async fn active_leases(tx: &mut Transaction<'static, Sqlite>, team_id: &str, now: i64) -> AppResult<Vec<LeaseRow>> {
    Ok(sqlx::query_as(concat!(lease_select!(), "WHERE l.team_id = ?1 AND l.expires_at > ?2 ORDER BY l.created_at"))
        .bind(team_id)
        .bind(now)
        .fetch_all(&mut **tx)
        .await?)
}

async fn waits(tx: &mut Transaction<'static, Sqlite>, team_id: &str) -> AppResult<Vec<WaitRow>> {
    Ok(sqlx::query_as(concat!(wait_select!(), "WHERE w.team_id = ?1 ORDER BY w.id")).bind(team_id).fetch_all(&mut **tx).await?)
}

async fn begin(db: &SqlitePool) -> AppResult<Transaction<'static, Sqlite>> {
    Ok(db.begin_with("BEGIN IMMEDIATE").await?)
}

// ---------------------------------------------------------------------------------------------
// Notifications (sent after commit: deliver_ping uses its own transaction)
// ---------------------------------------------------------------------------------------------

enum Notice {
    /// `released`: who freed it and their note, when the grant follows an explicit release.
    Granted { from: Actor, to: Recipient, waiter: Actor, scope: String, repo: String, minutes: i64, released: Option<(String, String)> },
    Expired { holder: Actor, to: Recipient, scope: String, minutes: i64 },
    /// A human took the lease away (admin release); `body` is the whole message.
    ForceReleased { by: Actor, to: Recipient, body: String },
}

async fn deliver(db: &SqlitePool, notices: Vec<Notice>) {
    for n in notices {
        let r = match n {
            Notice::Granted { from, to, waiter, scope, repo, minutes, released } => {
                let where_ = if repo.is_empty() { String::new() } else { format!(" in {repo}") };
                let by = match released {
                    Some((who, note)) if note.is_empty() => format!(" {who} released it."),
                    Some((who, note)) => format!(" {who} released it: \"{note}\"."),
                    None => String::new(),
                };
                let body = format!(
                    "[swarmboard] you now hold {scope}{where_} (you were queued).{by} The lease lasts {minutes}m and renews \
                     while you make tool calls; release(scope:\"{scope}\") when done."
                );
                let target = to.display().to_string();
                let r = deliver_ping(db, &from, &target, &body, None, &[to], false).await.map(|_| ());
                if let Some(n) = scope.strip_prefix('#').and_then(|n| n.parse::<i64>().ok()) {
                    let _ = claim_post(db, &waiter, n, false).await;
                }
                r
            }
            Notice::Expired { holder, to, scope, minutes } => {
                let body = format!(
                    "[swarmboard] your lease on {scope} expired: no tool calls from you for {minutes}m. Others may be \
                     editing it now; check/claim it again before you continue."
                );
                let target = to.display().to_string();
                deliver_ping(db, &holder, &target, &body, None, &[to], false).await.map(|_| ())
            }
            Notice::ForceReleased { by, to, body } => {
                let target = to.display().to_string();
                deliver_ping(db, &by, &target, &body, None, &[to], false).await.map(|_| ())
            }
        };
        if let Err(e) = r {
            tracing::warn!(error = %e, "lease notice failed");
        }
    }
}

fn lease_event_data(l: &LeaseRow) -> serde_json::Value {
    json!({ "scope": l.scope, "repo": l.repo, "note": l.note, "lease": true })
}

/// Leases freed on purpose (agent release or admin force-release) right before granting: lets
/// the waiters' grant pings say who released and why.
struct Freed<'a> {
    by: &'a Actor,
    /// "paul/api" or "paul (admin)".
    by_display: String,
    note: String,
    rows: &'a [LeaseRow],
}

impl Freed<'_> {
    /// Who released `w`'s scope and their note, if it was one of these leases.
    fn credit(&self, w: &WaitRow) -> Option<(String, String)> {
        self.rows
            .iter()
            .any(|l| conflicts(l.kind(), &l.repo, &l.scope, w.kind(), &w.repo, &w.scope))
            .then(|| (self.by_display.clone(), self.note.clone()))
    }
}

/// Expire lapsed leases (task leases also hand their post back), drop stale queue entries.
async fn expire_in_tx(tx: &mut Transaction<'static, Sqlite>, team_id: &str, now: i64, notices: &mut Vec<Notice>) -> AppResult<usize> {
    let expired: Vec<LeaseRow> =
        sqlx::query_as(concat!(lease_select!(), "WHERE l.team_id = ?1 AND l.expires_at <= ?2 ORDER BY l.expires_at"))
            .bind(team_id)
            .bind(now)
            .fetch_all(&mut **tx)
            .await?;
    for l in &expired {
        sqlx::query("DELETE FROM leases WHERE id = ?1").bind(l.id.as_str()).execute(&mut **tx).await?;
        let mut post_id: Option<String> = None;
        if let Some(n) = l.scope.strip_prefix('#').and_then(|n| n.parse::<i64>().ok()) {
            post_id = sqlx::query_scalar(
                "UPDATE posts SET claimed_by_agent_id = NULL, claimed_by_user_id = NULL, claimed_at = NULL,
                        status = CASE WHEN status = 'in_progress' THEN 'open' ELSE status END, updated_at = ?4
                 WHERE team_id = ?1 AND number = ?2 AND claimed_by_agent_id = ?3 RETURNING id",
            )
            .bind(team_id)
            .bind(n)
            .bind(l.agent_id.as_str())
            .bind(now)
            .fetch_optional(&mut **tx)
            .await?;
        }
        record_event(&mut **tx, &l.actor(), "expired", post_id.as_deref(), lease_event_data(l)).await?;
        notices.push(Notice::Expired {
            holder: l.actor(),
            to: Recipient::Agent { id: l.agent_id.clone(), display: l.display() },
            scope: l.scope.clone(),
            minutes: l.minutes,
        });
    }
    sqlx::query(
        "DELETE FROM lease_waits WHERE team_id = ?1
           AND agent_id IN (SELECT id FROM agents WHERE team_id = ?1 AND last_seen_at < ?2)",
    )
    .bind(team_id)
    .bind(now - STALE_WAIT_MS)
    .execute(&mut **tx)
    .await?;
    Ok(expired.len())
}

/// Hand freed scopes to waiters, FIFO. A waiter is skipped while its scope conflicts with a live
/// lease or with an earlier waiter that is still blocked (no queue jumping). Returns the
/// (scope, new holder) grants.
async fn grant_in_tx(
    tx: &mut Transaction<'static, Sqlite>,
    team_id: &str,
    now: i64,
    freed: Option<&Freed<'_>>,
    notices: &mut Vec<Notice>,
) -> AppResult<Vec<(String, String)>> {
    let mut granted = Vec::new();
    let ws = waits(tx, team_id).await?;
    if ws.is_empty() {
        return Ok(granted);
    }
    let mut active = active_leases(tx, team_id, now).await?;
    let mut blocked: Vec<WaitRow> = Vec::new();
    for w in ws {
        let hit = |kind: ScopeKind, repo: &str, scope: &str, agent: &str| {
            agent != w.agent_id && conflicts(kind, repo, scope, w.kind(), &w.repo, &w.scope)
        };
        if active.iter().any(|l| hit(l.kind(), &l.repo, &l.scope, &l.agent_id))
            || blocked.iter().any(|b| hit(b.kind(), &b.repo, &b.scope, &b.agent_id))
        {
            blocked.push(w);
            continue;
        }
        let id = new_id("lse");
        let expires = now + w.minutes * 60_000;
        sqlx::query(
            "INSERT INTO leases (id, team_id, repo, scope, kind, agent_id, user_id, note, minutes, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .bind(id.as_str())
        .bind(team_id)
        .bind(w.repo.as_str())
        .bind(w.scope.as_str())
        .bind(w.kind.as_str())
        .bind(w.agent_id.as_str())
        .bind(w.user_id.as_str())
        .bind(w.note.as_str())
        .bind(w.minutes)
        .bind(now)
        .bind(expires)
        .execute(&mut **tx)
        .await?;
        sqlx::query("DELETE FROM lease_waits WHERE id = ?1").bind(w.id).execute(&mut **tx).await?;
        let row = LeaseRow {
            id,
            team_id: w.team_id.clone(),
            repo: w.repo.clone(),
            scope: w.scope.clone(),
            kind: w.kind.clone(),
            agent_id: w.agent_id.clone(),
            user_id: w.user_id.clone(),
            note: w.note.clone(),
            minutes: w.minutes,
            created_at: now,
            expires_at: expires,
            handle: w.handle.clone(),
            agent_name: w.agent_name.clone(),
        };
        record_event(&mut **tx, &w.actor(), "granted", None, lease_event_data(&row)).await?;
        notices.push(Notice::Granted {
            from: freed.map(|f| f.by.clone()).unwrap_or_else(|| w.actor()),
            to: Recipient::Agent { id: w.agent_id.clone(), display: w.display() },
            waiter: w.actor(),
            scope: w.scope.clone(),
            repo: w.repo.clone(),
            minutes: w.minutes,
            released: freed.and_then(|f| f.credit(&w)),
        });
        granted.push((w.scope.clone(), w.display()));
        active.push(row);
    }
    Ok(granted)
}

// ---------------------------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------------------------

/// Extend all of an agent's live leases to now + their window (called on every tool call/hook).
pub async fn renew(db: &SqlitePool, agent_id: &str) -> AppResult<u64> {
    let now = now_ms();
    let r = sqlx::query("UPDATE leases SET expires_at = ?2 + minutes * 60000 WHERE agent_id = ?1 AND expires_at > ?2")
        .bind(agent_id)
        .bind(now)
        .execute(db)
        .await?;
    Ok(r.rows_affected())
}

/// Lazily expire lapsed leases in a team and grant freed scopes to waiters. Cheap when nothing is due.
pub async fn tick(db: &SqlitePool, team_id: &str) -> AppResult<()> {
    let now = now_ms();
    let due: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM leases WHERE team_id = ?1 AND expires_at <= ?2)")
        .bind(team_id)
        .bind(now)
        .fetch_one(db)
        .await?;
    if due == 0 {
        return Ok(());
    }
    let mut notices = Vec::new();
    let mut tx = begin(db).await?;
    expire_in_tx(&mut tx, team_id, now, &mut notices).await?;
    grant_in_tx(&mut tx, team_id, now, None, &mut notices).await?;
    tx.commit().await?;
    deliver(db, notices).await;
    Ok(())
}

#[derive(Debug)]
#[allow(dead_code)] // fields kept for callers/tests
pub enum ClaimOutcome {
    /// `renewed`: you already held exactly this scope.
    Granted { scope: Scope, repo: String, minutes: i64, expires_at: i64, renewed: bool, post: Option<Post> },
    /// Held by others (overlapping leases, oldest first).
    Taken { scope: Scope, repo: String, holders: Vec<LeaseRow> },
    Queued { scope: Scope, repo: String, position: i64, holders: Vec<LeaseRow> },
}

/// Take an exclusive lease. `agent_repo` scopes path leases (None = conflicts with paths in any repo).
pub async fn claim(
    db: &SqlitePool,
    actor: &Actor,
    agent_repo: Option<&str>,
    raw_scope: &str,
    note: Option<&str>,
    minutes: Option<i64>,
    wait: bool,
) -> AppResult<ClaimOutcome> {
    let agent_id = actor.agent_id.clone().ok_or_else(|| bad("only agents take leases"))?;
    let scope = parse_scope(raw_scope)?;
    let repo = if scope.kind == ScopeKind::Path { agent_repo.unwrap_or("").to_string() } else { String::new() };
    let minutes = minutes.unwrap_or(DEFAULT_MINUTES).clamp(1, MAX_MINUTES);
    let note = truncate_chars(note.unwrap_or("").trim(), MAX_NOTE);
    if let Some(n) = scope.task_number() {
        require_post(db, &actor.team_id, n).await?;
    }
    tick(db, &actor.team_id).await?;

    let now = now_ms();
    let mut tx = begin(db).await?;
    let active = active_leases(&mut tx, &actor.team_id, now).await?;
    let holders: Vec<LeaseRow> = active
        .iter()
        .filter(|l| l.agent_id != agent_id && conflicts(l.kind(), &l.repo, &l.scope, scope.kind, &repo, &scope.text))
        .cloned()
        .collect();
    if !holders.is_empty() {
        if !wait {
            return Ok(ClaimOutcome::Taken { scope, repo, holders });
        }
        let already: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM lease_waits WHERE team_id = ?1 AND agent_id = ?2 AND scope = ?3 AND repo = ?4",
        )
        .bind(actor.team_id.as_str())
        .bind(agent_id.as_str())
        .bind(scope.text.as_str())
        .bind(repo.as_str())
        .fetch_optional(&mut *tx)
        .await?;
        let my_id = match already {
            Some(id) => id,
            None => sqlx::query_scalar(
                "INSERT INTO lease_waits (team_id, repo, scope, kind, agent_id, user_id, note, minutes, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) RETURNING id",
            )
            .bind(actor.team_id.as_str())
            .bind(repo.as_str())
            .bind(scope.text.as_str())
            .bind(scope.kind.as_str())
            .bind(agent_id.as_str())
            .bind(actor.user_id.as_str())
            .bind(note.as_str())
            .bind(minutes)
            .bind(now)
            .fetch_one(&mut *tx)
            .await?,
        };
        let ws = waits(&mut tx, &actor.team_id).await?;
        let position = ws
            .iter()
            .filter(|w| w.id <= my_id && (w.id == my_id || conflicts(w.kind(), &w.repo, &w.scope, scope.kind, &repo, &scope.text)))
            .count() as i64;
        tx.commit().await?;
        return Ok(ClaimOutcome::Queued { scope, repo, position, holders });
    }

    let expires = now + minutes * 60_000;
    let mine: Option<String> = active
        .iter()
        .find(|l| l.agent_id == agent_id && l.scope == scope.text && l.repo == repo)
        .map(|l| l.id.clone());
    let renewed = mine.is_some();
    match mine {
        Some(id) => {
            sqlx::query("UPDATE leases SET minutes = ?2, expires_at = ?3, note = CASE WHEN ?4 = '' THEN note ELSE ?4 END WHERE id = ?1")
                .bind(id.as_str())
                .bind(minutes)
                .bind(expires)
                .bind(note.as_str())
                .execute(&mut *tx)
                .await?;
        }
        None => {
            sqlx::query(
                "INSERT INTO leases (id, team_id, repo, scope, kind, agent_id, user_id, note, minutes, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )
            .bind(new_id("lse"))
            .bind(actor.team_id.as_str())
            .bind(repo.as_str())
            .bind(scope.text.as_str())
            .bind(scope.kind.as_str())
            .bind(agent_id.as_str())
            .bind(actor.user_id.as_str())
            .bind(note.as_str())
            .bind(minutes)
            .bind(now)
            .bind(expires)
            .execute(&mut *tx)
            .await?;
            if scope.kind != ScopeKind::Task {
                record_event(
                    &mut *tx,
                    actor,
                    "claimed",
                    None,
                    json!({ "scope": scope.text, "repo": repo, "note": note, "lease": true }),
                )
                .await?;
            }
        }
    }
    sqlx::query("DELETE FROM lease_waits WHERE team_id = ?1 AND agent_id = ?2 AND scope = ?3 AND repo = ?4")
        .bind(actor.team_id.as_str())
        .bind(agent_id.as_str())
        .bind(scope.text.as_str())
        .bind(repo.as_str())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    let mut post = None;
    if let Some(n) = scope.task_number() {
        match claim_post(db, actor, n, false).await {
            Ok(p) => post = Some(p),
            Err(e) => {
                // The post is claimed outside the lease system (e.g. a human in the dashboard).
                if !renewed {
                    sqlx::query("DELETE FROM leases WHERE agent_id = ?1 AND scope = ?2 AND team_id = ?3")
                        .bind(agent_id.as_str())
                        .bind(scope.text.as_str())
                        .bind(actor.team_id.as_str())
                        .execute(db)
                        .await?;
                }
                return Err(e);
            }
        }
    }
    Ok(ClaimOutcome::Granted { scope, repo, minutes, expires_at: expires, renewed, post })
}

#[derive(Debug, Default)]
pub struct ReleaseOutcome {
    pub released: Vec<String>,
    /// The release note (trimmed; empty if none).
    pub note: String,
    /// (scope, new holder) for waiters that got a freed scope.
    pub granted: Vec<(String, String)>,
    pub left_queue: Vec<String>,
    /// Posts whose status/notes were updated ("#12" scopes).
    pub posts: Vec<Post>,
    /// Problems updating "#n" posts (lease is released regardless).
    pub warnings: Vec<String>,
}

/// Release one scope (exact text as claimed) or everything. `note` goes in the "released" event
/// and the next holder's grant ping; for "#n" scopes it is also posted as a reply and `status`
/// sets the post status (default "open"; e.g. "done").
pub async fn release(
    db: &SqlitePool,
    actor: &Actor,
    raw_scope: Option<&str>,
    all: bool,
    status: Option<&str>,
    note: Option<&str>,
) -> AppResult<ReleaseOutcome> {
    let agent_id = actor.agent_id.clone().ok_or_else(|| bad("only agents hold leases"))?;
    let scope = match raw_scope.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Some(parse_scope(s)?),
        None if all => None,
        None => return Err(bad("pass scope (as you claimed it) or all:true")),
    };
    let now = now_ms();
    let mut notices = Vec::new();
    let mut out = ReleaseOutcome { note: note.unwrap_or("").trim().to_string(), ..Default::default() };
    let short_note = truncate_chars(&out.note, MAX_NOTE);
    let mut tx = begin(db).await?;
    expire_in_tx(&mut tx, &actor.team_id, now, &mut notices).await?;
    let mine: Vec<LeaseRow> = sqlx::query_as(concat!(lease_select!(), "WHERE l.agent_id = ?1 ORDER BY l.created_at"))
        .bind(agent_id.as_str())
        .fetch_all(&mut *tx)
        .await?;
    let matches = |s: &str| scope.as_ref().is_none_or(|sc| sc.text == s);
    let freed_rows: Vec<LeaseRow> = mine.iter().filter(|l| matches(&l.scope)).cloned().collect();
    for l in &freed_rows {
        sqlx::query("DELETE FROM leases WHERE id = ?1").bind(l.id.as_str()).execute(&mut *tx).await?;
        if l.kind() != ScopeKind::Task {
            let data = json!({ "scope": l.scope, "repo": l.repo, "note": short_note, "lease": true });
            record_event(&mut *tx, actor, "released", None, data).await?;
        }
        out.released.push(l.scope.clone());
    }
    let my_waits: Vec<(i64, String)> = sqlx::query_as("SELECT id, scope FROM lease_waits WHERE agent_id = ?1 ORDER BY id")
        .bind(agent_id.as_str())
        .fetch_all(&mut *tx)
        .await?;
    for (id, s) in my_waits.into_iter().filter(|(_, s)| matches(s)) {
        sqlx::query("DELETE FROM lease_waits WHERE id = ?1").bind(id).execute(&mut *tx).await?;
        out.left_queue.push(s);
    }
    if out.released.is_empty() && out.left_queue.is_empty() && scope.as_ref().is_none_or(|s| s.kind != ScopeKind::Task) {
        let held: Vec<String> = mine.iter().map(|l| l.scope.clone()).collect();
        let what = scope.as_ref().map(|s| s.text.clone()).unwrap_or_else(|| "anything".into());
        return Err(AppError::NotFound(if held.is_empty() {
            format!("you hold no lease on {what} (you hold none)")
        } else {
            format!("you hold no lease on {what}; yours: {}", held.join(", "))
        }));
    }
    let by_display = freed_rows.first().map(LeaseRow::display).unwrap_or_default();
    let freed = Freed { by: actor, by_display, note: short_note, rows: &freed_rows };
    out.granted = grant_in_tx(&mut tx, &actor.team_id, now, Some(&freed), &mut notices).await?;
    tx.commit().await?;

    // Task scopes: hand the post back (or finish it) and leave the note.
    let mut tasks: Vec<i64> = out.released.iter().filter_map(|s| s.strip_prefix('#')?.parse().ok()).collect();
    if let Some(n) = scope.as_ref().and_then(Scope::task_number)
        && !tasks.contains(&n)
    {
        tasks.push(n); // claimed through the dashboard/older flow without a lease
    }
    for n in tasks {
        if !out.note.is_empty()
            && let Err(e) = add_comment(db, actor, n, &out.note).await
        {
            out.warnings.push(format!("#{n}: note not posted ({e})"));
        }
        let p = require_post(db, &actor.team_id, n).await;
        let mine = p.as_ref().is_ok_and(|p| p.claimed_by.as_ref().is_some_and(|c| c.agent_id.as_deref() == Some(agent_id.as_str())));
        if mine || status.is_some() {
            match release_post(db, actor, n, status).await {
                Ok(p) => out.posts.push(p),
                Err(e) => out.warnings.push(format!("#{n}: {e}")),
            }
        }
    }
    deliver(db, notices).await;
    Ok(out)
}

/// The agent's client exited: free everything it held and queued for.
pub async fn release_agent(db: &SqlitePool, actor: &Actor) -> AppResult<usize> {
    match release(db, actor, None, true, None, None).await {
        Ok(o) => Ok(o.released.len()),
        Err(AppError::NotFound(_)) => Ok(0),
        Err(e) => Err(e),
    }
}

/// Take leases away from their holders on a human's behalf (`by`, no agent): task leases hand
/// their post back, each release is logged as `lease_force_released`.
async fn force_release_in_tx(
    tx: &mut Transaction<'static, Sqlite>,
    rows: &[LeaseRow],
    by: &Actor,
    by_handle: &str,
    note: &str,
    now: i64,
) -> AppResult<()> {
    for l in rows {
        sqlx::query("DELETE FROM leases WHERE id = ?1").bind(l.id.as_str()).execute(&mut **tx).await?;
        let mut post_id: Option<String> = None;
        if l.kind() == ScopeKind::Task
            && let Some(n) = l.scope.strip_prefix('#').and_then(|n| n.parse::<i64>().ok())
        {
            post_id = sqlx::query_scalar(
                "UPDATE posts SET claimed_by_agent_id = NULL, claimed_by_user_id = NULL, claimed_at = NULL,
                        status = CASE WHEN status = 'in_progress' THEN 'open' ELSE status END, updated_at = ?4
                 WHERE team_id = ?1 AND number = ?2 AND claimed_by_agent_id = ?3 RETURNING id",
            )
            .bind(l.team_id.as_str())
            .bind(n)
            .bind(l.agent_id.as_str())
            .bind(now)
            .fetch_optional(&mut **tx)
            .await?;
        }
        let data = json!({ "scope": l.scope, "repo": l.repo, "by": by_handle, "holder": l.display(), "note": note });
        record_event(&mut **tx, by, "lease_force_released", post_id.as_deref(), data).await?;
    }
    Ok(())
}

/// Owner/admin action: free whatever agent holds `scope` (exact text as claimed) in `repo`
/// (None = any repo), grant it to the next waiter and tell the old holder (both see the optional
/// `note`). Returns how many leases were freed; NotFound if none.
pub async fn force_release(
    db: &SqlitePool,
    by: &Actor,
    by_handle: &str,
    raw_scope: &str,
    repo: Option<&str>,
    note: Option<&str>,
) -> AppResult<usize> {
    let scope = parse_scope(raw_scope)?;
    let note = truncate_chars(note.unwrap_or("").trim(), MAX_NOTE);
    let repo = repo.map(str::trim);
    let now = now_ms();
    let mut notices = Vec::new();
    let mut tx = begin(db).await?;
    expire_in_tx(&mut tx, &by.team_id, now, &mut notices).await?;
    let rows: Vec<LeaseRow> = active_leases(&mut tx, &by.team_id, now)
        .await?
        .into_iter()
        .filter(|l| l.scope == scope.text && repo.is_none_or(|r| l.repo == r))
        .collect();
    if rows.is_empty() {
        let where_ = repo.filter(|r| !r.is_empty()).map(|r| format!(" in {r}")).unwrap_or_default();
        return Err(AppError::NotFound(format!("no lease on {}{where_}", scope.text)));
    }
    force_release_in_tx(&mut tx, &rows, by, by_handle, &note, now).await?;
    let why = if note.is_empty() { String::new() } else { format!(": \"{note}\"") };
    for l in &rows {
        notices.push(Notice::ForceReleased {
            by: by.clone(),
            to: Recipient::Agent { id: l.agent_id.clone(), display: l.display() },
            body: format!("{by_handle} (admin) released {}{why}", l.scope),
        });
    }
    let freed = Freed { by, by_display: format!("{by_handle} (admin)"), note, rows: &rows };
    grant_in_tx(&mut tx, &by.team_id, now, Some(&freed), &mut notices).await?;
    tx.commit().await?;
    deliver(db, notices).await;
    Ok(rows.len())
}

/// A human left or was removed from the team: free all their agents' leases there (granting to
/// waiters) and drop their queue entries. Returns how many leases were freed.
pub async fn release_user(db: &SqlitePool, team_id: &str, user_id: &str, by: &Actor, by_handle: &str) -> AppResult<usize> {
    let now = now_ms();
    let mut notices = Vec::new();
    let mut tx = begin(db).await?;
    expire_in_tx(&mut tx, team_id, now, &mut notices).await?;
    sqlx::query("DELETE FROM lease_waits WHERE team_id = ?1 AND user_id = ?2").bind(team_id).bind(user_id).execute(&mut *tx).await?;
    let rows: Vec<LeaseRow> = active_leases(&mut tx, team_id, now).await?.into_iter().filter(|l| l.user_id == user_id).collect();
    force_release_in_tx(&mut tx, &rows, by, by_handle, "", now).await?;
    let freed = Freed { by, by_display: format!("{by_handle} (admin)"), note: String::new(), rows: &rows };
    grant_in_tx(&mut tx, team_id, now, Some(&freed), &mut notices).await?;
    tx.commit().await?;
    deliver(db, notices).await;
    Ok(rows.len())
}

/// Live leases + queue for a team (after expiring lapsed ones).
pub async fn list(db: &SqlitePool, team_id: &str) -> AppResult<ClaimsView> {
    tick(db, team_id).await?;
    let now = now_ms();
    let leases: Vec<LeaseRow> =
        sqlx::query_as(concat!(lease_select!(), "WHERE l.team_id = ?1 AND l.expires_at > ?2 ORDER BY l.repo, l.scope"))
            .bind(team_id)
            .bind(now)
            .fetch_all(db)
            .await?;
    let ws: Vec<WaitRow> =
        sqlx::query_as(concat!(wait_select!(), "WHERE w.team_id = ?1 ORDER BY w.id")).bind(team_id).fetch_all(db).await?;
    let queue = ws
        .iter()
        .map(|w| LeaseWait {
            scope: w.scope.clone(),
            kind: w.kind.clone(),
            repo: w.repo.clone(),
            waiter: author(w.user_id.clone(), w.handle.clone(), Some(w.agent_id.clone()), Some(w.agent_name.clone())),
            position: ws
                .iter()
                .filter(|o| o.id < w.id && conflicts(o.kind(), &o.repo, &o.scope, w.kind(), &w.repo, &w.scope))
                .count() as i64
                + 1,
            created_at: w.created_at,
        })
        .collect();
    Ok(ClaimsView { leases: leases.iter().map(LeaseRow::to_lease).collect(), queue })
}

/// What overlaps `scope` for this agent: others' leases, my own, and how many wait on them.
#[derive(Debug)]
#[allow(dead_code)]
pub struct CheckItem {
    pub scope: Scope,
    pub repo: String,
    pub others: Vec<LeaseRow>,
    pub mine: Vec<LeaseRow>,
    pub waiting: Vec<String>,
}

pub async fn check(db: &SqlitePool, actor: &Actor, agent_repo: Option<&str>, scopes: &[String]) -> AppResult<Vec<CheckItem>> {
    tick(db, &actor.team_id).await?;
    let now = now_ms();
    let leases: Vec<LeaseRow> = sqlx::query_as(concat!(lease_select!(), "WHERE l.team_id = ?1 AND l.expires_at > ?2 ORDER BY l.created_at"))
        .bind(actor.team_id.as_str())
        .bind(now)
        .fetch_all(db)
        .await?;
    let ws: Vec<WaitRow> = sqlx::query_as(concat!(wait_select!(), "WHERE w.team_id = ?1 ORDER BY w.id"))
        .bind(actor.team_id.as_str())
        .fetch_all(db)
        .await?;
    let me = actor.agent_id.as_deref().unwrap_or("");
    let mut out = Vec::new();
    for raw in scopes {
        let scope = parse_scope(raw)?;
        let repo = if scope.kind == ScopeKind::Path { agent_repo.unwrap_or("").to_string() } else { String::new() };
        let hits: Vec<LeaseRow> = leases
            .iter()
            .filter(|l| conflicts(l.kind(), &l.repo, &l.scope, scope.kind, &repo, &scope.text))
            .cloned()
            .collect();
        let (mine, others): (Vec<LeaseRow>, Vec<LeaseRow>) = hits.into_iter().partition(|l| l.agent_id == me);
        let waiting = ws
            .iter()
            .filter(|w| conflicts(w.kind(), &w.repo, &w.scope, scope.kind, &repo, &scope.text))
            .map(|w| w.display())
            .collect();
        out.push(CheckItem { scope, repo, others, mine, waiting });
    }
    Ok(out)
}

/// Live leases in a team (no expiry pass; call `tick` or `list` first).
pub async fn active(db: &SqlitePool, team_id: &str) -> AppResult<Vec<LeaseRow>> {
    Ok(sqlx::query_as(concat!(lease_select!(), "WHERE l.team_id = ?1 AND l.expires_at > ?2 ORDER BY l.repo, l.scope"))
        .bind(team_id)
        .bind(now_ms())
        .fetch_all(db)
        .await?)
}

/// Live leases of one agent (for briefings).
pub async fn agent_leases(db: &SqlitePool, agent_id: &str) -> AppResult<Vec<LeaseRow>> {
    Ok(sqlx::query_as(concat!(lease_select!(), "WHERE l.agent_id = ?1 AND l.expires_at > ?2 ORDER BY l.created_at"))
        .bind(agent_id)
        .bind(now_ms())
        .fetch_all(db)
        .await?)
}

/// Scopes an agent is queued for, with position.
pub async fn agent_waits(db: &SqlitePool, team_id: &str, agent_id: &str) -> AppResult<Vec<(String, i64)>> {
    let v = list(db, team_id).await?;
    Ok(v.queue.into_iter().filter(|w| w.waiter.agent_id.as_deref() == Some(agent_id)).map(|w| (w.scope, w.position)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::collab::{
        self,
        tests::{fixture, named_agent},
    };

    #[test]
    fn scope_parsing() {
        let p = |s: &str| parse_scope(s).unwrap();
        assert_eq!(p("#12"), Scope { kind: ScopeKind::Task, text: "#12".into() });
        assert_eq!(p("ci").kind, ScopeKind::Resource);
        assert_eq!(p("Deploy:Staging"), Scope { kind: ScopeKind::Resource, text: "deploy:staging".into() });
        assert_eq!(p("port:3000").kind, ScopeKind::Resource);
        assert_eq!(p("./server/src/auth/"), Scope { kind: ScopeKind::Path, text: "server/src/auth/**".into() });
        assert_eq!(p("/web//src/Landing.tsx").text, "web/src/Landing.tsx");
        assert_eq!(p("Cargo.toml").kind, ScopeKind::Path);
        assert_eq!(p("path:README").text, "README");
        assert_eq!(p(".").text, "**");
        assert!(parse_scope("  ").is_err());
    }

    #[test]
    fn repo_normalization() {
        for raw in ["https://github.com/Acme/App.git", "git@github.com:acme/app.git", "ssh://git@github.com:22/acme/app", "github.com/acme/app/"] {
            assert_eq!(normalize_repo(raw).as_deref(), Some("github.com/acme/app"), "{raw}");
        }
        assert_eq!(normalize_repo("MyProject").as_deref(), Some("myproject"));
        assert_eq!(normalize_repo("  "), None);
    }

    #[test]
    fn overlap() {
        assert!(paths_overlap("server/src/auth/**", "server/src/auth/login.rs"));
        assert!(paths_overlap("server/src/auth/login.rs", "server/src/auth/**"));
        assert!(paths_overlap("**", "web/x.ts"));
        assert!(paths_overlap("*", "anything"));
        assert!(paths_overlap("web/src/*.tsx", "web/src/**"));
        assert!(paths_overlap("server/src", "server/src/main.rs"), "literal dir covers files under it");
        assert!(paths_overlap("server/src/auth*", "server/src/auth.rs"));
        assert!(paths_overlap("a/b.rs", "a/b.rs"));
        assert!(!paths_overlap("server/src/auth", "server/src/auth.rs"), "sibling file, not under the dir");
        assert!(!paths_overlap("web/**", "server/**"));
        assert!(!paths_overlap("web/src/a.ts", "web/src/b.ts"));
        assert!(!paths_overlap("server/src/auth/**", "server/src/api/mod.rs"));
        // Kinds and repos.
        let (p, r) = (ScopeKind::Path, ScopeKind::Resource);
        assert!(!conflicts(p, "github.com/a/x", "src/**", p, "github.com/a/y", "src/main.rs"));
        assert!(conflicts(p, "", "src/**", p, "github.com/a/y", "src/main.rs"), "unknown repo is conservative");
        assert!(conflicts(r, "", "ci", r, "", "ci"));
        assert!(!conflicts(r, "", "ci", r, "", "deploy:staging"));
        assert!(!conflicts(r, "", "ci", p, "", "ci/**"));
    }

    async fn agent(f: &collab::tests::Fixture, user: &str, name: &str) -> Actor {
        named_agent(&f.db, &f.team, user, name).await.1
    }

    fn taken(o: &ClaimOutcome) -> Vec<String> {
        match o {
            ClaimOutcome::Taken { holders, .. } => holders.iter().map(|h| format!("{} {}", h.display(), h.scope)).collect(),
            other => panic!("expected Taken, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn lease_conflict_queue_and_release() {
        let f = fixture().await;
        let api = agent(&f, &f.paul, "api").await;
        let ui = agent(&f, &f.bob, "ui").await;
        let repo = Some("github.com/acme/app");

        let o = claim(&f.db, &api, repo, "server/src/auth/**", Some("refactor"), None, false).await.unwrap();
        assert!(matches!(o, ClaimOutcome::Granted { renewed: false, .. }));
        let o = claim(&f.db, &ui, repo, "server/src/auth/login.rs", None, None, false).await.unwrap();
        assert_eq!(taken(&o), vec!["paul/api server/src/auth/**"]);
        // Same path in another repo is fine.
        let o = claim(&f.db, &ui, Some("github.com/acme/other"), "server/src/auth/login.rs", None, None, false).await.unwrap();
        assert!(matches!(o, ClaimOutcome::Granted { .. }), "{o:?}");
        // Re-claim by the holder renews.
        let o = claim(&f.db, &api, repo, "server/src/auth/**", None, Some(60), false).await.unwrap();
        assert!(matches!(o, ClaimOutcome::Granted { renewed: true, minutes: 60, .. }));

        // Resource: queue, then release grants + pings.
        claim(&f.db, &api, repo, "ci", Some("e2e"), None, false).await.unwrap();
        let o = claim(&f.db, &ui, repo, "ci", None, None, true).await.unwrap();
        assert!(matches!(o, ClaimOutcome::Queued { position: 1, .. }), "{o:?}");
        let o = claim(&f.db, &ui, repo, "ci", None, None, true).await.unwrap();
        assert!(matches!(o, ClaimOutcome::Queued { position: 1, .. }), "idempotent: {o:?}");
        let checked = check(&f.db, &ui, repo, &["ci".into()]).await.unwrap();
        assert_eq!(checked[0].others.len(), 1);
        assert_eq!(checked[0].waiting, vec!["bob/ui"]);

        let r = release(&f.db, &api, Some("ci"), false, None, Some("  tests green ")).await.unwrap();
        assert_eq!(r.released, vec!["ci"]);
        assert_eq!(r.note, "tests green");
        assert_eq!(r.granted, vec![("ci".to_string(), "bob/ui".to_string())]);
        let v = list(&f.db, &f.team).await.unwrap();
        assert!(v.leases.iter().any(|l| l.scope == "ci" && l.holder.display == "bob/ui"));
        assert!(v.queue.is_empty());
        let pings = collab::take_agent_pings(&f.db, ui.agent_id.as_deref().unwrap()).await.unwrap();
        assert!(
            pings.iter().any(|p| p.body.contains("you now hold ci (you were queued). paul/api released it: \"tests green\". The lease lasts")
                && p.from.display == "paul/api"),
            "{pings:?}"
        );
        let acts = collab::activity(&f.db, &f.team, None, None).await.unwrap();
        let rel = acts.iter().find(|a| a.kind == "released").expect("released event");
        assert_eq!((rel.data["scope"].as_str(), rel.data["repo"].as_str(), rel.data["note"].as_str()), (Some("ci"), Some(""), Some("tests green")));

        // Releasing something you don't hold explains what you do hold.
        let e = release(&f.db, &api, Some("ci"), false, None, None).await.unwrap_err();
        assert!(e.to_string().contains("server/src/auth/**"), "{e}");

        // release(all) frees everything.
        let r = release(&f.db, &api, None, true, None, None).await.unwrap();
        assert_eq!(r.released, vec!["server/src/auth/**"]);
        let o = claim(&f.db, &ui, repo, "server/src/auth/login.rs", None, None, false).await.unwrap();
        assert!(matches!(o, ClaimOutcome::Granted { .. }));
        let kinds: Vec<String> = collab::activity(&f.db, &f.team, None, None).await.unwrap().into_iter().map(|a| a.kind).collect();
        for k in ["claimed", "released", "granted"] {
            assert!(kinds.iter().any(|x| x == k), "{k} in {kinds:?}");
        }
    }

    #[tokio::test]
    async fn admin_force_release_grants_to_waiter() {
        let f = fixture().await;
        let api = agent(&f, &f.paul, "api").await;
        let ui = agent(&f, &f.bob, "ui").await;
        let admin = Actor { team_id: f.team.clone(), user_id: f.paul.clone(), agent_id: None };
        let repo = Some("github.com/acme/app");
        claim(&f.db, &ui, None, "ci", Some("e2e"), None, false).await.unwrap();
        claim(&f.db, &api, None, "ci", None, None, true).await.unwrap();
        claim(&f.db, &ui, repo, "web/**", None, None, false).await.unwrap();

        assert!(matches!(force_release(&f.db, &admin, "paul", "deploy", None, None).await, Err(AppError::NotFound(_))));
        assert!(matches!(force_release(&f.db, &admin, "paul", "web/**", Some("github.com/acme/other"), None).await, Err(AppError::NotFound(_))));

        assert_eq!(force_release(&f.db, &admin, "paul", "ci", None, None).await.unwrap(), 1);
        let v = list(&f.db, &f.team).await.unwrap();
        let ci: Vec<&str> = v.leases.iter().filter(|l| l.scope == "ci").map(|l| l.holder.display.as_str()).collect();
        assert_eq!(ci, vec!["paul/api"], "next waiter holds it");
        assert!(v.queue.is_empty());
        let to_waiter = collab::take_agent_pings(&f.db, api.agent_id.as_deref().unwrap()).await.unwrap();
        assert!(to_waiter.iter().any(|p| p.body.contains("you now hold ci") && p.from.display == "paul"), "{to_waiter:?}");
        let to_holder = collab::take_agent_pings(&f.db, ui.agent_id.as_deref().unwrap()).await.unwrap();
        assert!(to_holder.iter().any(|p| p.body == "paul (admin) released ci"), "{to_holder:?}");

        assert_eq!(force_release(&f.db, &admin, "paul", "web/**", repo, Some("stale")).await.unwrap(), 1);
        let acts = collab::activity(&f.db, &f.team, None, None).await.unwrap();
        let forced: Vec<&serde_json::Value> = acts.iter().filter(|a| a.kind == "lease_force_released").map(|a| &a.data).collect();
        assert_eq!(forced.len(), 2);
        assert!(forced.iter().any(|d| d["scope"] == "ci" && d["repo"] == "" && d["by"] == "paul"), "{forced:?}");
        assert!(forced.iter().any(|d| d["scope"] == "web/**" && d["repo"] == "github.com/acme/app" && d["note"] == "stale"), "{forced:?}");
    }

    #[tokio::test]
    async fn release_without_waiter_or_note() {
        let f = fixture().await;
        let api = agent(&f, &f.paul, "api").await;
        let ui = agent(&f, &f.bob, "ui").await;
        claim(&f.db, &api, None, "ci", None, None, false).await.unwrap();
        let r = release(&f.db, &api, Some("ci"), false, None, Some("tests green")).await.unwrap();
        assert!(r.granted.is_empty());
        assert_eq!(r.note, "tests green");

        // No note: the grant ping still names who released it.
        claim(&f.db, &api, None, "ci", None, None, false).await.unwrap();
        claim(&f.db, &ui, None, "ci", None, None, true).await.unwrap();
        release(&f.db, &api, Some("ci"), false, None, None).await.unwrap();
        let pings = collab::take_agent_pings(&f.db, ui.agent_id.as_deref().unwrap()).await.unwrap();
        assert!(pings.iter().any(|p| p.body.contains("(you were queued). paul/api released it. The lease")), "{pings:?}");
        let notes: Vec<String> = collab::activity(&f.db, &f.team, None, None)
            .await
            .unwrap()
            .into_iter()
            .filter(|a| a.kind == "released")
            .map(|a| a.data["note"].as_str().unwrap_or("?").to_string())
            .collect();
        assert!(notes.contains(&"tests green".to_string()) && notes.contains(&String::new()), "{notes:?}");
    }

    #[tokio::test]
    async fn auto_renew_and_expiry_grants_queue() {
        let f = fixture().await;
        let api = agent(&f, &f.paul, "api").await;
        let ui = agent(&f, &f.bob, "ui").await;
        let api_id = api.agent_id.clone().unwrap();
        claim(&f.db, &api, None, "deploy:staging", Some("deploying"), Some(1), false).await.unwrap();
        claim(&f.db, &ui, None, "deploy:staging", None, None, true).await.unwrap();

        // Renew pushes expiry forward.
        sqlx::query("UPDATE leases SET expires_at = ?2 WHERE agent_id = ?1").bind(api_id.as_str()).bind(now_ms() + 5_000).execute(&f.db).await.unwrap();
        assert_eq!(renew(&f.db, &api_id).await.unwrap(), 1);
        let exp: i64 = sqlx::query_scalar("SELECT expires_at FROM leases WHERE agent_id = ?1").bind(api_id.as_str()).fetch_one(&f.db).await.unwrap();
        assert!(exp > now_ms() + 50_000);

        // Go quiet: lapsed → renew can't revive it; tick expires + grants to the waiter.
        sqlx::query("UPDATE leases SET expires_at = ?2 WHERE agent_id = ?1").bind(api_id.as_str()).bind(now_ms() - 1).execute(&f.db).await.unwrap();
        assert_eq!(renew(&f.db, &api_id).await.unwrap(), 0);
        tick(&f.db, &f.team).await.unwrap();
        let v = list(&f.db, &f.team).await.unwrap();
        assert_eq!(v.leases.len(), 1);
        assert_eq!(v.leases[0].holder.display, "bob/ui");
        let ui_pings = collab::take_agent_pings(&f.db, ui.agent_id.as_deref().unwrap()).await.unwrap();
        assert!(ui_pings.iter().any(|p| p.body.contains("you now hold deploy:staging")));
        let api_pings = collab::take_agent_pings(&f.db, &api_id).await.unwrap();
        assert!(api_pings.iter().any(|p| p.body.contains("lease on deploy:staging expired")));
        let kinds: Vec<String> = collab::activity(&f.db, &f.team, None, None).await.unwrap().into_iter().map(|a| a.kind).collect();
        assert!(kinds.contains(&"expired".to_string()) && kinds.contains(&"granted".to_string()));
    }

    #[tokio::test]
    async fn restarted_agent_resumes_its_leases() {
        let f = fixture().await;
        let api = agent(&f, &f.paul, "api").await;
        claim(&f.db, &api, None, "ci", None, None, false).await.unwrap();
        // Busy in another session right now → suffix, no leases.
        let mut s2 = collab::tests::session(&f.db, &f.team, &f.paul, "claude-code").await;
        assert_eq!(collab::hello(&f.db, &mut s2, "api", None, None).await.unwrap().agent.name, "api-2");
        // Old process exited without closing its session; after a short idle a new one resumes it.
        sqlx::query("UPDATE agents SET last_seen_at = ?1 WHERE name = 'api'").bind(now_ms() - 120_000).execute(&f.db).await.unwrap();
        let mut s3 = collab::tests::session(&f.db, &f.team, &f.paul, "claude-code").await;
        let o = collab::hello(&f.db, &mut s3, "api", None, None).await.unwrap();
        assert!(o.adopted && o.agent.id == api.agent_id.clone().unwrap());
        assert_eq!(release(&f.db, &api, Some("ci"), false, None, None).await.unwrap().released, vec!["ci"]);
    }

    #[tokio::test]
    async fn task_leases_drive_the_post() {
        let f = fixture().await;
        let api = agent(&f, &f.paul, "api").await;
        let ui = agent(&f, &f.bob, "ui").await;
        let p = collab::create_post(
            &f.db,
            &api,
            crate::models::NewPost { board: "backend".into(), title: "Fix auth".into(), body: String::new(), kind: Some("task".into()), status: None, priority: None },
        )
        .await
        .unwrap();
        let tag = format!("#{}", p.number);
        let o = claim(&f.db, &api, None, &tag, None, None, false).await.unwrap();
        match o {
            ClaimOutcome::Granted { post: Some(p), .. } => assert_eq!(p.status, "in_progress"),
            other => panic!("{other:?}"),
        }
        assert_eq!(taken(&claim(&f.db, &ui, None, &tag, None, None, false).await.unwrap()), vec![format!("paul/api {tag}")]);
        let r = release(&f.db, &api, Some(&tag), false, Some("done"), Some("shipped in abc123")).await.unwrap();
        assert_eq!(r.posts[0].status, "done");
        let d = collab::get_post(&f.db, &f.team, p.number).await.unwrap();
        assert!(d.post.claimed_by.is_none());
        assert_eq!(d.comments[0].body, "shipped in abc123");
    }
}
