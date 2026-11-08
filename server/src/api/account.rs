//! Me, teams, members, invites, API tokens.

use axum::{
    extract::State,
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::extract::{JsonBody, JsonOrEmpty, PathParam};
use crate::{
    auth::{require_member, require_owner, CurrentUser},
    config::MAX_TEAM_MEMBERS,
    domain::accounts,
    error::{AppError, AppResult},
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/me", get(get_me).patch(patch_me))
        .route("/api/teams/{tid}", get(get_team).patch(patch_team))
        .route("/api/teams/{tid}/invite", post(rotate_invite))
        .route("/api/teams/{tid}/members/{uid}", delete(remove_member))
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
}

async fn patch_team(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
    JsonBody(body): JsonBody<PatchTeam>,
) -> AppResult<Json<Value>> {
    require_owner(&state, &tid, &user.id).await?;
    let team = match body.name {
        Some(name) => accounts::rename_team(&state.db, &tid, &name).await?,
        None => accounts::get_team(&state.db, &tid).await?.ok_or_else(|| AppError::NotFound("team not found".into()))?,
    };
    Ok(Json(json!({ "team": team })))
}

async fn rotate_invite(State(state): State<AppState>, CurrentUser(user): CurrentUser, PathParam(tid): PathParam<String>) -> AppResult<Json<Value>> {
    require_member(&state, &tid, &user.id).await?;
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
    Ok(Json(json!({ "team_id": team_id })))
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
