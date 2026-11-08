//! Leases + queue for the dashboard (agents take them over MCP; see domain::collab::leases).

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::extract::{JsonBody, PathParam};
use crate::{
    auth::{require_admin, require_member, CurrentUser},
    domain::collab::leases,
    error::AppResult,
    models::{Actor, ClaimsView},
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/teams/{tid}/claims", get(list_claims))
        .route("/api/teams/{tid}/claims/release", post(force_release))
}

async fn list_claims(State(state): State<AppState>, CurrentUser(user): CurrentUser, PathParam(tid): PathParam<String>) -> AppResult<Json<ClaimsView>> {
    require_member(&state, &tid, &user.id).await?;
    Ok(Json(leases::list(&state.db, &tid).await?))
}

#[derive(Deserialize)]
struct ForceRelease {
    scope: String,
    /// Path leases are per repo; omit to release the scope in every repo.
    repo: Option<String>,
}

/// Owner/admin: take a lease away from whichever agent holds it; the next waiter gets it.
async fn force_release(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    PathParam(tid): PathParam<String>,
    JsonBody(body): JsonBody<ForceRelease>,
) -> AppResult<Json<Value>> {
    require_admin(&state, &tid, &user.id).await?;
    let actor = Actor { team_id: tid, user_id: user.id.clone(), agent_id: None };
    let released = leases::force_release(&state.db, &actor, &user.handle, &body.scope, body.repo.as_deref()).await?;
    Ok(Json(json!({ "ok": true, "released": released })))
}
