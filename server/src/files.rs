//! Public file serving (/f/<id>/<name>) and one-time uploads (PUT /u/<code>). OWNER: MCP subagent.
//! Also the SSRF-guarded remote fetch used by the MCP `upload(url=…)` tool.

use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    domain::collab::{self, MAX_FILE_BYTES},
    error::{AppError, AppResult},
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new().route("/f/{id}/{filename}", get(serve_file)).route("/u/{code}", put(upload))
}

fn valid_code(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric())
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, [(header::CONTENT_TYPE, "text/plain; charset=utf-8")], "not found").into_response()
}

/// Types browsers may render inline. Everything else (HTML, SVG, JS, …) is forced to download.
const INLINE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
    "image/bmp",
    "image/x-icon",
    "application/pdf",
    "text/plain",
];

/// Response with the safe headers for user-uploaded content.
pub fn file_response(filename: &str, content_type: &str, body: impl Into<Body>) -> Response {
    let essence = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let inline = INLINE_TYPES.contains(&essence.as_str());
    let ct = if essence == "text/plain" { "text/plain; charset=utf-8".to_string() } else { content_type.to_string() };
    let mut resp = Response::new(body.into());
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&ct).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    let safe_name = collab::sanitize_filename(filename);
    let disposition = if inline { format!("inline; filename=\"{safe_name}\"") } else { format!("attachment; filename=\"{safe_name}\"") };
    if let Ok(v) = HeaderValue::from_str(&disposition) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    // Chrome's PDF viewer refuses to run under a sandbox CSP; everything else gets locked down.
    if essence != "application/pdf" {
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"),
        );
    }
    // The dashboard (another origin) embeds these images.
    h.insert(header::HeaderName::from_static("cross-origin-resource-policy"), HeaderValue::from_static("cross-origin"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    resp
}

async fn serve_file(State(state): State<AppState>, Path((id, _filename)): Path<(String, String)>) -> Response {
    if !valid_code(&id) {
        return not_found();
    }
    let (filename, content_type, _size) = match collab::file_meta(&state.db, &id).await {
        Ok(Some(m)) => m,
        Ok(None) => return not_found(),
        Err(e) => return e.into_response(),
    };
    match state.storage.get(&id).await {
        Ok(Some(obj)) => {
            let mut resp = file_response(&filename, &content_type, obj.body);
            if let Some(n) = obj.size {
                resp.headers_mut().insert(header::CONTENT_LENGTH, HeaderValue::from(n));
            }
            resp
        }
        Ok(None) => {
            tracing::warn!(%id, "file row without data");
            not_found()
        }
        Err(e) => {
            tracing::error!(%id, error = %e, "file storage read failed");
            AppError::Internal(e).into_response()
        }
    }
}

#[derive(Deserialize)]
struct UploadQuery {
    filename: Option<String>,
}

/// `PUT /u/<code>?filename=x` with the raw file as body (`curl -T file <url>`). One use only.
async fn upload(
    State(state): State<AppState>,
    Path(code): Path<String>,
    Query(q): Query<UploadQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<Response> {
    let gone = || AppError::NotFound("upload link is invalid, used or expired; call the upload tool again for a new one".into());
    if !valid_code(&code) || !collab::upload_link_valid(&state.db, &code).await? {
        return Err(gone());
    }
    if body.is_empty() {
        return Err(AppError::BadRequest("empty body; upload with: curl -T <file> '<this url>'".into()));
    }
    if body.len() > MAX_FILE_BYTES {
        return Ok((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({ "error": "too_large", "message": "file is over 10 MB; max is 10 MB" })),
        )
            .into_response());
    }
    let actor = collab::consume_upload_link(&state.db, &code).await?.ok_or_else(gone)?;
    let filename = q.filename.unwrap_or_else(|| "file".into());
    let ct = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok());
    let info = collab::save_file(&state, &actor, &filename, ct, &body).await?;
    let mut v = serde_json::to_value(&info).map_err(|e| AppError::Internal(e.into()))?;
    v["markdown"] = json!(format!("![{}]({})", info.filename, info.url));
    Ok(Json(v).into_response())
}

// ---------------------------------------------------------------------------------------------
// Remote fetch for upload(url=…)
// ---------------------------------------------------------------------------------------------

/// Only globally routable addresses: no loopback, private, link-local, CGNAT, metadata, etc.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64)
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || (o[0] == 198 && (o[1] & 0xfe) == 18)
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(v4));
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] == 0x2001 && s[1] == 0x0db8)
                || (s[0] == 0x64 && s[1] == 0xff9b))
        }
    }
}

/// Download a public http(s) URL (max 10 MB, no redirects, pinned to the vetted IPs).
/// Returns the bytes and the Content-Type header.
pub async fn fetch_public_url(raw: &str) -> Result<(Vec<u8>, Option<String>), String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "url must be an absolute http(s) URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("url must be http or https".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("url must not contain credentials".into());
    }
    let port = url.port_or_known_default().ok_or("url has no port")?;
    let host = url.host_str().ok_or("url has no host")?.to_string();
    let refuse = || "refusing to fetch a private or local address".to_string();
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .no_proxy();
    let literal = host.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>().ok();
    match literal {
        Some(ip) => {
            if !is_public_ip(ip) {
                return Err(refuse());
            }
        }
        None => {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
                .await
                .map_err(|e| format!("could not resolve {host}: {e}"))?
                .collect();
            if addrs.is_empty() {
                return Err(format!("could not resolve {host}"));
            }
            if addrs.iter().any(|a| !is_public_ip(a.ip())) {
                return Err(refuse());
            }
            builder = builder.resolve_to_addrs(&host, &addrs);
        }
    }
    let client = builder.build().map_err(|e| format!("http client: {e}"))?;
    let mut resp = client.get(url).send().await.map_err(|e| format!("fetch failed: {e}"))?;
    let status = resp.status();
    if status.is_redirection() {
        let to = resp.headers().get(header::LOCATION).and_then(|v| v.to_str().ok()).unwrap_or("?");
        return Err(format!("url redirects (to {to}); pass the final URL"));
    }
    if !status.is_success() {
        return Err(format!("fetch failed: HTTP {status}"));
    }
    if resp.content_length().is_some_and(|n| n as usize > MAX_FILE_BYTES) {
        return Err("file is over 10 MB; max is 10 MB".into());
    }
    let ct = resp.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_string);
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("fetch failed: {e}"))? {
        if buf.len() + chunk.len() > MAX_FILE_BYTES {
            return Err("file is over 10 MB; max is 10 MB".into());
        }
        buf.extend_from_slice(&chunk);
    }
    Ok((buf, ct))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_ip_filter() {
        for ip in ["127.0.0.1", "10.1.2.3", "192.168.1.1", "172.16.0.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "::1", "fd00::1", "fe80::1", "::ffff:127.0.0.1"] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "46.225.177.152", "2606:4700:4700::1111"] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn safe_headers() {
        let r = file_response("x.html", "text/html", b"<script>".to_vec());
        let h = r.headers();
        assert!(h[header::CONTENT_DISPOSITION].to_str().unwrap().starts_with("attachment"));
        assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(h[header::CACHE_CONTROL], "public, max-age=31536000, immutable");
        let svg = file_response("x.svg", "image/svg+xml", b"<svg/>".to_vec());
        assert!(svg.headers()[header::CONTENT_DISPOSITION].to_str().unwrap().starts_with("attachment"));
        let png = file_response("x.png", "image/png", vec![1]);
        assert!(png.headers()[header::CONTENT_DISPOSITION].to_str().unwrap().starts_with("inline"));
        let txt = file_response("x.txt", "text/plain", vec![b'a']);
        assert_eq!(txt.headers()[header::CONTENT_TYPE], "text/plain; charset=utf-8");
    }

    #[tokio::test]
    async fn fetch_refuses_local() {
        assert!(fetch_public_url("http://127.0.0.1:8191/healthz").await.unwrap_err().contains("private"));
        assert!(fetch_public_url("http://localhost/").await.unwrap_err().contains("private"));
        assert!(fetch_public_url("http://[::1]/").await.unwrap_err().contains("private"));
        assert!(fetch_public_url("file:///etc/passwd").await.is_err());
    }
}
