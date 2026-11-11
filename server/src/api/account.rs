//! Me, orgs (teams), members, invites, API tokens.

use std::sync::LazyLock;

use axum::{
    extract::{Query, State},
    routing::{delete, get, patch, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::extract::{JsonBody, JsonOrEmpty, PathParam};
use crate::{
    auth::{require_admin, require_member, CurrentUser, RateLimiter},
    config::MAX_TEAM_MEMBERS,
    domain::accounts,
    email::{self, MagicLinkPurpose},
    error::{AppError, AppResult},
    state::AppState,
    util::now_ms,
};

const HOUR_MS: i64 = 60 * 60 * 1000;
const EMAIL_INVITES_PER_USER_HOUR: usize = 10;
const EMAIL_INVITES_PER_ORG_DAY: usize = 30;
/// Across all orgs, so many throwaway orgs can't flood one inbox.
const EMAIL_INVITES_PER_RECIPIENT_DAY: usize = 3;

static INVITE_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/me", get(get_me).patch(patch_me))
        .route("/api/orgs", post(create_org))
        .route("/api/orgs/slug", get(check_slug))
        .route("/api/orgs/{slug}", get(get_org))
        .route("/api/teams/{tid}", get(get_team).patch(patch_team))
        .route("/api/teams/{tid}/invite", post(rotate_invite))
        .route("/api/teams/{tid}/invites/email", post(email_invite))
        .route("/api/teams/{tid}/members/{uid}", patch(patch_member).delete(remove_member))
        .route("/api/teams/{tid}/transfer", post(transfer))
        .route("/api/invites/{code}", get(invite_info))
        .route("/api/invites/{code}/accept", post(accept_invite))
        .route("/api/teams/{tid}/tokens", get(list_tokens).post(create_token))
        .route("/api/tokens/{id}", delete(revoke_token))
}

fn ok() -> Json<Value> {
    Json(json!({ "ok": true }))
}

// --- me ---------------------------------------------------------------------------------------

async fn get_me(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> AppResult<Json<Value>> {
    let teams = accounts::list_user_teams(&state.db, &user.id).await?;
    Ok(Json(json!({ "user": user, "teams": teams })))
}

#[derive(Deserialize)]
struct PatchMe {
    handle: Option<String>,
}

async fn patch_me(State(state): State<AppState>, CurrentUser(user): CurrentUser, JsonBody(body): JsonBody<PatchMe>) -> AppResult<Json<Value>> {
    let user = match body.handle {
        Some(h) if !h.trim().eq_ignore_ascii_case(&user.handle) => accounts::update_handle(&state.db, &user.id, &h).await?,
        _ => user,
    };
    Ok(Json(json!({ "user": user })))
}

// --- orgs -------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct NewOrg {
    name: String,
    slug: Option<String>,
}

/// `{name, slug?}` -> TeamSummary; the caller becomes owner.
async fn create_org(State(state): State<AppState>, CurrentUser(user): CurrentUser, JsonBody(body): JsonBody<NewOrg>) -> AppResult<Json<accounts::TeamSummary>> {
    Ok(Json(accounts::create_org(&state.db, &user.id, &body.name, body.slug.as_deref()).await?))
}

#[derive(Deserialize)]
struct SlugQuery {
    #[serde(default)]
    slug: String,
}

/// `?slug=x` -> `{slug, available, reason?}` for live checks while typing.
async fn check_slug(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
    Query(q): Query<SlugQuery>,
) -> AppResult<Json<accounts::SlugAvailability>> {
    if q.slug.len() > 256 {
        return Err(AppError::BadRequest("slug is too long".into()));
    }
    Ok(Json(accounts::slug_availability(&state.db, &q.slug).await?))
}

/// TeamSummary for members; 404 for everyone else, so slugs don't leak which orgs exist.
async fn get_org(State(state): State<AppState>, CurrentUser(user): CurrentUser, PathParam(slug): PathParam<String>) -> AppResult<Json<accounts::TeamSummary>> {
    let org = accounts::org_by_slug(&state.db, &slug, &user.id).await?;
    Ok(Json(org.ok_or_else(|| AppError::NotFound("org not found".into()))?))
}

// --- teams ------------------------------------------------------------------------------------

async fn get_team(State(state): State<AppState>, CurrentUser(user): CurrentUser, PathParam(tid): PathParam<String>) -> AppResult<Json<Value>> {
    require_member(&state, &tid, &user.id).await?;
    let team = accounts::get_team(&state.db, &tid).await?.ok_or_else(|| AppError::NotFound("team not found".into()))?;
    let members = accounts::list_members(&state.db, &tid).await?;
    // The standing invite is attributed to the owner; rotating re-attributes it to whoever rotated.
    let code = accounts::current_invite(&state.db, &tid, &team.owner_id).await?;
    let url = accounts::invite_url(&state.cfg, &code);
    Ok(Json(json!({
        "team": team,
        "members": members,
        "invite": { "code": code, "url": url },
        "max_members": MAX_TEAM_MEMBERS,
    })))
}

#[derive(Deserialize)]
struct PatchTeam {
    name: Option<String>,
    slug: Option<String>,
}

async fn patch_team(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
    JsonBody(body): JsonBody<PatchTeam>,
) -> AppResult<Json<Value>> {
    require_member(&state, &tid, &user.id).await?;
    // Name: owner or admin. Slug: owner only (checked in the domain, along with no-op edits).
    let team = accounts::update_team(&state.db, &tid, &user.id, body.name.as_deref(), body.slug.as_deref()).await?;
    Ok(Json(json!({ "team": team })))
}

async fn rotate_invite(State(state): State<AppState>, CurrentUser(user): CurrentUser, PathParam(tid): PathParam<String>) -> AppResult<Json<Value>> {
    require_admin(&state, &tid, &user.id).await?;
    let code = accounts::rotate_invite(&state.db, &tid, &user.id).await?;
    let url = accounts::invite_url(&state.cfg, &code);
    Ok(Json(json!({ "code": code, "url": url })))
}

async fn remove_member(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam((tid, uid)): PathParam<(String, String)>,
) -> AppResult<Json<Value>> {
    accounts::remove_member(&state.db, &tid, &user.id, &uid).await?;
    Ok(ok())
}

#[derive(Deserialize)]
struct PatchMember {
    role: String,
}

/// Owner only: `{role: "admin" | "member"}` -> the member.
async fn patch_member(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam((tid, uid)): PathParam<(String, String)>,
    JsonBody(body): JsonBody<PatchMember>,
) -> AppResult<Json<accounts::Member>> {
    Ok(Json(accounts::set_role(&state.db, &tid, &user.id, &uid, &body.role).await?))
}

#[derive(Deserialize)]
struct Transfer {
    user_id: String,
}

/// Owner only: the member becomes owner, the previous owner becomes admin.
async fn transfer(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
    JsonBody(body): JsonBody<Transfer>,
) -> AppResult<Json<Value>> {
    accounts::transfer_ownership(&state.db, &tid, &user.id, &body.user_id).await?;
    Ok(ok())
}

// --- invites ----------------------------------------------------------------------------------

/// Public: what the /join page shows before sign-in.
async fn invite_info(State(state): State<AppState>, PathParam(code): PathParam<String>) -> AppResult<Json<accounts::InviteInfo>> {
    let not_found = || AppError::NotFound("this invite link is no longer valid".into());
    if code.len() > 64 {
        return Err(not_found());
    }
    Ok(Json(accounts::invite_info(&state.db, &code).await?.ok_or_else(not_found)?))
}

async fn accept_invite(State(state): State<AppState>, CurrentUser(user): CurrentUser, PathParam(code): PathParam<String>) -> AppResult<Json<Value>> {
    if code.len() > 64 {
        return Err(AppError::NotFound("this invite link is no longer valid".into()));
    }
    let (team_id, _) = accounts::join_via_invite(&state.db, &code, &user.id).await?;
    let team = accounts::get_team(&state.db, &team_id).await?.ok_or_else(|| AppError::NotFound("org not found".into()))?;
    Ok(Json(json!({ "team_id": team_id, "slug": team.slug })))
}

#[derive(Deserialize)]
struct EmailInvite {
    email: String,
}

/// Any member: `{email}` -> `{ok:true}`. Emails a magic link that signs the invitee up or in and
/// joins them to the org (the login token carries the org's current invite code).
async fn email_invite(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
    JsonBody(body): JsonBody<EmailInvite>,
) -> AppResult<Json<Value>> {
    require_member(&state, &tid, &user.id).await?;
    let to = accounts::normalize_email(&body.email).ok_or_else(|| AppError::BadRequest("enter a valid email".into()))?;
    accounts::check_email_invite(&state.db, &tid, &to).await?;
    let now = now_ms();
    if !INVITE_LIMITER.allow(&format!("invite-user:{}", user.id), EMAIL_INVITES_PER_USER_HOUR, HOUR_MS, now) {
        return Err(AppError::RateLimited("you've sent a lot of invites; try again in an hour"));
    }
    if !INVITE_LIMITER.allow(&format!("invite-org:{tid}"), EMAIL_INVITES_PER_ORG_DAY, 24 * HOUR_MS, now) {
        return Err(AppError::RateLimited("this org has sent a lot of invites today; try again tomorrow"));
    }
    if !INVITE_LIMITER.allow(&format!("invite-to:{to}"), EMAIL_INVITES_PER_RECIPIENT_DAY, 24 * HOUR_MS, now)
        || !crate::auth::allow_email_to(&to, now)
    {
        return Err(AppError::RateLimited("that address has been sent several invites already; try again later"));
    }

    let team = accounts::get_team(&state.db, &tid).await?.ok_or_else(|| AppError::NotFound("team not found".into()))?;
    let code = accounts::current_invite(&state.db, &tid, &team.owner_id).await?;
    let secret = accounts::create_login_token(&state.db, &to, Some(&code)).await?;
    let link = format!("{}/auth?token={}", state.cfg.web_url, secret);
    let purpose = MagicLinkPurpose::Join { team_name: team.name, inviter: user.handle.clone() };
    email::send_magic_link(&state, &to, &link, purpose).await?;
    accounts::record_invite_sent(&state.db, &tid, &user, &to).await?;
    Ok(ok())
}

// --- API tokens -------------------------------------------------------------------------------

async fn list_tokens(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
) -> AppResult<Json<Vec<accounts::ApiTokenInfo>>> {
    require_member(&state, &tid, &user.id).await?;
    Ok(Json(accounts::list_api_tokens(&state.db, &user.id, &tid).await?))
}

#[derive(Deserialize, Default)]
struct NewToken {
    label: Option<String>,
}

async fn create_token(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
    JsonOrEmpty(body): JsonOrEmpty<NewToken>,
) -> AppResult<Json<Value>> {
    require_member(&state, &tid, &user.id).await?;
    let created = accounts::create_api_token(&state.db, &user.id, &tid, body.label.as_deref()).await?;
    let setup = crate::mcp::setup_snippets(&state.cfg, &created.token);
    Ok(Json(json!({
        "id": created.id,
        "prefix": created.prefix,
        "label": created.label,
        "token": created.token,
        "setup": setup,
    })))
}

async fn revoke_token(State(state): State<AppState>, CurrentUser(user): CurrentUser, PathParam(id): PathParam<String>) -> AppResult<Json<Value>> {
    accounts::revoke_api_token(&state.db, &user.id, &id).await?;
    Ok(ok())
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{Arc, Mutex},
    };

    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::*;
    use crate::{config::Config, db::test_pool};

    async fn test_state() -> AppState {
        let cfg = Config {
            bind: "127.0.0.1:0".into(),
            database_url: "sqlite::memory:".into(),
            data_dir: std::env::temp_dir(),
            web_url: "http://localhost:5191".into(),
            public_api_url: "http://localhost:8191".into(),
            resend_api_key: None,
            email_from: "swarmboard <login@swarmboard.app>".into(),
            cookie_secure: false,
            s3: None,
            backup_keep: 0,
        };
        AppState::new(test_pool().await, cfg).unwrap()
    }

    async fn user_with_session(db: &sqlx::SqlitePool, email: &str) -> (accounts::User, String) {
        let (u, _) = accounts::find_or_create_user(db, email).await.unwrap();
        let cookie = format!("sb_session={}", accounts::create_session(db, &u.id).await.unwrap());
        (u, cookie)
    }

    async fn invite(app: &Router, tid: &str, cookie: &str, email: &str) -> (StatusCode, Value) {
        let req = Request::post(format!("/api/teams/{tid}/invites/email"))
            .header(header::COOKIE, cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "email": email }).to_string()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Invite -> logged link (no RESEND key) -> verify joins the org.
    #[tokio::test]
    async fn email_invite_happy_path() {
        let logs = Capture::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt().with_ansi(false).with_writer(move || writer.clone()).finish();
        // With a single registered dispatcher, tracing-core computes callsite interest from whichever
        // thread hits it first (other tests, no subscriber -> "disabled"). A second one disables that shortcut.
        let _second = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
        let _guard = tracing::subscriber::set_default(subscriber);
        tracing::callsite::rebuild_interest_cache();

        let state = test_state().await;
        let app = crate::app(state.clone());
        let db = &state.db;
        let (owner, c_owner) = user_with_session(db, "owner@example.com").await;
        let tid = accounts::create_org(db, &owner.id, "Acme", None).await.unwrap().id;

        let (st, b) = invite(&app, &tid, &c_owner, "nope").await;
        assert_eq!((st, b["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_request")));

        let (st, b) = invite(&app, &tid, &c_owner, " Friend@Example.COM ").await;
        assert_eq!((st, b), (StatusCode::OK, json!({"ok": true})));

        let code = accounts::current_invite(db, &tid, &owner.id).await.unwrap();
        let (email, stored): (String, Option<String>) =
            sqlx::query_as("SELECT email, invite_code FROM login_tokens").fetch_one(db).await.unwrap();
        assert_eq!((email.as_str(), stored.as_deref()), ("friend@example.com", Some(code.as_str())));

        let data: String = sqlx::query_scalar("SELECT data FROM events WHERE team_id = ? AND kind = 'invite_sent'")
            .bind(&tid)
            .fetch_one(db)
            .await
            .unwrap();
        let data: Value = serde_json::from_str(&data).unwrap();
        assert_eq!(data, json!({"by": "owner", "by_id": owner.id, "email_domain": "example.com"}));

        let logged = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        let link = logged
            .split_whitespace()
            .find_map(|w| w.strip_prefix("link="))
            .unwrap_or_else(|| panic!("magic link logged without RESEND_API_KEY: {logged}"))
            .to_string();
        let secret = link.strip_prefix("http://localhost:5191/auth?token=").expect("link points at the web app");

        let req = Request::post("/api/auth/verify")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "token": secret }).to_string()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["user"]["email"], "friend@example.com");
        assert_eq!(body["joined_team_id"], tid.as_str());
        assert_eq!(body["joined_team_slug"], "acme");
        assert_eq!(body["needs_org"], false);
        assert_eq!(body["join_error"], Value::Null);

        // Now a member: inviting them again is a conflict.
        let (st, b) = invite(&app, &tid, &c_owner, "friend@example.com").await;
        assert_eq!((st, b["message"].as_str()), (StatusCode::CONFLICT, Some("already a member")));
    }

    #[tokio::test]
    async fn email_invite_conflicts_and_access() {
        let state = test_state().await;
        let app = crate::app(state.clone());
        let db = &state.db;
        let (owner, c_owner) = user_with_session(db, "boss@example.com").await;
        let tid = accounts::create_org(db, &owner.id, "Full House", None).await.unwrap().id;

        // Any member may invite, not just admins; the owner's own address counts as a member.
        let (st, b) = invite(&app, &tid, &c_owner, "BOSS@example.com").await;
        assert_eq!((st, b), (StatusCode::CONFLICT, json!({"error": "conflict", "message": "already a member"})));

        let code = accounts::current_invite(db, &tid, &owner.id).await.unwrap();
        let mut member_cookie = String::new();
        for h in ["ann", "bob", "cat", "dan"] {
            let (u, c) = user_with_session(db, &format!("{h}@example.com")).await;
            accounts::join_via_invite(db, &code, &u.id).await.unwrap();
            member_cookie = c;
        }
        let (st, b) = invite(&app, &tid, &member_cookie, "eve@example.com").await;
        assert_eq!((st, b["message"].as_str()), (StatusCode::CONFLICT, Some("this team is full (5 people)")));

        // Not a member of any org, and a member of another org: both 403.
        let (_, c_out) = user_with_session(db, "outsider@example.com").await;
        let (st, b) = invite(&app, &tid, &c_out, "eve@example.com").await;
        assert_eq!((st, b["error"].as_str()), (StatusCode::FORBIDDEN, Some("forbidden")));
        let (other, c_other) = user_with_session(db, "other@example.com").await;
        let other_tid = accounts::create_org(db, &other.id, "Other", None).await.unwrap().id;
        let (st, _) = invite(&app, &tid, &c_other, "eve@example.com").await;
        assert_eq!(st, StatusCode::FORBIDDEN);
        let (st, _) = invite(&app, &other_tid, &c_owner, "eve@example.com").await;
        assert_eq!(st, StatusCode::FORBIDDEN);
        let (st, _) = invite(&app, &tid, "sb_session=nope", "eve@example.com").await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);

        let sent: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM login_tokens").fetch_one(db).await.unwrap();
        assert_eq!(sent, 0, "no token is minted for refused invites");
    }

    #[tokio::test]
    async fn email_invite_rate_limits() {
        let state = test_state().await;
        let app = crate::app(state.clone());
        let db = &state.db;
        let (owner, c_owner) = user_with_session(db, "rl-owner@example.com").await;
        let tid = accounts::create_org(db, &owner.id, "Busy", None).await.unwrap().id;
        let code = accounts::current_invite(db, &tid, &owner.id).await.unwrap();
        let mut cookies = vec![];
        for h in ["rl-adam", "rl-mia", "rl-max"] {
            let (u, c) = user_with_session(db, &format!("{h}@example.com")).await;
            accounts::join_via_invite(db, &code, &u.id).await.unwrap();
            cookies.push(c);
        }

        // 10 per user per hour.
        for i in 0..10 {
            assert_eq!(invite(&app, &tid, &c_owner, &format!("o{i}@example.com")).await.0, StatusCode::OK);
        }
        let (st, b) = invite(&app, &tid, &c_owner, "o10@example.com").await;
        assert_eq!((st, b["error"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited")));

        // 30 per org per day, across members.
        for c in &cookies[..2] {
            for i in 0..10 {
                assert_eq!(invite(&app, &tid, c, &format!("m{i}@example.com")).await.0, StatusCode::OK);
            }
        }
        let (st, b) = invite(&app, &tid, &cookies[2], "late@example.com").await;
        assert_eq!(st, StatusCode::TOO_MANY_REQUESTS);
        assert!(b["message"].as_str().unwrap().contains("today"), "{b}");

        let sent: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE team_id = ? AND kind = 'invite_sent'")
            .bind(&tid)
            .fetch_one(db)
            .await
            .unwrap();
        assert_eq!(sent, 30);
    }
}
