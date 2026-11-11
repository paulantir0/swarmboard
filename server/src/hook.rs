//! Claude Code hook endpoint (/api/hook): delivers pings on the agent's next turn. OWNER: MCP subagent.
//!
//! Every hook runs `curl -X POST "<api>/api/hook?event=<Event>&session=<id>[&reason=<r>]" || true`.
//! Only those query parameters are sent; the hook's stdin JSON stays on the machine. Older installs
//! that POST the whole stdin JSON still work (event/session are read from the body as a fallback),
//! and the body is never stored or logged.
//! Responses are the hook JSON output for that event; anything unexpected yields `{}` so a hook
//! never breaks the user's session.

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};

use crate::{
    domain::{accounts, collab},
    error::AppResult,
    mcp::{format, transport::bearer_token},
    models::{Actor, TokenAuth},
    state::AppState,
    util::now_ms,
};

pub fn router() -> Router<AppState> {
    Router::new().route("/api/hook", post(hook))
}

#[derive(Debug, Default, serde::Deserialize)]
struct HookQuery {
    event: Option<String>,
    session: Option<String>,
    reason: Option<String>,
}

async fn hook(State(state): State<AppState>, Query(q): Query<HookQuery>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return (StatusCode::UNAUTHORIZED, Json(json!({}))).into_response();
    };
    let auth = match accounts::authenticate_token(&state.db, token).await {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::UNAUTHORIZED, Json(json!({}))).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "hook auth failed");
            return Json(json!({})).into_response();
        }
    };
    // Hooks fire on every tool use; this only stops a runaway loop on one token.
    if !crate::auth::LIMITER.allow(&format!("hook-token:{}", auth.token_id), 3_000, 60 * 1000, now_ms()) {
        return (StatusCode::TOO_MANY_REQUESTS, Json(json!({}))).into_response();
    }
    // Legacy installs send the whole hook JSON; read only the three fields we need from it.
    let input: Value = if q.event.is_some() || body.is_empty() { Value::Null } else { serde_json::from_slice(&body).unwrap_or(Value::Null) };
    let event = q.event.as_deref().or_else(|| input.get("hook_event_name").and_then(Value::as_str)).unwrap_or("");
    let session_id = q.session.as_deref().or_else(|| input.get("session_id").and_then(Value::as_str)).map(str::trim).unwrap_or("");
    let reason = q.reason.as_deref().filter(|r| !r.is_empty()).or_else(|| input.get("reason").and_then(Value::as_str));
    match respond_with(&state, &auth, event, session_id, reason).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, event, "hook failed");
            Json(json!({})).into_response()
        }
    }
}

/// The hook JSON output for one event. `reason` is SessionEnd's `reason` field ("clear", "logout", "prompt_input_exit", "other").
pub async fn respond_with(
    state: &AppState,
    auth: &TokenAuth,
    event: &str,
    session_id: &str,
    reason: Option<&str>,
) -> AppResult<Value> {
    if session_id.is_empty() || session_id.len() > 200 {
        return Ok(json!({}));
    }
    let db = &state.db;
    let linked = collab::agent_by_session_key(db, &auth.team_id, &auth.user_id, session_id).await?;
    if linked.is_none() {
        tracing::debug!(event, "hook: session key not linked to an agent yet");
    }
    match event {
        "SessionStart" => {
            let mut ctx = format!(
                "swarmboard: your session key is {session_id}. Call the swarmboard hello tool with session=\"{session_id}\" \
                 (plus a short name for yourself) so teammates' pings reach you between turns."
            );
            if let Some(a) = &linked {
                ctx.push_str(&format!(" You were {} in this session: pass name=\"{}\" to keep its claims and inbox.", a.display, a.name));
            }
            Ok(json!({ "hookSpecificOutput": { "hookEventName": "SessionStart", "additionalContext": ctx } }))
        }
        "UserPromptSubmit" | "PostToolUse" | "Stop" => {
            let Some(agent) = linked else {
                return Ok(json!({}));
            };
            collab::touch_agent(db, &agent.id).await?;
            // The agent is working (any tool, not just ours): keep its leases alive.
            if event != "Stop" {
                collab::leases::renew(db, &agent.id).await?;
            }
            let pings = collab::take_agent_pings(db, &agent.id).await?;
            tracing::debug!(event, agent = %agent.display, delivered = pings.len(), "hook");
            if pings.is_empty() {
                return Ok(json!({}));
            }
            let text = format::pings_block(&pings, Some(&agent.display), now_ms());
            if event == "Stop" {
                Ok(json!({
                    "decision": "block",
                    "reason": format!("{text}\nHandle them (answer with the swarmboard send tool) before stopping."),
                }))
            } else {
                Ok(json!({ "hookSpecificOutput": { "hookEventName": event, "additionalContext": text } }))
            }
        }
        // Claude Code exited: free the agent's name now instead of after the activity window.
        // On /clear the process (and its MCP connection) lives on, so leave it alone.
        "SessionEnd" => {
            if let Some(agent) = linked.filter(|_| reason != Some("clear")) {
                collab::close_agent_sessions(db, &agent.id).await?;
                let actor = Actor { team_id: auth.team_id.clone(), user_id: auth.user_id.clone(), agent_id: Some(agent.id.clone()) };
                collab::leases::release_agent(db, &actor).await?;
            }
            Ok(json!({}))
        }
        _ => Ok(json!({})),
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::{
        config::Config,
        domain::collab::tests::{fixture, named_agent},
    };

    #[tokio::test]
    async fn hook_outputs_per_event() {
        let f = fixture().await;
        let state = AppState::new(f.db.clone(), Config::from_env()).unwrap();
        let auth = TokenAuth { token_id: "tok".into(), user_id: f.paul.clone(), team_id: f.team.clone(), handle: "paul".into() };

        let start = respond_with(&state, &auth, "SessionStart", "abc-123", None).await.unwrap();
        assert_eq!(start["hookSpecificOutput"]["hookEventName"], "SessionStart");
        let ctx = start["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(ctx.starts_with("swarmboard: your session key is abc-123. Call the swarmboard hello tool with session=\"abc-123\""));

        // Not linked yet → nothing to say.
        assert_eq!(respond_with(&state, &auth, "UserPromptSubmit", "abc-123", None).await.unwrap(), json!({}));

        let (mut s, me) = named_agent(&f.db, &f.team, &f.paul, "ui-fixer").await;
        collab::hello(&f.db, &mut s, "ui-fixer", None, Some("abc-123")).await.unwrap();
        let bob = Actor { team_id: f.team.clone(), user_id: f.bob.clone(), agent_id: None };
        assert_eq!(respond_with(&state, &auth, "PostToolUse", "abc-123", None).await.unwrap(), json!({}));

        collab::send_ping(&f.db, &bob, "paul/ui-fixer", "check #1", None).await.unwrap();
        let up = respond_with(&state, &auth, "UserPromptSubmit", "abc-123", None).await.unwrap();
        assert_eq!(up["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
        assert!(up["hookSpecificOutput"]["additionalContext"].as_str().unwrap().contains("bob → you"));
        // Delivered once.
        assert_eq!(respond_with(&state, &auth, "PostToolUse", "abc-123", None).await.unwrap(), json!({}));

        collab::send_ping(&f.db, &bob, "ui-fixer", "and #2", None).await.unwrap();
        let ptu = respond_with(&state, &auth, "PostToolUse", "abc-123", None).await.unwrap();
        assert_eq!(ptu["hookSpecificOutput"]["hookEventName"], "PostToolUse");

        collab::send_ping(&f.db, &bob, "paul/*", "stop please", None).await.unwrap();
        let stop = respond_with(&state, &auth, "Stop", "abc-123", None).await.unwrap();
        assert_eq!(stop["decision"], "block");
        assert!(stop["reason"].as_str().unwrap().ends_with("Handle them (answer with the swarmboard send tool) before stopping."));
        assert_eq!(respond_with(&state, &auth, "Stop", "abc-123", None).await.unwrap(), json!({}));

        let again = respond_with(&state, &auth, "SessionStart", "abc-123", None).await.unwrap();
        assert!(again["hookSpecificOutput"]["additionalContext"].as_str().unwrap().contains("You were paul/ui-fixer"));

        // Another human's token never sees paul's session.
        let bob_auth = TokenAuth { token_id: "t2".into(), user_id: f.bob.clone(), team_id: f.team.clone(), handle: "bob".into() };
        collab::send_ping(&f.db, &bob, "paul/ui-fixer", "x", None).await.unwrap();
        assert_eq!(respond_with(&state, &bob_auth, "UserPromptSubmit", "abc-123", None).await.unwrap(), json!({}));
        assert_eq!(respond_with(&state, &auth, "Notification", "abc-123", None).await.unwrap(), json!({}));

        // Hooks renew leases; SessionEnd (exit) releases them.
        collab::leases::claim(&f.db, &me, None, "ci", None, Some(1), false).await.unwrap();
        sqlx::query("UPDATE leases SET expires_at = ?1").bind(now_ms() + 1000).execute(&f.db).await.unwrap();
        respond_with(&state, &auth, "PostToolUse", "abc-123", None).await.unwrap();
        let exp: i64 = sqlx::query_scalar("SELECT expires_at FROM leases").fetch_one(&f.db).await.unwrap();
        assert!(exp > now_ms() + 30_000);

        // SessionEnd: /clear keeps the agent; exit closes its MCP sessions → inactive, name free.
        let active = || async {
            collab::list_agents(&f.db, &f.team, true).await.unwrap().into_iter().any(|a| a.display == "paul/ui-fixer")
        };
        assert_eq!(respond_with(&state, &auth, "SessionEnd", "abc-123", Some("clear")).await.unwrap(), json!({}));
        assert!(active().await);
        assert_eq!(respond_with(&state, &auth, "SessionEnd", "abc-123", Some("prompt_input_exit")).await.unwrap(), json!({}));
        assert!(!active().await);
        assert_eq!(collab::leases::list(&f.db, &f.team).await.unwrap().leases.len(), 0);
        let mut s2 = collab::tests::session(&f.db, &f.team, &f.paul, "claude-code").await;
        let o = collab::hello(&f.db, &mut s2, "ui-fixer", None, None).await.unwrap();
        assert!(o.adopted && Some(o.agent.id) == me.agent_id);
    }
}
