//! Dashboard REST API under /api (cookie session). OWNER: backend/auth subagent.
//!
//! - `account`: me, teams, members, invites, API tokens (domain::accounts)
//! - `collab`:  boards, posts, claims, comments, agents, pings, inbox, files, activity (domain::collab)
use axum::{routing::any, Router};

use crate::{error::AppError, state::AppState};

mod account;
mod claims;
mod collab;
pub mod extract;

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(account::router())
        .merge(collab::router())
        .merge(claims::router())
        // Unknown /api paths get the JSON error shape instead of an empty 404.
        // Static routes registered elsewhere (e.g. /api/hook) take precedence over this wildcard.
        .route("/api/{*rest}", any(|| async { AppError::NotFound("no such endpoint".into()) }))
}
