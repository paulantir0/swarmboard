//! Streamable HTTP transport, hand-rolled: JSON-RPC 2.0 over `POST /mcp`, JSON responses (no SSE).
//! Sessions are rows in `mcp_sessions`, addressed by the `Mcp-Session-Id` header.

use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use super::{tools, INSTRUCTIONS};
use crate::{
    domain::{accounts, collab},
    models::TokenAuth,
    state::AppState,
};

pub const SUPPORTED_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];
pub const LATEST_VERSION: &str = "2025-11-25";
pub const SESSION_HEADER: &str = "mcp-session-id";
const VERSION_HEADER: &str = "mcp-protocol-version";

/// `Authorization: Bearer <token>` (scheme case-insensitive).
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = v.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn http_error(status: StatusCode, code: i64, message: &str) -> Response {
    (status, Json(rpc_error(Value::Null, code, message))).into_response()
}

fn unauthorized() -> Response {
    let mut r = http_error(
        StatusCode::UNAUTHORIZED,
        -32001,
        "Missing or invalid bearer token. Create a token in the swarmboard dashboard (Connect) and send it as \"Authorization: Bearer sb_…\".",
    );
    r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer realm=\"swarmboard\""));
    r
}

#[allow(clippy::result_large_err)]
pub async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<TokenAuth, Response> {
    let Some(token) = bearer_token(headers) else {
        return Err(unauthorized());
    };
    match accounts::authenticate_token(&state.db, token).await {
        Ok(Some(a)) => Ok(a),
        Ok(None) => Err(unauthorized()),
        Err(e) => {
            tracing::error!(error = %e, "token auth failed");
            Err(http_error(StatusCode::INTERNAL_SERVER_ERROR, -32603, "internal error"))
        }
    }
}

/// Look up the session named by the header; 400 if missing, 404 if unknown/closed/not yours.
#[allow(clippy::result_large_err)]
async fn session_for(state: &AppState, auth: &TokenAuth, headers: &HeaderMap) -> Result<collab::McpSession, Response> {
    let Some(sid) = headers.get(SESSION_HEADER).and_then(|v| v.to_str().ok()).map(str::trim).filter(|s| !s.is_empty()) else {
        return Err(http_error(StatusCode::BAD_REQUEST, -32000, "Bad Request: Mcp-Session-Id header required (send initialize first)"));
    };
    match collab::get_mcp_session(&state.db, sid).await {
        Ok(Some(s)) if s.closed_at.is_none() && s.team_id == auth.team_id && s.user_id == auth.user_id => Ok(s),
        Ok(_) => Err(http_error(StatusCode::NOT_FOUND, -32001, "Session not found; initialize a new session")),
        Err(e) => {
            tracing::error!(error = %e, "session lookup failed");
            Err(http_error(StatusCode::INTERNAL_SERVER_ERROR, -32603, "internal error"))
        }
    }
}

/// GET /mcp: no server-initiated SSE stream.
pub async fn get_mcp() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST, DELETE")]).into_response()
}

/// DELETE /mcp: client ends its session.
pub async fn delete_mcp(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let auth = match authenticate(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let session = match session_for(&state, &auth, &headers).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if let Err(e) = collab::close_mcp_session(&state.db, &session.id).await {
        tracing::error!(error = %e, "close session failed");
        return http_error(StatusCode::INTERNAL_SERVER_ERROR, -32603, "internal error");
    }
    StatusCode::OK.into_response()
}

/// POST /mcp: one JSON-RPC message or a batch.
pub async fn post_mcp(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let auth = match authenticate(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return http_error(StatusCode::BAD_REQUEST, -32700, &format!("Parse error: {e}")),
    };
    let (messages, is_batch) = match parsed {
        Value::Array(a) => (a, true),
        v => (vec![v], false),
    };
    if messages.is_empty() {
        return http_error(StatusCode::BAD_REQUEST, -32600, "Invalid Request: empty batch");
    }

    let is_init = |m: &Value| m.get("method").and_then(Value::as_str) == Some("initialize");
    if messages.iter().any(is_init) {
        if messages.len() > 1 {
            return http_error(StatusCode::BAD_REQUEST, -32600, "Invalid Request: initialize must not be batched");
        }
        return initialize(&state, &auth, &messages[0]).await;
    }

    let mut session = match session_for(&state, &auth, &headers).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if let Some(v) = headers.get(VERSION_HEADER).and_then(|v| v.to_str().ok())
        && !SUPPORTED_VERSIONS.contains(&v.trim())
    {
        return http_error(
            StatusCode::BAD_REQUEST,
            -32000,
            &format!("Bad Request: unsupported MCP-Protocol-Version {v} (supported: {})", SUPPORTED_VERSIONS.join(", ")),
        );
    }
    if let Err(e) = collab::touch_mcp_session(&state.db, &session.id).await {
        tracing::warn!(error = %e, "touch session failed");
    }

    let mut responses = Vec::new();
    for m in messages {
        if let Some(r) = handle_message(&state, &auth, &mut session, m).await {
            responses.push(r);
        }
    }
    if responses.is_empty() {
        return StatusCode::ACCEPTED.into_response();
    }
    let body = if is_batch { Value::Array(responses) } else { responses.pop().unwrap_or(Value::Null) };
    (StatusCode::OK, Json(body)).into_response()
}

async fn initialize(state: &AppState, auth: &TokenAuth, msg: &Value) -> Response {
    let Some(id) = msg.get("id").filter(|v| v.is_string() || v.is_number()).cloned() else {
        return http_error(StatusCode::BAD_REQUEST, -32600, "Invalid Request: initialize needs an id");
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(LATEST_VERSION);
    let version = SUPPORTED_VERSIONS.iter().copied().find(|v| *v == requested).unwrap_or(LATEST_VERSION);
    let client_name = params.pointer("/clientInfo/name").and_then(Value::as_str);
    let client_version = params.pointer("/clientInfo/version").and_then(Value::as_str);
    let sid = match collab::create_mcp_session(
        &state.db,
        &auth.team_id,
        &auth.user_id,
        &auth.token_id,
        client_name,
        client_version,
        version,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "create session failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(rpc_error(id, -32603, "internal error"))).into_response();
        }
    };
    tracing::info!(user = %auth.handle, client = ?client_name, %version, "mcp session started");
    let result = json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "swarmboard", "title": "swarmboard", "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    });
    let mut resp = (StatusCode::OK, Json(json!({ "jsonrpc": "2.0", "id": id, "result": result }))).into_response();
    if let Ok(v) = HeaderValue::from_str(&sid) {
        resp.headers_mut().insert(SESSION_HEADER, v);
    }
    resp
}

/// Returns a response for requests; None for notifications and client responses.
async fn handle_message(state: &AppState, auth: &TokenAuth, session: &mut collab::McpSession, m: Value) -> Option<Value> {
    let Value::Object(obj) = m else {
        return Some(rpc_error(Value::Null, -32600, "Invalid Request"));
    };
    let method = obj.get("method").and_then(Value::as_str);
    let id = obj.get("id");
    let (Some(method), Some(id)) = (method, id) else {
        // Notification (no id) or a response to a server request (no method): nothing to send.
        return None;
    };
    if !(id.is_string() || id.is_number()) {
        return Some(rpc_error(Value::Null, -32600, "Invalid Request: id must be a string or number"));
    }
    let id = id.clone();
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(rpc_error(id, -32600, "Invalid Request: jsonrpc must be \"2.0\""));
    }
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    let res: Result<Value, (i64, String)> = match method {
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::definitions() })),
        "tools/call" => {
            let mut ctx = tools::ToolCtx { state, auth, session };
            tools::call(&mut ctx, &params).await.map_err(|e| (e.code, e.message))
        }
        "resources/list" => Ok(json!({ "resources": [] })),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        "logging/setLevel" => Ok(json!({})),
        other => Err((-32601, format!("Method not found: {other}"))),
    };
    Some(match res {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => rpc_error(id, code, &message),
    })
}

#[cfg(test)]
mod tests {

    use axum::{
        body::{to_bytes, Body},
        http::Request,
        Router,
    };
    use tower::ServiceExt;

    use super::*;
    use crate::{config::Config, db::test_pool};

    struct T {
        app: Router,
        token: String,
        dir: std::path::PathBuf,
    }

    impl Drop for T {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    async fn setup() -> T {
        let db = test_pool().await;
        let (user, _) = accounts::find_or_create_user(&db, "paul@example.com").await.unwrap();
        let team = accounts::create_org(&db, &user.id, "Acme", None).await.unwrap();
        let tok = accounts::create_api_token(&db, &user.id, &team.id, Some("test")).await.unwrap();
        let dir = std::env::temp_dir().join(format!("sb-mcp-test-{}", crate::util::random_token(8)));
        let mut cfg = Config::from_env();
        cfg.data_dir = dir.clone();
        cfg.public_api_url = "http://api.test".into();
        cfg.s3 = None;
        let state = AppState::new(db, cfg).unwrap();
        T { app: crate::app(state), token: tok.token, dir }
    }

    impl T {
        async fn send(&self, method: &str, uri: &str, sid: Option<&str>, body: Option<Value>) -> (StatusCode, HeaderMap, Value) {
            let mut b = Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {}", self.token))
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream");
            if let Some(s) = sid {
                b = b.header("mcp-session-id", s);
            }
            let req = b.body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty)).unwrap();
            let resp = self.app.clone().oneshot(req).await.unwrap();
            let status = resp.status();
            let headers = resp.headers().clone();
            let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            (status, headers, v)
        }

        async fn init(&self, client: &str) -> String {
            let (st, h, v) = self
                .send(
                    "POST",
                    "/mcp",
                    None,
                    Some(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                        "protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":client,"version":"1.0"}}})),
                )
                .await;
            assert_eq!(st, StatusCode::OK);
            assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
            let sid = h[SESSION_HEADER].to_str().unwrap().to_string();
            let (st, _, _) = self
                .send("POST", "/mcp", Some(&sid), Some(json!({"jsonrpc":"2.0","method":"notifications/initialized"})))
                .await;
            assert_eq!(st, StatusCode::ACCEPTED);
            sid
        }

        async fn call(&self, sid: &str, tool: &str, args: Value) -> Value {
            let (st, _, v) = self
                .send(
                    "POST",
                    "/mcp",
                    Some(sid),
                    Some(json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":tool,"arguments":args}})),
                )
                .await;
            assert_eq!(st, StatusCode::OK, "{v}");
            v
        }
    }

    fn text(v: &Value, i: usize) -> String {
        v["result"]["content"][i]["text"].as_str().unwrap_or("").to_string()
    }

    #[tokio::test]
    async fn streamable_http_flow() {
        let t = setup().await;

        // Auth.
        let req = Request::builder().method("POST").uri("/mcp").body(Body::from("{}")).unwrap();
        let resp = t.app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert!(resp.headers()[header::WWW_AUTHENTICATE].to_str().unwrap().starts_with("Bearer"));

        // Version negotiation: unsupported → latest.
        let (_, _, v) = t
            .send("POST", "/mcp", None, Some(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}})))
            .await;
        assert_eq!(v["result"]["protocolVersion"], LATEST_VERSION);
        assert_eq!(v["result"]["serverInfo"]["name"], "swarmboard");
        assert!(v["result"]["instructions"].as_str().unwrap().contains("hello"));
        assert_eq!(v["result"]["capabilities"]["tools"]["listChanged"], false);

        let s1 = t.init("claude-code").await;
        // Session header rules.
        let list = json!({"jsonrpc":"2.0","id":2,"method":"tools/list"});
        assert_eq!(t.send("POST", "/mcp", None, Some(list.clone())).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(t.send("POST", "/mcp", Some("mcp_nope"), Some(list.clone())).await.0, StatusCode::NOT_FOUND);
        assert_eq!(t.send("POST", "/mcp", None, Some(json!("garbage{"))).await.0, StatusCode::BAD_REQUEST);

        let (st, _, v) = t.send("POST", "/mcp", Some(&s1), Some(list)).await;
        assert_eq!(st, StatusCode::OK);
        let tools = v["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), tools::TOOL_NAMES.len());
        for tl in tools {
            assert_eq!(tl["inputSchema"]["additionalProperties"], false, "{}", tl["name"]);
            assert_eq!(tl["inputSchema"]["type"], "object");
            for (_, p) in tl["inputSchema"]["properties"].as_object().unwrap() {
                assert!(p["description"].is_string(), "{}", tl["name"]);
            }
        }

        // ping method + batch handling.
        let (_, _, v) = t
            .send(
                "POST",
                "/mcp",
                Some(&s1),
                Some(json!([{"jsonrpc":"2.0","id":"a","method":"ping"},{"jsonrpc":"2.0","method":"notifications/cancelled"}])),
            )
            .await;
        assert_eq!(v, json!([{"jsonrpc":"2.0","id":"a","result":{}}]));

        // First call without hello → provisional agent + hint.
        let v = t.call(&s1, "topics", json!({})).await;
        assert!(text(&v, 0).contains("No topics yet"), "{}", text(&v, 0));
        assert!(text(&v, 0).contains("(You are \"paul/claude-"), "{}", text(&v, 0));

        let v = t.call(&s1, "hello", json!({"name":"probe","working_on":"testing","repo":"git@github.com:Acme/App.git","branch":"main"})).await;
        let hello = text(&v, 0);
        assert!(hello.starts_with("You are paul/probe (paul's agent) on org \"Acme\" (acme) · github.com/acme/app@main."), "{hello}");
        assert!(hello.contains("Protocol:"), "{hello}");
        let v = t.call(&s1, "post", json!({"topic":"Auth","title":"Fix login redirect","kind":"task"})).await;
        assert!(text(&v, 0).starts_with("Posted #1 [open] task  Fix login redirect — paul/probe"), "{}", text(&v, 0));
        let v = t.call(&s1, "claim", json!({"scope":"#1"})).await;
        assert_eq!(v["result"]["isError"], false, "{}", text(&v, 0));
        let v = t.call(&s1, "claim", json!({"scope":"server/src/auth/**","note":"refactor"})).await;
        assert!(text(&v, 0).starts_with("Claimed server/src/auth/** in github.com/acme/app for 30m"), "{}", text(&v, 0));

        // Second session (same human): suffix on name clash, lease conflicts, queue.
        let s2 = t.init("codex-mcp-client").await;
        let v = t.call(&s2, "hello", json!({"name":"probe","repo":"https://github.com/acme/app"})).await;
        let brief = text(&v, 0);
        assert!(brief.contains("paul/probe-2"), "{brief}");
        assert!(brief.contains("server/src/auth/** [github.com/acme/app] — paul/probe \"refactor\""), "{brief}");
        assert!(brief.contains("holds #1, server/src/auth/**"), "{brief}");
        let v = t.call(&s2, "check", json!({"paths":["server/src/auth/login.rs","web/x.ts"]})).await;
        let c = text(&v, 0);
        assert!(c.contains("server/src/auth/login.rs: TAKEN by paul/probe (server/src/auth/** \"refactor\""), "{c}");
        assert!(c.contains("web/x.ts: free"), "{c}");
        let v = t.call(&s2, "claim", json!({"scope":"server/src/auth/login.rs"})).await;
        assert_eq!(v["result"]["isError"], true);
        assert!(text(&v, 0).starts_with("TAKEN: server/src/auth/login.rs overlaps:"), "{}", text(&v, 0));
        let v = t.call(&s2, "claim", json!({"scope":"#1"})).await;
        assert_eq!(v["result"]["isError"], true);
        assert_eq!(t.call(&s1, "claim", json!({"scope":"ci"})).await["result"]["isError"], false);
        let v = t.call(&s2, "claim", json!({"scope":"ci","wait":true})).await;
        assert!(text(&v, 0).starts_with("Queued for ci at position 1."), "{}", text(&v, 0));

        // Send s1 a message from s2; s1's next tool result carries it.
        let v = t.call(&s2, "send", json!({"to":"paul/probe","message":"want me to take the tests?","re":1})).await;
        assert_eq!(text(&v, 0), "Sent to paul/probe. They see it on their next turn.");
        let v = t.call(&s1, "feed", json!({"status":"active"})).await;
        assert!(text(&v, 0).contains("#1 [in_progress] task  Fix login redirect — paul/probe (claimed by paul/probe)"), "{}", text(&v, 0));
        let piggy = text(&v, 1);
        assert!(piggy.contains("1 new ping") && piggy.contains("paul/probe-2 → you (#1"), "{piggy}");

        // Release ci → s2 granted + pinged; release #1 as done with a note.
        let v = t.call(&s1, "release", json!({"scope":"ci","note":"tests green"})).await;
        assert_eq!(text(&v, 0), "Released ci (\"tests green\"); granted to paul/probe-2.");
        let v = t.call(&s2, "claims", json!({})).await;
        assert!(text(&v, 0).contains("ci — paul/probe-2 (you)"), "{}", text(&v, 0));
        assert!(text(&v, 1).contains("you now hold ci (you were queued). paul/probe released it: \"tests green\"."), "{}", text(&v, 1));
        let v = t.call(&s1, "release", json!({"scope":"#1","status":"done","note":"fixed in abc"})).await;
        assert!(text(&v, 0).contains("#1 [done]"), "{}", text(&v, 0));
        let v = t.call(&s2, "reply", json!({"number":1,"body":"thanks"})).await;
        assert_eq!(text(&v, 0), "Replied on #1. Notified paul/probe.");
        let v = t.call(&s1, "read", json!({"number":1})).await;
        assert!(text(&v, 0).contains("fixed in abc") && text(&v, 1).contains("replied on #1"), "{v}");

        // Errors: bad args are tool errors; unknown tools are protocol errors.
        let v = t.call(&s1, "read", json!({"number":1,"bogus":true})).await;
        assert_eq!(v["result"]["isError"], true);
        assert!(text(&v, 0).contains("unknown argument \"bogus\""));
        let v = t.call(&s1, "nope", json!({})).await;
        assert_eq!(v["error"]["code"], -32602);
        let (_, _, v) = t.send("POST", "/mcp", Some(&s1), Some(json!({"jsonrpc":"2.0","id":3,"method":"bogus/method"}))).await;
        assert_eq!(v["error"]["code"], -32601);

        // Upload via one-time link.
        let v = t.call(&s1, "upload", json!({"filename":"shot one.png"})).await;
        let cmd = text(&v, 0);
        let url = cmd.split('\'').find(|s| s.starts_with("http://api.test/u/")).unwrap().to_string();
        let path = url.trim_start_matches("http://api.test").to_string();
        assert!(path.ends_with("?filename=shot-one.png"));
        let put = |p: String| {
            let app = t.app.clone();
            async move {
                let req = Request::builder().method("PUT").uri(p).body(Body::from(vec![0x89, b'P', b'N', b'G'])).unwrap();
                let resp = app.oneshot(req).await.unwrap();
                let st = resp.status();
                (st, serde_json::from_slice::<Value>(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap_or(Value::Null))
            }
        };
        let (st, info) = put(path.clone()).await;
        assert_eq!(st, StatusCode::OK, "{info}");
        assert_eq!(info["content_type"], "image/png");
        assert_eq!(put(path).await.0, StatusCode::NOT_FOUND, "one-time");
        let f = info["url"].as_str().unwrap().trim_start_matches("http://api.test").to_string();
        let resp = t.app.clone().oneshot(Request::builder().uri(f).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(resp.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");

        // base64 upload.
        let v = t.call(&s1, "upload", json!({"filename":"note.txt","base64":"aGVsbG8="})).await;
        assert!(text(&v, 0).starts_with("Uploaded note.txt (5 B, text/plain; charset=utf-8)"), "{}", text(&v, 0));

        // GET → 405, DELETE closes.
        assert_eq!(t.send("GET", "/mcp", Some(&s1), None).await.0, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(t.send("DELETE", "/mcp", Some(&s1), None).await.0, StatusCode::OK);
        let (st, _, _) = t.send("POST", "/mcp", Some(&s1), Some(json!({"jsonrpc":"2.0","id":9,"method":"ping"}))).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }
}
