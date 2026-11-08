//! Browser auth: magic-link sign-in and cookie sessions. OWNER: backend/auth subagent.
//!
//! - `POST /api/auth/email  {email, invite?}` -> emails a 20-minute single-use link, always `{ok:true}`
//! - `POST /api/auth/verify {token}`          -> consumes it, sets the `sb_session` cookie
//! - `POST /api/auth/logout`                  -> deletes the session, clears the cookie
//!
//! `CurrentUser` is the extractor every dashboard route uses; `require_member` checks team access.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use axum::{
    extract::{FromRequestParts, State},
    http::{request::Parts, HeaderMap, Method},
    routing::post,
    Json, Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    api::extract::JsonBody,
    config::Config,
    domain::accounts::{self, Role, User, SESSION_TTL_MS},
    email::{self, MagicLinkPurpose},
    error::{AppError, AppResult},
    state::AppState,
    util::now_ms,
};

pub const SESSION_COOKIE: &str = "sb_session";

const WINDOW_MS: i64 = 15 * 60 * 1000;
const LINKS_PER_EMAIL: usize = 5;
const LINKS_PER_IP: usize = 20;
/// Safety net for the email quota if per-IP limits are dodged with spoofed headers.
const LINKS_GLOBAL: usize = 500;
const VERIFIES_PER_IP: usize = 30;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/email", post(request_link))
        .route("/api/auth/verify", post(verify))
        .route("/api/auth/logout", post(logout))
}

// ---------------------------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct EmailRequest {
    email: String,
    #[serde(default)]
    invite: Option<String>,
}

async fn request_link(State(state): State<AppState>, headers: HeaderMap, JsonBody(req): JsonBody<EmailRequest>) -> AppResult<Json<Value>> {
    check_origin(&state.cfg, &Method::POST, &headers)?;
    let email = accounts::normalize_email(&req.email).ok_or_else(|| AppError::BadRequest("enter a valid email".into()))?;
    let ip = client_ip(&headers);
    let now = now_ms();
    if !LIMITER.allow(&format!("link-ip:{ip}"), LINKS_PER_IP, WINDOW_MS, now)
        || !LIMITER.allow(&format!("link-email:{email}"), LINKS_PER_EMAIL, WINDOW_MS, now)
        || !LIMITER.allow("link-global", LINKS_GLOBAL, WINDOW_MS, now)
    {
        tracing::warn!(%ip, %email, "magic link rate limited");
        return Err(AppError::TooManyRequests);
    }

    // An invite only changes the email's wording and what happens after verify;
    // unknown codes are silently ignored so the response never differs.
    let code = req.invite.map(|c| c.trim().to_string()).filter(|c| !c.is_empty() && c.len() <= 64);
    let invite = match &code {
        Some(code) => accounts::invite_info(&state.db, code).await?,
        None => None,
    };
    let purpose = match &invite {
        Some(info) if !info.full => MagicLinkPurpose::Join { team_name: info.team_name.clone(), inviter: info.inviter_handle.clone() },
        _ => MagicLinkPurpose::SignIn,
    };
    let stored_code = if invite.is_some() { code.as_deref() } else { None };

    let secret = accounts::create_login_token(&state.db, &email, stored_code).await?;
    let link = format!("{}/auth?token={}", state.cfg.web_url, secret);
    email::send_magic_link(&state, &email, &link, purpose).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct VerifyRequest {
    token: String,
}

async fn verify(State(state): State<AppState>, headers: HeaderMap, jar: CookieJar, JsonBody(req): JsonBody<VerifyRequest>) -> AppResult<(CookieJar, Json<Value>)> {
    check_origin(&state.cfg, &Method::POST, &headers)?;
    let ip = client_ip(&headers);
    if !LIMITER.allow(&format!("verify-ip:{ip}"), VERIFIES_PER_IP, WINDOW_MS, now_ms()) {
        return Err(AppError::TooManyRequests);
    }
    let claim = accounts::consume_login_token(&state.db, &req.token)
        .await?
        .ok_or_else(|| AppError::BadRequest("this link has expired or was already used".into()))?;
    let (user, created) = accounts::find_or_create_user(&state.db, &claim.email).await?;

    let mut joined_team_id = None;
    let mut join_error = None;
    if let Some(code) = &claim.invite_code {
        match accounts::join_via_invite(&state.db, code, &user.id).await {
            Ok((team_id, _)) => joined_team_id = Some(team_id),
            Err(e @ (AppError::Conflict(_) | AppError::NotFound(_))) => join_error = Some(e.to_string()),
            Err(e) => return Err(e),
        }
    }

    // Replace (not stack) an existing session in this browser.
    if let Some(old) = jar.get(SESSION_COOKIE) {
        accounts::delete_session(&state.db, old.value()).await?;
    }
    let secret = accounts::create_session(&state.db, &user.id).await?;
    let teams = accounts::list_user_teams(&state.db, &user.id).await?;
    let joined_team_slug = joined_team_id.as_ref().and_then(|id| teams.iter().find(|t| &t.id == id)).map(|t| t.slug.clone());
    // No orgs yet: the web shows "Name your org".
    let needs_org = teams.is_empty();
    if created {
        crate::notify::signup(
            &state,
            crate::notify::Signup { email: user.email.clone(), handle: user.handle.clone(), ip: ip.clone(), joined: joined_team_slug.clone() },
            &headers,
        );
    }
    let jar = jar.add(session_cookie(&state.cfg, &secret));
    Ok((
        jar,
        Json(json!({
            "user": user,
            "teams": teams,
            "joined_team_id": joined_team_id,
            "joined_team_slug": joined_team_slug,
            "join_error": join_error,
            "needs_org": needs_org,
        })),
    ))
}

async fn logout(State(state): State<AppState>, headers: HeaderMap, jar: CookieJar) -> AppResult<(CookieJar, Json<Value>)> {
    check_origin(&state.cfg, &Method::POST, &headers)?;
    if let Some(c) = jar.get(SESSION_COOKIE) {
        accounts::delete_session(&state.db, c.value()).await?;
    }
    Ok((jar.add(clear_cookie(&state.cfg)), Json(json!({ "ok": true }))))
}

// ---------------------------------------------------------------------------------------------
// Cookies
// ---------------------------------------------------------------------------------------------

/// `sb_session=<secret>; HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000` (+ `Secure`).
pub fn session_cookie(cfg: &Config, secret: &str) -> Cookie<'static> {
    cookie_with(cfg, secret, SESSION_TTL_MS / 1000)
}

fn clear_cookie(cfg: &Config) -> Cookie<'static> {
    cookie_with(cfg, "", 0)
}

fn cookie_with(cfg: &Config, value: &str, max_age_secs: i64) -> Cookie<'static> {
    // Parsed from a string so Max-Age can be set without pulling in the `time` crate.
    let mut s = format!("{SESSION_COOKIE}={value}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age_secs}");
    if cfg.cookie_secure {
        s.push_str("; Secure");
    }
    Cookie::parse(s).expect("static cookie format")
}

fn session_secret(headers: &HeaderMap) -> Option<String> {
    CookieJar::from_headers(headers)
        .get(SESSION_COOKIE)
        .map(|c| c.value().to_string())
        .filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------------------------------------
// Extractor + membership check
// ---------------------------------------------------------------------------------------------

/// The signed-in human (cookie session). Rejects with 401 `{error:"unauthorized"}`.
#[derive(Debug, Clone)]
pub struct CurrentUser(pub User);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if let Some(cached) = parts.extensions.get::<CurrentUser>() {
            return Ok(cached.clone());
        }
        check_origin(&state.cfg, &parts.method, &parts.headers)?;
        let secret = session_secret(&parts.headers).ok_or(AppError::Unauthorized)?;
        let user = accounts::session_user(&state.db, &secret).await?.ok_or(AppError::Unauthorized)?;
        let current = CurrentUser(user);
        parts.extensions.insert(current.clone());
        Ok(current)
    }
}

/// The user's role in the team, or 403.
pub async fn require_member(state: &AppState, team_id: &str, user_id: &str) -> AppResult<Role> {
    accounts::member_role(&state.db, team_id, user_id)
        .await?
        .ok_or_else(|| AppError::Forbidden("not a member of this team".into()))
}

/// Like `require_member`, but only the owner or an admin passes.
pub async fn require_admin(state: &AppState, team_id: &str, user_id: &str) -> AppResult<Role> {
    let role = require_member(state, team_id, user_id).await?;
    if !role.is_admin() {
        return Err(AppError::Forbidden("only the team owner or an admin can do that".into()));
    }
    Ok(role)
}

// ---------------------------------------------------------------------------------------------
// CSRF: cookie-authenticated writes must come from our own origin (when the browser says).
// SameSite=Lax already blocks cross-site POSTs; this also covers sibling subdomains.
// ---------------------------------------------------------------------------------------------

fn check_origin(cfg: &Config, method: &Method, headers: &HeaderMap) -> AppResult<()> {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(());
    }
    match headers.get("origin").and_then(|v| v.to_str().ok()) {
        None => Ok(()), // non-browser clients (curl) and some same-origin requests send none
        Some(origin) if origin_allowed(cfg, origin) => Ok(()),
        Some(origin) => {
            tracing::warn!(%origin, "rejected cross-origin request");
            Err(AppError::Forbidden("cross-origin request rejected".into()))
        }
    }
}

fn origin_allowed(cfg: &Config, origin: &str) -> bool {
    let origin = origin.trim().trim_end_matches('/');
    origin.eq_ignore_ascii_case(&cfg.web_url)
        || origin.eq_ignore_ascii_case(&cfg.public_api_url)
        || (is_loopback(origin) && is_loopback(&cfg.web_url))
}

/// http(s)://localhost[:port], 127.0.0.1, [::1], *.localhost
fn is_loopback(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")) else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    let host = if authority.starts_with('[') {
        authority.split(']').next().unwrap_or("").trim_start_matches('[')
    } else {
        authority.split(':').next().unwrap_or("")
    };
    let host = host.to_ascii_lowercase();
    host == "localhost" || host == "127.0.0.1" || host == "::1" || host.ends_with(".localhost")
}

// ---------------------------------------------------------------------------------------------
// Client IP + rate limiting
// ---------------------------------------------------------------------------------------------

/// First hop of `X-Forwarded-For` (set by Vercel and passed through Caddy), then `X-Real-IP`.
pub fn client_ip(headers: &HeaderMap) -> String {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    // First non-empty entry: Caddy sends "<incoming X-Forwarded-For>, <peer>", so a direct caller with no
    // header of its own yields ", <peer>" and we fall through to the real peer address.
    let first_hop = header("x-forwarded-for").and_then(|v| v.split(',').map(str::trim).find(|s| !s.is_empty()));
    let ip = first_hop.or_else(|| header("x-real-ip").map(str::trim).filter(|s| !s.is_empty())).unwrap_or("unknown");
    ip.chars().take(64).collect()
}

/// Sliding-window counter per key. In-memory: resets on restart, which is fine for this.
pub struct RateLimiter {
    hits: Mutex<HashMap<String, Vec<i64>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        RateLimiter { hits: Mutex::new(HashMap::new()) }
    }

    /// Records a hit for `key` and returns true if it is within `limit` hits per `window_ms`.
    /// Rejected attempts are not recorded.
    pub fn allow(&self, key: &str, limit: usize, window_ms: i64, now: i64) -> bool {
        let mut hits = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        if hits.len() > 10_000 {
            hits.retain(|_, v| v.last().is_some_and(|t| now - t < window_ms));
        }
        let v = hits.entry(key.to_string()).or_default();
        v.retain(|t| now - t < window_ms);
        if v.len() >= limit {
            return false;
        }
        v.push(now);
        true
    }
}

static LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {

    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::*;
    use crate::db::test_pool;

    fn test_cfg() -> Config {
        Config {
            bind: "127.0.0.1:0".into(),
            database_url: "sqlite::memory:".into(),
            data_dir: std::env::temp_dir(),
            web_url: "http://localhost:5191".into(),
            public_api_url: "http://localhost:8191".into(),
            resend_api_key: None,
            email_from: "swarmboard <login@swarmboard.app>".into(),
            cookie_secure: false,
            s3: None,
        }
    }

    async fn test_state() -> AppState {
        AppState::new(test_pool().await, test_cfg()).unwrap()
    }

    fn post_json(uri: &str, body: Value) -> Request<Body> {
        Request::post(uri).header(header::CONTENT_TYPE, "application/json").body(Body::from(body.to_string())).unwrap()
    }

    async fn body_json(res: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    fn set_cookie(res: &axum::response::Response) -> String {
        res.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap().to_string()
    }

    #[test]
    fn rate_limiter_window() {
        let rl = RateLimiter::new();
        for i in 0..5 {
            assert!(rl.allow("k", 5, 1000, 100 + i));
        }
        assert!(!rl.allow("k", 5, 1000, 200));
        assert!(rl.allow("other", 5, 1000, 200), "keys are independent");
        assert!(!rl.allow("k", 5, 1000, 1099), "still inside the window");
        assert!(rl.allow("k", 5, 1000, 1100), "oldest hit aged out");
    }

    #[test]
    fn client_ip_direct_caller_peer() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", ", 198.51.100.9".parse().unwrap());
        assert_eq!(client_ip(&h), "198.51.100.9");
    }

    #[test]
    fn client_ip_first_hop() {
        let mut h = HeaderMap::new();
        assert_eq!(client_ip(&h), "unknown");
        h.insert("x-real-ip", "10.0.0.9".parse().unwrap());
        assert_eq!(client_ip(&h), "10.0.0.9");
        h.insert("x-forwarded-for", " 203.0.113.7 , 76.76.21.21, 172.18.0.2".parse().unwrap());
        assert_eq!(client_ip(&h), "203.0.113.7");
    }

    #[test]
    fn origins() {
        let mut cfg = test_cfg();
        assert!(origin_allowed(&cfg, "http://localhost:5191"));
        assert!(origin_allowed(&cfg, "http://127.0.0.1:5191"), "loopback in dev");
        assert!(!origin_allowed(&cfg, "https://evil.example"));
        cfg.web_url = "https://swarmboard.app".into();
        cfg.public_api_url = "https://api.swarmboard.app".into();
        assert!(origin_allowed(&cfg, "https://swarmboard.app"));
        assert!(origin_allowed(&cfg, "https://api.swarmboard.app"));
        assert!(!origin_allowed(&cfg, "http://localhost:5191"), "no loopback in prod");
        assert!(!origin_allowed(&cfg, "https://evil.swarmboard.app"));
        assert!(!origin_allowed(&cfg, "null"));
        assert!(is_loopback("http://[::1]:8191"));
        assert!(is_loopback("http://app.localhost"));
        assert!(!is_loopback("http://localhost.evil.com"));
    }

    #[test]
    fn cookie_attributes() {
        let mut cfg = test_cfg();
        let c = session_cookie(&cfg, "abc").to_string();
        assert!(c.starts_with("sb_session=abc"));
        for attr in ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=2592000"] {
            assert!(c.contains(attr), "{c} missing {attr}");
        }
        assert!(!c.contains("Secure"));
        cfg.cookie_secure = true;
        assert!(session_cookie(&cfg, "abc").to_string().contains("Secure"));
        assert!(clear_cookie(&cfg).to_string().contains("Max-Age=0"));
    }

    /// Full browser flow through the router: email -> verify (cookie) -> /api/me -> logout.
    #[tokio::test]
    async fn magic_link_flow() {
        let state = test_state().await;
        let app = crate::app(state.clone());

        // Invalid email is a 400; a valid one is always {ok:true}.
        let res = app.clone().oneshot(post_json("/api/auth/email", json!({"email": "nope"}))).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(res).await["error"], "bad_request");
        let res = app
            .clone()
            .oneshot(post_json("/api/auth/email", json!({"email": "Flow@Example.com", "invite": "bogus"})))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body_json(res).await, json!({"ok": true}));
        let (email, invite): (String, Option<String>) = sqlx::query_as("SELECT email, invite_code FROM login_tokens")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!((email.as_str(), invite), ("flow@example.com", None), "unknown invite codes are dropped");

        // The emailed secret isn't recoverable from the DB, so mint one the same way for verify.
        let secret = accounts::create_login_token(&state.db, "flow@example.com", None).await.unwrap();
        let res = app.clone().oneshot(post_json("/api/auth/verify", json!({"token": secret}))).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let cookie = set_cookie(&res);
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax") && cookie.contains("Max-Age=2592000"));
        let body = body_json(res).await;
        assert_eq!(body["user"]["handle"], "flow");
        assert_eq!(body["teams"], json!([]), "signup creates no org");
        assert_eq!(body["joined_team_id"], Value::Null);
        assert_eq!(body["joined_team_slug"], Value::Null);
        assert_eq!(body["needs_org"], true);
        let session = cookie.split(';').next().unwrap().to_string();

        // Reusing the link fails.
        let res = app.clone().oneshot(post_json("/api/auth/verify", json!({"token": secret}))).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        let me = |cookie: Option<String>| {
            let mut req = Request::get("/api/me");
            if let Some(c) = cookie {
                req = req.header(header::COOKIE, c);
            }
            req.body(Body::empty()).unwrap()
        };
        let res = app.clone().oneshot(me(Some(session.clone()))).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = body_json(res).await;
        assert_eq!(body["user"]["email"], "flow@example.com");
        assert_eq!(body["teams"], json!([]));

        let res = app.clone().oneshot(me(None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(body_json(res).await["error"], "unauthorized");

        // Cross-origin writes are refused even with a valid cookie.
        let req = Request::post("/api/auth/logout")
            .header(header::COOKIE, &session)
            .header(header::ORIGIN, "https://evil.example")
            .body(Body::empty())
            .unwrap();
        assert_eq!(app.clone().oneshot(req).await.unwrap().status(), StatusCode::FORBIDDEN);

        let req = Request::post("/api/auth/logout")
            .header(header::COOKIE, &session)
            .header(header::ORIGIN, "http://localhost:5191")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(set_cookie(&res).contains("Max-Age=0"));
        let res = app.clone().oneshot(me(Some(session))).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "session deleted on logout");
    }

    /// Who may call which team-admin route, end to end through the router.
    #[tokio::test]
    async fn role_route_matrix() {
        let state = test_state().await;
        let app = crate::app(state.clone());
        let db = &state.db;
        let (owner, _) = accounts::find_or_create_user(db, "owner@example.com").await.unwrap();
        let tid = accounts::create_org(db, &owner.id, "Crew HQ", None).await.unwrap().id;
        let code = accounts::current_invite(db, &tid, &owner.id).await.unwrap();
        let mut ids = vec![];
        for h in ["adam", "mia", "max"] {
            let (u, _) = accounts::find_or_create_user(db, &format!("{h}@example.com")).await.unwrap();
            accounts::join_via_invite(db, &code, &u.id).await.unwrap();
            ids.push(u.id);
        }
        let (adam, mia, max) = (ids[0].clone(), ids[1].clone(), ids[2].clone());
        let cookie = |uid: &str| {
            let (db, uid) = (db.clone(), uid.to_string());
            async move { format!("sb_session={}", accounts::create_session(&db, &uid).await.unwrap()) }
        };
        let (c_owner, c_adam, c_mia) = (cookie(&owner.id).await, cookie(&adam).await, cookie(&mia).await);
        let call = |method: &str, uri: String, cookie: &str, body: Value| {
            Request::builder()
                .method(method)
                .uri(uri)
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        };
        let send = |req: Request<Body>| {
            let app = app.clone();
            async move {
                let res = app.oneshot(req).await.unwrap();
                let status = res.status();
                (status, body_json(res).await)
            }
        };

        // Roles: owner only.
        let (st, b) = send(call("PATCH", format!("/api/teams/{tid}/members/{adam}"), &c_mia, json!({"role": "admin"}))).await;
        assert_eq!((st, b["error"].as_str()), (StatusCode::FORBIDDEN, Some("forbidden")));
        let (st, b) = send(call("PATCH", format!("/api/teams/{tid}/members/{adam}"), &c_owner, json!({"role": "admin"}))).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!((b["user_id"].as_str(), b["role"].as_str(), b["handle"].as_str()), (Some(adam.as_str()), Some("admin"), Some("adam")));
        let (st, _) = send(call("PATCH", format!("/api/teams/{tid}/members/{mia}"), &c_adam, json!({"role": "admin"}))).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "admins can't change roles");
        let (st, b) = send(call("PATCH", format!("/api/teams/{tid}/members/{}", owner.id), &c_owner, json!({"role": "member"}))).await;
        assert_eq!((st, b["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_request")));

        // GET shows roles and the invite to everyone.
        let (st, b) = send(Request::get(format!("/api/teams/{tid}")).header(header::COOKIE, &c_mia).body(Body::empty()).unwrap()).await;
        assert_eq!(st, StatusCode::OK);
        assert!(b["invite"]["url"].as_str().unwrap().contains("/join/"));
        assert!(b["members"].as_array().unwrap().iter().any(|m| m["handle"] == "adam" && m["role"] == "admin"));
        let (_, b) = send(Request::get("/api/me").header(header::COOKIE, &c_adam).body(Body::empty()).unwrap()).await;
        assert!(b["teams"].as_array().unwrap().iter().any(|t| t["id"] == tid.as_str() && t["role"] == "admin"));

        // Rename + rotate: owner or admin.
        for (c, want) in [(&c_mia, StatusCode::FORBIDDEN), (&c_adam, StatusCode::OK), (&c_owner, StatusCode::OK)] {
            assert_eq!(send(call("PATCH", format!("/api/teams/{tid}"), c, json!({"name": "crew"}))).await.0, want);
            assert_eq!(send(call("POST", format!("/api/teams/{tid}/invite"), c, json!({}))).await.0, want);
        }

        // Force release: owner or admin; 404 when nothing is held.
        let (st, _) = send(call("POST", format!("/api/teams/{tid}/claims/release"), &c_mia, json!({"scope": "ci"}))).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
        let (st, b) = send(call("POST", format!("/api/teams/{tid}/claims/release"), &c_adam, json!({"scope": "ci"}))).await;
        assert_eq!((st, b["error"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
        let (_s, bot) = crate::domain::collab::tests::named_agent(db, &tid, &mia, "bot").await;
        crate::domain::collab::leases::claim(db, &bot, None, "ci", None, None, false).await.unwrap();
        let (st, b) = send(call("POST", format!("/api/teams/{tid}/claims/release"), &c_adam, json!({"scope": "ci"}))).await;
        assert_eq!((st, b), (StatusCode::OK, json!({"ok": true, "released": 1})));

        // Removal.
        let (st, _) = send(call("DELETE", format!("/api/teams/{tid}/members/{max}"), &c_mia, json!({}))).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
        let (st, _) = send(call("DELETE", format!("/api/teams/{tid}/members/{}", owner.id), &c_adam, json!({}))).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
        let (st, b) = send(call("DELETE", format!("/api/teams/{tid}/members/{}", owner.id), &c_owner, json!({}))).await;
        assert_eq!((st, b["message"].as_str()), (StatusCode::BAD_REQUEST, Some("transfer ownership first")));
        let (st, b) = send(call("DELETE", format!("/api/teams/{tid}/members/{max}"), &c_adam, json!({}))).await;
        assert_eq!((st, b), (StatusCode::OK, json!({"ok": true})));

        // Transfer: owner only, to a member.
        let (st, _) = send(call("POST", format!("/api/teams/{tid}/transfer"), &c_adam, json!({"user_id": mia}))).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
        let (st, _) = send(call("POST", format!("/api/teams/{tid}/transfer"), &c_owner, json!({"user_id": max}))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "max was removed");
        let (st, b) = send(call("POST", format!("/api/teams/{tid}/transfer"), &c_owner, json!({"user_id": mia}))).await;
        assert_eq!((st, b), (StatusCode::OK, json!({"ok": true})));
        let (_, b) = send(Request::get(format!("/api/teams/{tid}")).header(header::COOKIE, &c_mia).body(Body::empty()).unwrap()).await;
        assert_eq!(b["team"]["owner_id"], mia.as_str());
        assert_eq!(b["members"][0]["handle"], "mia");
        let (st, _) = send(call("DELETE", format!("/api/teams/{tid}/members/{}", owner.id), &c_owner, json!({}))).await;
        assert_eq!(st, StatusCode::OK, "former owner (now admin) can leave");
    }

    #[tokio::test]
    async fn verify_with_invite_joins_team() {
        let state = test_state().await;
        let app = crate::app(state.clone());
        let (owner, _) = accounts::find_or_create_user(&state.db, "owner@example.com").await.unwrap();
        let team = accounts::create_org(&state.db, &owner.id, "Acme", None).await.unwrap().id;
        let code = accounts::current_invite(&state.db, &team, &owner.id).await.unwrap();

        // The invite code is stored with the login token when valid.
        let res = app
            .clone()
            .oneshot(post_json("/api/auth/email", json!({"email": "friend@example.com", "invite": code})))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let stored: Option<String> = sqlx::query_scalar("SELECT invite_code FROM login_tokens WHERE email = 'friend@example.com'")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(stored.as_deref(), Some(code.as_str()));

        let secret = accounts::create_login_token(&state.db, "friend@example.com", Some(&code)).await.unwrap();
        let res = app.clone().oneshot(post_json("/api/auth/verify", json!({"token": secret}))).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = body_json(res).await;
        assert_eq!(body["joined_team_id"], json!(team));
        assert_eq!(body["joined_team_slug"], "acme");
        assert_eq!(body["needs_org"], false);
        assert_eq!(body["teams"].as_array().unwrap().len(), 1);
        assert_eq!(body["teams"][0]["slug"], "acme");

        // The owner signing in again (no invite) already has an org.
        let secret = accounts::create_login_token(&state.db, "owner@example.com", None).await.unwrap();
        let body = body_json(app.clone().oneshot(post_json("/api/auth/verify", json!({"token": secret}))).await.unwrap()).await;
        assert_eq!((body["needs_org"].clone(), body["joined_team_slug"].clone()), (json!(false), Value::Null));
    }

    /// Org routes end to end: create, availability, lookup by slug, invites, slug changes.
    #[tokio::test]
    async fn org_routes() {
        let state = test_state().await;
        let app = crate::app(state.clone());
        let db = &state.db;
        let (paul, _) = accounts::find_or_create_user(db, "paul@example.com").await.unwrap();
        let (zed, _) = accounts::find_or_create_user(db, "zed@example.com").await.unwrap();
        let c_paul = format!("sb_session={}", accounts::create_session(db, &paul.id).await.unwrap());
        let c_zed = format!("sb_session={}", accounts::create_session(db, &zed.id).await.unwrap());
        let send = |method: &str, uri: &str, cookie: &str, body: Option<Value>| {
            let req = Request::builder()
                .method(method)
                .uri(uri)
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty))
                .unwrap();
            let app = app.clone();
            async move {
                let res = app.oneshot(req).await.unwrap();
                let status = res.status();
                (status, body_json(res).await)
            }
        };

        let (st, b) = send("GET", "/api/orgs/slug?slug=Acme%20Inc", &c_paul, None).await;
        assert_eq!((st, b), (StatusCode::OK, json!({"slug": "acme-inc", "available": true})));
        let (st, org) = send("POST", "/api/orgs", &c_paul, Some(json!({"name": "Acme Inc"}))).await;
        assert_eq!(st, StatusCode::OK);
        let tid = org["id"].as_str().unwrap().to_string();
        assert_eq!(org, json!({"id": tid, "name": "Acme Inc", "slug": "acme-inc", "role": "owner", "member_count": 1, "max_members": 5}));
        let (st, b) = send("GET", "/api/orgs/slug?slug=ACME-inc", &c_zed, None).await;
        assert_eq!((st, b), (StatusCode::OK, json!({"slug": "acme-inc", "available": false, "reason": "swarmboard.app/acme-inc is taken"})));
        let (st, b) = send("POST", "/api/orgs", &c_zed, Some(json!({"name": "Mine", "slug": "Acme-Inc"}))).await;
        assert_eq!((st, b), (StatusCode::CONFLICT, json!({"error": "conflict", "message": "swarmboard.app/acme-inc is taken"})));
        let (st, b) = send("POST", "/api/orgs", &c_zed, Some(json!({"name": "Mine", "slug": "settings"}))).await;
        assert_eq!((st, b["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_request")));
        let (st, _) = send("POST", "/api/orgs", "sb_session=nope", Some(json!({"name": "Mine"}))).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);

        // /api/me and /api/teams/:tid carry the slug.
        let (_, me) = send("GET", "/api/me", &c_paul, None).await;
        assert_eq!(me["teams"][0]["slug"], "acme-inc");
        let (_, t) = send("GET", &format!("/api/teams/{tid}"), &c_paul, None).await;
        assert_eq!(t["team"]["slug"], "acme-inc");

        // By slug: members only; others get the same 404 as an unknown slug.
        let (st, b) = send("GET", "/api/orgs/Acme-Inc", &c_paul, None).await;
        assert_eq!((st, b["id"].as_str()), (StatusCode::OK, Some(tid.as_str())));
        let (st, b) = send("GET", "/api/orgs/acme-inc", &c_zed, None).await;
        let (st2, b2) = send("GET", "/api/orgs/no-such-org", &c_zed, None).await;
        assert_eq!((st, &b), (StatusCode::NOT_FOUND, &b2));
        assert_eq!(st2, StatusCode::NOT_FOUND);

        // Invites show and return the slug.
        let code = t["invite"]["code"].as_str().unwrap().to_string();
        let (st, b) = send("GET", &format!("/api/invites/{code}"), &c_zed, None).await;
        assert_eq!((st, b["team_slug"].as_str(), b["team_name"].as_str()), (StatusCode::OK, Some("acme-inc"), Some("Acme Inc")));
        let (st, b) = send("POST", &format!("/api/invites/{code}/accept"), &c_zed, None).await;
        assert_eq!((st, b), (StatusCode::OK, json!({"team_id": tid, "slug": "acme-inc"})));
        let (st, b) = send("GET", "/api/orgs/acme-inc", &c_zed, None).await;
        assert_eq!((st, b["role"].as_str()), (StatusCode::OK, Some("member")));

        // PATCH: slug is owner-only.
        accounts::set_role(db, &tid, &paul.id, &zed.id, "admin").await.unwrap();
        let uri = format!("/api/teams/{tid}");
        let (st, _) = send("PATCH", &uri, &c_zed, Some(json!({"slug": "zed-co"}))).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
        let (st, b) = send("PATCH", &uri, &c_zed, Some(json!({"name": "Acme", "slug": "acme-inc"}))).await;
        assert_eq!((st, b["team"]["name"].as_str()), (StatusCode::OK, Some("Acme")), "admin renames; unchanged slug is fine");
        accounts::create_org(db, &zed.id, "Zed Co", None).await.unwrap();
        let (st, _) = send("PATCH", &uri, &c_paul, Some(json!({"slug": "zed-co"}))).await;
        assert_eq!(st, StatusCode::CONFLICT);
        let (st, b) = send("PATCH", &uri, &c_paul, Some(json!({"slug": "acme"}))).await;
        assert_eq!((st, b["team"]["slug"].as_str()), (StatusCode::OK, Some("acme")));
    }

    #[tokio::test]
    async fn email_rate_limit_per_address() {
        let state = test_state().await;
        let app = crate::app(state);
        let mut statuses = vec![];
        for _ in 0..6 {
            let req = Request::post("/api/auth/email")
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-forwarded-for", "198.51.100.77")
                .body(Body::from(json!({"email": "limited@example.com"}).to_string()))
                .unwrap();
            statuses.push(app.clone().oneshot(req).await.unwrap().status().as_u16());
        }
        assert_eq!(statuses, vec![200, 200, 200, 200, 200, 429]);
    }
}
