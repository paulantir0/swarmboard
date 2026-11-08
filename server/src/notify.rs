//! Owner notifications: one plain-text email per new signup, sent to `ADMIN_NOTIFY_EMAIL`
//! (unset = off). Sent in the background so signup latency never depends on Resend.

use std::time::Duration;

use axum::http::HeaderMap;
use serde_json::json;

use crate::state::AppState;

/// What we know about a new signup at the moment the magic link is redeemed.
#[derive(Debug, Clone)]
pub struct Signup {
    pub email: String,
    pub handle: String,
    pub ip: String,
    /// Org slug if they joined through an invite.
    pub joined: Option<String>,
}

/// Fire-and-forget email to the owner. No-op when `ADMIN_NOTIFY_EMAIL` is unset.
pub fn signup(state: &AppState, s: Signup, headers: &HeaderMap) {
    let Some(to) = std::env::var("ADMIN_NOTIFY_EMAIL").ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) else {
        return;
    };
    let (subject, text) = render(&s, headers);
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = send(&state, &to, &subject, &text).await {
            tracing::warn!(error = %e, "signup notification failed");
        }
    });
}

fn render(s: &Signup, headers: &HeaderMap) -> (String, String) {
    let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    // Vercel's geo headers, when its /api rewrite passes them through.
    let location = [h("x-vercel-ip-city").map(|c| c.replace("%20", " ")), h("x-vercel-ip-country-region"), h("x-vercel-ip-country")]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
    let mut lines = vec![
        format!("email:   {}", s.email),
        format!("handle:  {}", s.handle),
        format!("ip:      {}", s.ip),
    ];
    if !location.is_empty() {
        lines.push(format!("where:   {location}"));
    }
    lines.push(match &s.joined {
        Some(slug) => format!("joined:  swarmboard.app/{slug} (invite)"),
        None => "joined:  no org yet (will name one)".to_string(),
    });
    if let Some(ua) = h("user-agent") {
        lines.push(format!("browser: {ua}"));
    }
    (format!("New swarmboard signup: {}", s.email), lines.join("\n") + "\n")
}

async fn send(state: &AppState, to: &str, subject: &str, text: &str) -> anyhow::Result<()> {
    let Some(key) = state.cfg.resend_api_key.as_deref() else {
        tracing::info!(%to, %subject, "signup notification (no RESEND_API_KEY, not sent):\n{text}");
        return Ok(());
    };
    let resp = state
        .http
        .post("https://api.resend.com/emails")
        .bearer_auth(key)
        .timeout(Duration::from_secs(10))
        .json(&json!({ "from": state.cfg.email_from, "to": [to], "subject": subject, "text": text }))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        anyhow::bail!("resend {status}: {}", resp.text().await.unwrap_or_default());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_signup_with_ip_and_location() {
        let mut h = HeaderMap::new();
        h.insert("x-vercel-ip-city", "San%20Francisco".parse().unwrap());
        h.insert("x-vercel-ip-country", "US".parse().unwrap());
        h.insert("user-agent", "Mozilla/5.0".parse().unwrap());
        let s = Signup { email: "bob@acme.dev".into(), handle: "bob".into(), ip: "203.0.113.7".into(), joined: Some("acme".into()) };
        let (subject, text) = render(&s, &h);
        assert_eq!(subject, "New swarmboard signup: bob@acme.dev");
        assert!(text.contains("ip:      203.0.113.7"), "{text}");
        assert!(text.contains("where:   San Francisco, US"), "{text}");
        assert!(text.contains("joined:  swarmboard.app/acme (invite)"), "{text}");
        assert!(text.contains("browser: Mozilla/5.0"), "{text}");
    }

    #[test]
    fn renders_without_optional_headers() {
        let s = Signup { email: "a@b.co".into(), handle: "a".into(), ip: "unknown".into(), joined: None };
        let (_, text) = render(&s, &HeaderMap::new());
        assert!(!text.contains("where:"));
        assert!(text.contains("no org yet"));
    }
}
