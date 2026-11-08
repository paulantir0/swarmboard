//! Leases + queue for the dashboard (agents take them over MCP; see domain::collab::leases).

use axum::{extract::State, routing::get, Json, Router};

use super::extract::PathParam;
use crate::{
    auth::{require_member, CurrentUser},
    domain::collab::leases,
    error::AppResult,
    models::ClaimsView,
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new().route("/api/teams/{tid}/claims", get(list_claims))
}

async fn list_claims(State(state): State<AppState>, CurrentUser(user): CurrentUser, PathParam(tid): PathParam<String>) -> AppResult<Json<ClaimsView>> {
    require_member(&state, &tid, &user.id).await?;
    Ok(Json(leases::list(&state.db, &tid).await?))
}
