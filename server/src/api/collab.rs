//! Boards, posts, claims, comments, agents, pings, inbox, files, activity.
//! Thin handlers: membership check, build the human `Actor`, call domain::collab.

use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::extract::{flag, non_empty, JsonBody, JsonOrEmpty, PathParam, QueryParams};
use crate::{
    auth::{require_member, CurrentUser},
    domain::{accounts::User, collab},
    error::{AppError, AppResult},
    models::{Activity, Actor, Agent, AgentProfile, Board, Comment, FileInfo, MemberProfile, NewPost, Ping, PingSent, Post, PostDetail, PostFilter, PostPatch},
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/teams/{tid}/boards", get(list_boards).post(create_board))
        .route("/api/teams/{tid}/posts", get(list_posts).post(create_post))
        .route("/api/teams/{tid}/posts/{n}", get(get_post).patch(update_post))
        .route("/api/teams/{tid}/posts/{n}/claim", post(claim_post))
        .route("/api/teams/{tid}/posts/{n}/release", post(release_post))
        .route("/api/teams/{tid}/posts/{n}/comments", post(add_comment))
        .route("/api/teams/{tid}/agents", get(list_agents))
        .route("/api/teams/{tid}/agents/{agent_id}", get(get_agent))
        .route("/api/teams/{tid}/members/{uid}", get(get_member))
        .route("/api/teams/{tid}/pings", post(send_ping))
        .route("/api/teams/{tid}/inbox", get(inbox))
        .route("/api/teams/{tid}/inbox/read", post(inbox_read))
        .route("/api/teams/{tid}/files", post(upload_file))
        .route("/api/teams/{tid}/activity", get(activity))
}

/// 403 unless the user is in the team; otherwise the human actor for that team.
async fn actor(state: &AppState, user: &User, team_id: &str) -> AppResult<Actor> {
    require_member(state, team_id, &user.id).await?;
    Ok(Actor { team_id: team_id.to_string(), user_id: user.id.clone(), agent_id: None })
}

// --- boards -----------------------------------------------------------------------------------

async fn list_boards(State(state): State<AppState>, CurrentUser(user): CurrentUser, PathParam(tid): PathParam<String>) -> AppResult<Json<Vec<Board>>> {
    actor(&state, &user, &tid).await?;
    Ok(Json(collab::list_boards(&state.db, &tid).await?))
}

#[derive(Deserialize)]
struct NewBoard {
    key: String,
    name: Option<String>,
    description: Option<String>,
}

async fn create_board(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
    JsonBody(body): JsonBody<NewBoard>,
) -> AppResult<Json<Board>> {
    let actor = actor(&state, &user, &tid).await?;
    let key = body.key.trim();
    let name = non_empty(body.name).unwrap_or_else(|| key.to_string());
    let description = body.description.unwrap_or_default();
    Ok(Json(collab::create_board(&state.db, &actor, key, &name, description.trim()).await?))
}

// --- posts ------------------------------------------------------------------------------------

async fn list_posts(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
    QueryParams(f): QueryParams<PostFilter>,
) -> AppResult<Json<Vec<Post>>> {
    actor(&state, &user, &tid).await?;
    let filter = PostFilter {
        board: non_empty(f.board),
        status: non_empty(f.status),
        kind: non_empty(f.kind),
        claimed_by: non_empty(f.claimed_by),
        author: non_empty(f.author),
        author_user: non_empty(f.author_user),
        q: non_empty(f.q),
        limit: f.limit,
        before: f.before,
    };
    Ok(Json(collab::list_posts(&state.db, &tid, &filter).await?))
}

async fn create_post(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
    JsonBody(new): JsonBody<NewPost>,
) -> AppResult<Json<Post>> {
    let actor = actor(&state, &user, &tid).await?;
    Ok(Json(collab::create_post(&state.db, &actor, new).await?))
}

async fn get_post(State(state): State<AppState>, CurrentUser(user): CurrentUser, PathParam((tid, n)): PathParam<(String, i64)>) -> AppResult<Json<PostDetail>> {
    actor(&state, &user, &tid).await?;
    Ok(Json(collab::get_post(&state.db, &tid, n).await?))
}

async fn update_post(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam((tid, n)): PathParam<(String, i64)>,
    JsonBody(patch): JsonBody<PostPatch>,
) -> AppResult<Json<Post>> {
    let actor = actor(&state, &user, &tid).await?;
    Ok(Json(collab::update_post(&state.db, &actor, n, patch).await?))
}

#[derive(Deserialize, Default)]
struct ClaimBody {
    #[serde(default)]
    force: Option<bool>,
}

async fn claim_post(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam((tid, n)): PathParam<(String, i64)>,
    JsonOrEmpty(body): JsonOrEmpty<ClaimBody>,
) -> AppResult<Json<Post>> {
    let actor = actor(&state, &user, &tid).await?;
    Ok(Json(collab::claim_post(&state.db, &actor, n, body.force.unwrap_or(false)).await?))
}

#[derive(Deserialize, Default)]
struct ReleaseBody {
    #[serde(default)]
    status: Option<String>,
}

async fn release_post(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam((tid, n)): PathParam<(String, i64)>,
    JsonOrEmpty(body): JsonOrEmpty<ReleaseBody>,
) -> AppResult<Json<Post>> {
    let actor = actor(&state, &user, &tid).await?;
    let status = non_empty(body.status);
    Ok(Json(collab::release_post(&state.db, &actor, n, status.as_deref()).await?))
}

#[derive(Deserialize)]
struct CommentBody {
    body: String,
}

async fn add_comment(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam((tid, n)): PathParam<(String, i64)>,
    JsonBody(body): JsonBody<CommentBody>,
) -> AppResult<Json<Comment>> {
    let actor = actor(&state, &user, &tid).await?;
    if body.body.trim().is_empty() {
        return Err(AppError::BadRequest("comment is empty".into()));
    }
    Ok(Json(collab::add_comment(&state.db, &actor, n, &body.body).await?))
}

// --- agents, pings, inbox ---------------------------------------------------------------------

#[derive(Deserialize)]
struct AgentsQuery {
    active: Option<String>,
}

async fn list_agents(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
    QueryParams(q): QueryParams<AgentsQuery>,
) -> AppResult<Json<Vec<Agent>>> {
    actor(&state, &user, &tid).await?;
    Ok(Json(collab::list_agents(&state.db, &tid, flag(&q.active)).await?))
}

async fn get_agent(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam((tid, agent_id)): PathParam<(String, String)>,
) -> AppResult<Json<AgentProfile>> {
    actor(&state, &user, &tid).await?;
    let profile = collab::agent_profile(&state.db, &tid, &agent_id).await?;
    Ok(Json(profile.ok_or_else(|| AppError::NotFound("no such agent in this team".into()))?))
}

async fn get_member(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam((tid, user_id)): PathParam<(String, String)>,
) -> AppResult<Json<MemberProfile>> {
    actor(&state, &user, &tid).await?;
    let profile = collab::member_profile(&state.db, &tid, &user_id).await?;
    Ok(Json(profile.ok_or_else(|| AppError::NotFound("no such member in this team".into()))?))
}

#[derive(Deserialize)]
struct PingBody {
    to: String,
    body: String,
    post_number: Option<i64>,
}

async fn send_ping(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
    JsonBody(p): JsonBody<PingBody>,
) -> AppResult<Json<PingSent>> {
    let actor = actor(&state, &user, &tid).await?;
    if p.to.trim().is_empty() || p.body.trim().is_empty() {
        return Err(AppError::BadRequest("`to` and `body` are required".into()));
    }
    Ok(Json(collab::send_ping(&state.db, &actor, p.to.trim(), &p.body, p.post_number).await?))
}

#[derive(Deserialize)]
struct InboxQuery {
    all: Option<String>,
}

async fn inbox(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
    QueryParams(q): QueryParams<InboxQuery>,
) -> AppResult<Json<Vec<Ping>>> {
    actor(&state, &user, &tid).await?;
    Ok(Json(collab::user_inbox(&state.db, &tid, &user.id, flag(&q.all)).await?))
}

async fn inbox_read(State(state): State<AppState>, CurrentUser(user): CurrentUser, PathParam(tid): PathParam<String>) -> AppResult<Json<Value>> {
    actor(&state, &user, &tid).await?;
    collab::mark_user_inbox_read(&state.db, &tid, &user.id).await?;
    Ok(Json(json!({ "ok": true })))
}

// --- files, activity --------------------------------------------------------------------------

#[derive(Deserialize)]
struct FileQuery {
    filename: Option<String>,
}

/// Raw request body; name from `?filename=`, type from `Content-Type`.
async fn upload_file(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
    QueryParams(q): QueryParams<FileQuery>,
    headers: HeaderMap,
    bytes: Bytes,
) -> AppResult<Json<FileInfo>> {
    let actor = actor(&state, &user, &tid).await?;
    if bytes.is_empty() {
        return Err(AppError::BadRequest("empty file".into()));
    }
    let filename = non_empty(q.filename).unwrap_or_else(|| "file".to_string());
    let content_type = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::trim).filter(|v| !v.is_empty());
    Ok(Json(collab::save_file(&state, &actor, &filename, content_type, &bytes).await?))
}

#[derive(Deserialize)]
struct ActivityQuery {
    after: Option<i64>,
    limit: Option<i64>,
    /// Agent id: only that agent's events.
    agent: Option<String>,
    /// User id: events by that human or any of their agents.
    user: Option<String>,
}

async fn activity(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
    QueryParams(q): QueryParams<ActivityQuery>,
) -> AppResult<Json<Vec<Activity>>> {
    actor(&state, &user, &tid).await?;
    let f = collab::ActivityFilter { after: q.after, limit: q.limit, agent_id: non_empty(q.agent), user_id: non_empty(q.user) };
    Ok(Json(collab::activity_filtered(&state.db, &tid, &f).await?))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
    };
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::{
        config::Config,
        domain::{
            accounts,
            collab::{self, leases, tests::{add_user, fixture, named_agent}},
        },
        models::{Actor, NewPost},
        state::AppState,
        util::new_id,
    };

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

    fn np(title: &str) -> NewPost {
        NewPost { board: "general".into(), title: title.into(), body: String::new(), kind: None, status: None, priority: None }
    }

    struct World {
        app: axum::Router,
        team: String,
        other_team: String,
        paul: String,
        bob: String,
        zed: String,
        docs: String,
        idle: String,
        zed_agent: String,
        cookie_paul: String,
        cookie_zed: String,
    }

    impl World {
        async fn get(&self, uri: &str, cookie: &str) -> (StatusCode, Value) {
            let req = Request::get(uri).header(header::COOKIE, cookie).body(Body::empty()).unwrap();
            let res = self.app.clone().oneshot(req).await.unwrap();
            let status = res.status();
            let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
            (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
        }
        async fn ok(&self, uri: &str) -> Value {
            let (st, v) = self.get(uri, &self.cookie_paul).await;
            assert_eq!(st, StatusCode::OK, "{uri}: {v}");
            v
        }
    }

    /// Team 1: paul (owner, agents docs-agent [active] + idle [session closed]) and bob (agent api).
    /// Team 2: paul + zed (agent zed-bot), which team 1 must never expose.
    async fn world() -> World {
        let f = fixture().await;
        let db = &f.db;
        let (_s, docs) = named_agent(db, &f.team, &f.paul, "docs-agent").await;
        let (_s, api) = named_agent(db, &f.team, &f.bob, "api").await;
        let (idle_sess, idle) = named_agent(db, &f.team, &f.paul, "idle").await;
        collab::close_mcp_session(db, &idle_sess.id).await.unwrap();
        let human = Actor { team_id: f.team.clone(), user_id: f.paul.clone(), agent_id: None };

        collab::create_post(db, &human, np("human post")).await.unwrap(); // #1
        collab::create_post(db, &docs, np("agent post")).await.unwrap(); // #2
        collab::create_post(db, &api, np("bob post")).await.unwrap(); // #3
        collab::add_comment(db, &docs, 3, "looks good").await.unwrap();
        collab::send_ping(db, &docs, "bob/api", "hi", None).await.unwrap();
        collab::send_ping(db, &api, "paul/docs-agent", "hello back", None).await.unwrap();
        leases::claim(db, &docs, Some("github.com/acme/app"), "ci", Some("e2e"), None, false).await.unwrap();
        leases::claim(db, &api, None, "deploy", None, None, false).await.unwrap();
        leases::claim(db, &docs, None, "deploy", None, None, true).await.unwrap();

        let other_team = new_id("tm");
        sqlx::query("INSERT INTO teams (id, name, slug, owner_id, created_at) VALUES (?1, 'other', 'other', ?2, 0)")
            .bind(other_team.as_str())
            .bind(f.paul.as_str())
            .execute(db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO memberships (team_id, user_id, role, joined_at) VALUES (?1, ?2, 'owner', 0)")
            .bind(other_team.as_str())
            .bind(f.paul.as_str())
            .execute(db)
            .await
            .unwrap();
        let zed = add_user(db, &other_team, "zed").await;
        let (_s, zed_agent) = named_agent(db, &other_team, &zed, "zed-bot").await;

        let cookie_paul = format!("sb_session={}", accounts::create_session(db, &f.paul).await.unwrap());
        let cookie_zed = format!("sb_session={}", accounts::create_session(db, &zed).await.unwrap());
        let app = crate::app(AppState::new(f.db.clone(), test_cfg()).unwrap());
        World {
            app,
            team: f.team,
            other_team,
            paul: f.paul,
            bob: f.bob,
            zed,
            docs: docs.agent_id.unwrap(),
            idle: idle.agent_id.unwrap(),
            zed_agent: zed_agent.agent_id.unwrap(),
            cookie_paul,
            cookie_zed,
        }
    }

    fn strs(v: &Value, key: &str) -> Vec<String> {
        v.as_array().unwrap().iter().map(|x| x[key].as_str().unwrap_or_default().to_string()).collect()
    }

    #[tokio::test]
    async fn agent_profile_route() {
        let w = world().await;
        let t = &w.team;
        let v = w.ok(&format!("/api/teams/{t}/agents/{}", w.docs)).await;
        assert_eq!(v["agent"]["id"], w.docs.as_str());
        assert_eq!(v["agent"]["display"], "paul/docs-agent");
        assert_eq!(v["agent"]["active"], true);
        assert_eq!(strs(&v["leases"], "scope"), vec!["ci"]);
        assert_eq!(v["leases"][0]["note"], "e2e");
        assert!(v["leases"][0]["expires_at"].as_i64().unwrap() > 0);
        assert_eq!(strs(&v["queue"], "scope"), vec!["deploy"]);
        assert_eq!(v["queue"][0]["position"], 1);
        assert_eq!(
            v["stats"],
            serde_json::json!({"posts": 1, "replies": 1, "messages_sent": 1, "messages_received": 1, "sessions": 1})
        );
        let recent = v["recent"].as_array().unwrap();
        assert!(!recent.is_empty());
        assert!(recent.iter().all(|e| e["actor"]["agent_id"] == w.docs.as_str()), "{recent:?}");
        let ids: Vec<i64> = recent.iter().map(|e| e["id"].as_i64().unwrap()).collect();
        assert!(ids.windows(2).all(|p| p[0] > p[1]), "newest first: {ids:?}");

        // Agent from another team (even one paul can see there) is a 404 here; unknown ids too.
        let (st, b) = w.get(&format!("/api/teams/{t}/agents/{}", w.zed_agent), &w.cookie_paul).await;
        assert_eq!((st, b["error"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
        assert_eq!(w.get(&format!("/api/teams/{t}/agents/agt_nope"), &w.cookie_paul).await.0, StatusCode::NOT_FOUND);
        w.ok(&format!("/api/teams/{}/agents/{}", w.other_team, w.zed_agent)).await;
        // Non-members are refused before any lookup.
        assert_eq!(w.get(&format!("/api/teams/{t}/agents/{}", w.docs), &w.cookie_zed).await.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn member_profile_route() {
        let w = world().await;
        let t = &w.team;
        let v = w.ok(&format!("/api/teams/{t}/members/{}", w.paul)).await;
        assert_eq!(v["member"]["user_id"], w.paul.as_str());
        assert_eq!(v["member"]["handle"], "paul");
        assert_eq!(v["member"]["email"], "paul@example.com");
        assert_eq!(v["member"]["role"], "owner");
        assert!(v["member"]["joined_at"].is_i64());
        // Active first even though `idle` was seen more recently.
        assert_eq!(strs(&v["agents"], "id"), vec![w.docs.clone(), w.idle.clone()]);
        let recent = v["recent"].as_array().unwrap();
        assert!(recent.iter().all(|e| e["actor"]["user_id"] == w.paul.as_str()));
        assert!(recent.iter().any(|e| e["actor"]["agent_id"].is_null()), "includes the human's own events");
        assert!(recent.iter().any(|e| e["actor"]["agent_id"] == w.docs.as_str()), "and their agents'");

        let v = w.ok(&format!("/api/teams/{t}/members/{}", w.bob)).await;
        assert_eq!(v["member"]["role"], "member");
        assert_eq!(strs(&v["agents"], "display"), vec!["bob/api"]);

        // zed is only in the other team.
        let (st, _) = w.get(&format!("/api/teams/{t}/members/{}", w.zed), &w.cookie_paul).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert_eq!(w.get(&format!("/api/teams/{t}/members/{}", w.paul), &w.cookie_zed).await.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn activity_filters() {
        let w = world().await;
        let t = &w.team;
        let all = w.ok(&format!("/api/teams/{t}/activity")).await;
        let by_agent = w.ok(&format!("/api/teams/{t}/activity?agent={}", w.docs)).await;
        let by_agent = by_agent.as_array().unwrap();
        assert!(!by_agent.is_empty() && by_agent.len() < all.as_array().unwrap().len());
        assert!(by_agent.iter().all(|e| e["actor"]["agent_id"] == w.docs.as_str()));

        let by_user = w.ok(&format!("/api/teams/{t}/activity?user={}", w.paul)).await;
        let by_user = by_user.as_array().unwrap();
        assert!(by_user.iter().all(|e| e["actor"]["user_id"] == w.paul.as_str()));
        assert!(by_user.iter().any(|e| e["actor"]["agent_id"].is_null()));
        assert!(by_user.iter().any(|e| e["actor"]["agent_id"] == w.docs.as_str()));
        assert!(by_user.len() > by_agent.len());

        // after/limit still apply (oldest first), combined with a filter.
        let first = by_agent[0]["id"].as_i64().unwrap();
        let after = w.ok(&format!("/api/teams/{t}/activity?agent={}&after={first}&limit=1", w.docs)).await;
        assert_eq!(after.as_array().unwrap().len(), 1);
        assert_eq!(after[0]["id"], by_agent[1]["id"]);
        let bob = w.ok(&format!("/api/teams/{t}/activity?user={}&limit=500", w.bob)).await;
        assert!(bob.as_array().unwrap().iter().all(|e| e["actor"]["user_id"] == w.bob.as_str()));
        // A foreign agent id just matches nothing.
        assert_eq!(w.ok(&format!("/api/teams/{t}/activity?agent={}", w.zed_agent)).await, serde_json::json!([]));
    }

    #[tokio::test]
    async fn post_author_filters() {
        let w = world().await;
        let t = &w.team;
        let titles = |v: Value| strs(&v, "title");
        assert_eq!(titles(w.ok(&format!("/api/teams/{t}/posts?author=paul")).await), vec!["human post"]);
        assert_eq!(titles(w.ok(&format!("/api/teams/{t}/posts?author=@paul")).await), vec!["human post"]);
        assert_eq!(titles(w.ok(&format!("/api/teams/{t}/posts?author=paul/docs-agent")).await), vec!["agent post"]);
        assert_eq!(titles(w.ok(&format!("/api/teams/{t}/posts?author=paul/*")).await), vec!["agent post", "human post"]);
        // A bare agent name still works when it isn't someone's handle.
        assert_eq!(titles(w.ok(&format!("/api/teams/{t}/posts?author=docs-agent")).await), vec!["agent post"]);
        assert_eq!(titles(w.ok(&format!("/api/teams/{t}/posts?author_user={}", w.paul)).await), vec!["agent post", "human post"]);
        assert_eq!(titles(w.ok(&format!("/api/teams/{t}/posts?author_user={}", w.bob)).await), vec!["bob post"]);
        assert_eq!(titles(w.ok(&format!("/api/teams/{t}/posts?author_user={}", w.zed)).await), Vec::<String>::new());
        let both = w.ok(&format!("/api/teams/{t}/posts?author_user={}&author=paul", w.paul)).await;
        assert_eq!(titles(both), vec!["human post"]);
    }
}
