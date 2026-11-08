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
    let (user, _created) = accounts::find_or_create_user(&state.db, &claim.email).await?;

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
    let jar = jar.add(session_cookie(&state.cfg, &secret));
    Ok((
        jar,
        Json(json!({ "user": user, "teams": teams, "joined_team_id": joined_team_id, "join_error": join_error })),
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

/// Like `require_member`, but only the owner passes.
pub async fn require_owner(state: &AppState, team_id: &str, user_id: &str) -> AppResult<()> {
    match require_member(state, team_id, user_id).await? {
        Role::Owner => Ok(()),
        Role::Member => Err(AppError::Forbidden("only the team owner can do that".into())),
    }
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
    let first_hop = header("x-forwarded-for").and_then(|v| v.split(',').next()).map(str::trim).filter(|s| !s.is_empty());
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
    use std::sync::Arc;

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
        }
    }

    async fn test_state() -> AppState {
        AppState { db: test_pool().await, cfg: Arc::new(test_cfg()), http: reqwest::Client::new() }
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
        assert_eq!(body["teams"][0]["role"], "owner");
        assert_eq!(body["joined_team_id"], Value::Null);
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
        assert_eq!(body["teams"][0]["max_members"], 5);

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

    #[tokio::test]
    async fn verify_with_invite_joins_team() {
        let state = test_state().await;
        let app = crate::app(state.clone());
        let (owner, _) = accounts::find_or_create_user(&state.db, "owner@example.com").await.unwrap();
        let team = accounts::list_user_teams(&state.db, &owner.id).await.unwrap()[0].id.clone();
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
        assert_eq!(body["teams"].as_array().unwrap().len(), 2);
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
