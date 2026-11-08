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
    models::{Activity, Actor, Agent, Board, Comment, FileInfo, NewPost, Ping, PingSent, Post, PostDetail, PostFilter, PostPatch},
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
}

async fn activity(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
    QueryParams(q): QueryParams<ActivityQuery>,
) -> AppResult<Json<Vec<Activity>>> {
    actor(&state, &user, &tid).await?;
    Ok(Json(collab::activity(&state.db, &tid, q.after, q.limit).await?))
}
