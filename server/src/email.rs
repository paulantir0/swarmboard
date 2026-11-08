//! Transactional email via Resend. OWNER: email subagent.
//! Without RESEND_API_KEY (local dev) emails are logged to stdout instead of sent.

use std::time::Duration;

use anyhow::anyhow;
use serde::Deserialize;
use serde_json::json;

use crate::{
    error::{AppError, AppResult},
    state::AppState,
    util::random_token,
};

const RESEND_EMAILS_URL: &str = "https://api.resend.com/emails";
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

/// What the magic link is for; changes only the small line of context in the email.
#[derive(Debug, Clone)]
pub enum MagicLinkPurpose {
    SignIn,
    /// Joining `team_name`, invited by `inviter`.
    Join { team_name: String, inviter: String },
}

/// Send the sign-in email containing `link` (already a full URL to the web app).
pub async fn send_magic_link(state: &AppState, to: &str, link: &str, purpose: MagicLinkPurpose) -> AppResult<()> {
    let Some(key) = state.cfg.resend_api_key.as_deref() else {
        tracing::info!(%to, %link, ?purpose, "magic link (RESEND_API_KEY unset, not sent)");
        return Ok(());
    };
    let msg = render(to, link, &purpose);
    let id = post_email(&state.http, RESEND_EMAILS_URL, key, &state.cfg.email_from, to, &msg).await?;
    tracing::info!(%to, resend_id = %id, "magic link sent");
    Ok(())
}

/// A rendered email, ready to hand to Resend.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub subject: String,
    pub html: String,
    pub text: String,
}

#[derive(Deserialize)]
struct ResendOk {
    #[serde(default)]
    id: String,
}

#[derive(Deserialize)]
struct ResendErr {
    #[serde(default)]
    name: String,
    #[serde(default)]
    message: String,
}

/// POST one email to Resend. Returns the Resend email id. Errors carry Resend's message, never the key.
async fn post_email(
    http: &reqwest::Client,
    endpoint: &str,
    key: &str,
    from: &str,
    to: &str,
    msg: &Rendered,
) -> AppResult<String> {
    let body = json!({
        "from": from,
        "to": [to],
        "subject": msg.subject,
        "html": msg.html,
        "text": msg.text,
        // Unique per message so Gmail doesn't fold repeated sign-in emails into one thread.
        "headers": { "X-Entity-Ref-ID": random_token(24) },
    });
    let res = http
        .post(endpoint)
        .bearer_auth(key)
        .header("Idempotency-Key", format!("magic-link-{}", random_token(32)))
        .timeout(SEND_TIMEOUT)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            let what = if e.is_timeout() { "timed out" } else { "failed" };
            AppError::Internal(anyhow!("resend request {what}: {}", e.without_url()))
        })?;

    let status = res.status();
    let raw = res.text().await.unwrap_or_default();
    if status.is_success() {
        let id = serde_json::from_str::<ResendOk>(&raw).map(|r| r.id).unwrap_or_default();
        return Ok(id);
    }
    let detail = match serde_json::from_str::<ResendErr>(&raw) {
        Ok(e) if !e.message.is_empty() && !e.name.is_empty() => format!("{}: {}", e.name, e.message),
        Ok(e) if !e.message.is_empty() => e.message,
        _ => raw.chars().take(300).collect(),
    };
    Err(AppError::Internal(anyhow!("resend {}: {}", status.as_u16(), detail)))
}

// ---------------------------------------------------------------------------------------------
// Rendering

/// Body copy that differs between the two variants. All fields are plain text (not yet escaped).
struct Copy {
    subject: String,
    preheader: String,
    button: String,
    /// The one line above the button, as HTML (already escaped).
    lead_html: String,
    /// The same line for the plain-text part.
    lead_text: String,
}

const FOOTER: &str = "Expires in 20 minutes. If you didn't request this, ignore it.";

fn copy_for(to: &str, purpose: &MagicLinkPurpose) -> Copy {
    match purpose {
        MagicLinkPurpose::SignIn => Copy {
            subject: "Sign in to swarmboard".into(),
            preheader: "Your sign-in link. Expires in 20 minutes.".into(),
            button: "Sign in".into(),
            lead_html: format!("Sign in as <b style=\"font-weight:600;\">{}</b>.", no_autolink(&escape_html(to))),
            lead_text: format!("Sign in as {}:", one_line(to, 254)),
        },
        MagicLinkPurpose::Join { team_name, inviter } => {
            let team = one_line(team_name, 60);
            let inviter = one_line(inviter, 40);
            let subject = if team.is_empty() { "Join swarmboard".to_string() } else { format!("Join {team} on swarmboard") };
            let button = if team.is_empty() { "Join".to_string() } else { format!("Join {}", one_line(&team, 32)) };
            let who = if inviter.is_empty() { "Someone".to_string() } else { inviter.clone() };
            let target = if team.is_empty() { "a team".to_string() } else { team.clone() };
            Copy {
                subject,
                preheader: format!("{who} invited you to {target}."),
                button,
                lead_html: format!(
                    "<b style=\"font-weight:600;\">{}</b> invited you to <b style=\"font-weight:600;\">{}</b>.",
                    escape_html(&who),
                    escape_html(&target)
                ),
                lead_text: format!("{who} invited you to {target}. Join:"),
            }
        }
    }
}

/// Render subject, HTML and plain text for a magic-link email.
pub fn render(to: &str, link: &str, purpose: &MagicLinkPurpose) -> Rendered {
    let c = copy_for(to, purpose);
    let href = escape_html(link);
    let html = format!(
        r##"<!doctype html>
<html lang="en" xmlns="http://www.w3.org/1999/xhtml" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="x-apple-disable-message-reformatting">
<meta name="format-detection" content="telephone=no, date=no, address=no, email=no, url=no">
<meta name="color-scheme" content="light dark">
<meta name="supported-color-schemes" content="light dark">
<title>{title}</title>
<!--[if mso]><noscript><xml><o:OfficeDocumentSettings><o:PixelsPerInch>96</o:PixelsPerInch></o:OfficeDocumentSettings></xml></noscript><style>{mso_style}</style><![endif]-->
<style>{style}</style>
</head>
<body class="bg" style="margin:0;padding:0;width:100%;background:#fafafa;-webkit-text-size-adjust:100%;-ms-text-size-adjust:100%;">
<div style="display:none;font-size:1px;line-height:1px;max-height:0;max-width:0;opacity:0;overflow:hidden;mso-hide:all;">{preheader}{preheader_pad}</div>
<table role="presentation" class="bg" width="100%" cellpadding="0" cellspacing="0" border="0" bgcolor="#fafafa" style="width:100%;background:#fafafa;">
<tr>
<td class="outer" align="center" style="padding:48px 16px;">
<!--[if mso]><table role="presentation" width="480" cellpadding="0" cellspacing="0" border="0" align="center"><tr><td><![endif]-->
<table role="presentation" class="card" width="100%" cellpadding="0" cellspacing="0" border="0" bgcolor="#ffffff" style="width:100%;max-width:480px;background:#ffffff;border:1px solid #e5e5e5;border-radius:4px;border-collapse:separate;">
<tr>
<td class="pad" style="padding:32px 36px 28px;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0">
<tr><td class="ink mono" style="padding:0 0 20px;font-family:{mono};font-size:15px;line-height:20px;font-weight:500;letter-spacing:-0.2px;color:#111111;">swarmboard</td></tr>
<tr><td class="rule" height="1" bgcolor="#e5e5e5" style="height:1px;font-size:1px;line-height:1px;mso-line-height-rule:exactly;background:#e5e5e5;">&nbsp;</td></tr>
<tr><td class="ink" style="padding:24px 0 20px;font-family:{sans};font-size:15px;line-height:24px;color:#111111;">{lead}</td></tr>
<tr><td style="padding:0 0 16px;">
<table role="presentation" cellpadding="0" cellspacing="0" border="0"><tr>
<td class="btn-td" bgcolor="#2c4a7c" style="background:#2c4a7c;border-radius:3px;">
<a class="btn" href="{href}" target="_blank" style="display:inline-block;padding:12px 22px;border:1px solid #2c4a7c;border-radius:3px;background:#2c4a7c;color:#ffffff;font-family:{sans};font-size:15px;line-height:20px;font-weight:600;text-decoration:none;mso-padding-alt:0;text-underline-color:#2c4a7c;"><!--[if mso]><i style="mso-font-width:150%;mso-text-raise:24pt" hidden>&#8195;</i><span style="mso-text-raise:12pt;"><![endif]-->{button}<!--[if mso]></span><i style="mso-font-width:150%;" hidden>&#8195;&#8203;</i><![endif]--></a>
</td>
</tr></table>
</td></tr>
<tr><td class="muted mono url" style="padding:0 0 24px;font-family:{mono};font-size:12px;line-height:18px;color:#666666;word-break:break-all;"><a class="muted" href="{href}" target="_blank" style="color:#666666;text-decoration:none;word-break:break-all;">{href}</a></td></tr>
<tr><td class="rule" height="1" bgcolor="#e5e5e5" style="height:1px;font-size:1px;line-height:1px;mso-line-height-rule:exactly;background:#e5e5e5;">&nbsp;</td></tr>
<tr><td class="muted" style="padding:16px 0 0;font-family:{sans};font-size:12px;line-height:18px;color:#666666;">{footer}</td></tr>
</table>
</td>
</tr>
</table>
<!--[if mso]></td></tr></table><![endif]-->
</td>
</tr>
</table>
</body>
</html>
"##,
        title = escape_html(&c.subject),
        mso_style = MSO_STYLE,
        style = STYLE,
        preheader = escape_html(&c.preheader),
        preheader_pad = PREHEADER_PAD,
        mono = MONO,
        sans = SANS,
        lead = c.lead_html,
        href = href,
        button = escape_html(&c.button),
        footer = escape_html(FOOTER),
    );
    let text = format!("swarmboard\n\n{}\n\n{}\n\n{}\n", c.lead_text, link, FOOTER);
    Rendered { subject: c.subject, html, text }
}

const SANS: &str = "Inter,-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif";
const MONO: &str = "'IBM Plex Mono',ui-monospace,SFMono-Regular,Menlo,Consolas,'Liberation Mono',monospace";

/// Outlook desktop ignores font stacks with unknown first fonts; pin it to safe ones.
const MSO_STYLE: &str = "table,td,p,a,span,b{font-family:Arial,Helvetica,sans-serif !important;}.mono,.mono a{font-family:Consolas,'Courier New',monospace !important;}";

/// Dark palette mirrors the IndexerLabs warm-dark tokens. `[data-ogsc]`/`[data-ogsb]` target Outlook.com.
const STYLE: &str = r#"
:root { color-scheme: light dark; supported-color-schemes: light dark; }
body { margin: 0 !important; padding: 0 !important; width: 100% !important; }
a[x-apple-data-detectors] { color: inherit !important; text-decoration: none !important; font: inherit !important; }
@media (max-width: 520px) {
  .outer { padding: 24px 12px !important; }
  .pad { padding: 28px 24px 24px !important; }
}
@media (prefers-color-scheme: dark) {
  .bg { background: #11110f !important; }
  .card { background: #1b1a17 !important; border-color: #34312b !important; }
  .rule { background: #34312b !important; }
  .ink { color: #f2eee6 !important; }
  .muted { color: #b8b0a4 !important; }
  .btn-td { background: #9db9ea !important; }
  .btn { background: #9db9ea !important; border-color: #9db9ea !important; color: #11110f !important; }
}
[data-ogsb] .bg { background: #11110f !important; }
[data-ogsb] .card { background: #1b1a17 !important; border-color: #34312b !important; }
[data-ogsb] .rule { background: #34312b !important; }
[data-ogsc] .ink { color: #f2eee6 !important; }
[data-ogsc] .muted { color: #b8b0a4 !important; }
[data-ogsb] .btn-td, [data-ogsb] .btn { background: #9db9ea !important; border-color: #9db9ea !important; }
[data-ogsc] .btn { color: #11110f !important; }
"#;

/// Keeps clients from pulling body text into the inbox preview after the preheader.
const PREHEADER_PAD: &str = "&#8199;&#847;&#8199;&#847;&#8199;&#847;&#8199;&#847;&#8199;&#847;&#8199;&#847;&#8199;&#847;&#8199;&#847;&#8199;&#847;&#8199;&#847;&#8199;&#847;&#8199;&#847;";

/// Escape for HTML text and double-quoted attribute values.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Collapse control characters and runs of whitespace to single spaces, trim, cap at `max` chars.
fn one_line(s: &str, max: usize) -> String {
    let cleaned = s
        .chars()
        .map(|c| if c.is_control() || c.is_whitespace() { ' ' } else { c })
        .collect::<String>();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max {
        collapsed
    } else {
        let mut t: String = collapsed.chars().take(max.saturating_sub(1)).collect();
        t = t.trim_end().to_string();
        t.push('…');
        t
    }
}

/// Insert a zero-width non-joiner after `@` and before `.` so Gmail/Outlook don't turn an
/// (escaped) address into a blue mailto link. Input must already be HTML-escaped.
fn no_autolink(escaped: &str) -> String {
    escaped.replace('@', "@&#8204;").replace('.', "&#8204;.")
}

#[cfg(test)]
mod tests {

    use axum::{
        Json, Router,
        http::{HeaderMap, StatusCode},
        routing::post,
    };

    use super::*;
    use crate::config::Config;

    const LINK: &str = "https://swarmboard.app/auth?token=Ab12Cd34Ef56Gh78Ij90Kl12Mn34Op56";

    fn join(team: &str, inviter: &str) -> MagicLinkPurpose {
        MagicLinkPurpose::Join { team_name: team.into(), inviter: inviter.into() }
    }

    #[test]
    fn signin_variant() {
        let r = render("paul@example.com", LINK, &MagicLinkPurpose::SignIn);
        assert_eq!(r.subject, "Sign in to swarmboard");
        assert!(r.html.starts_with("<!doctype html>"));
        assert!(r.html.contains(r#"<meta name="color-scheme" content="light dark">"#));
        assert!(r.html.contains("@media (prefers-color-scheme: dark)"));
        assert!(r.html.contains("max-width:480px"));
        assert!(r.html.contains(">swarmboard</td>"));
        assert!(r.html.contains("<title>Sign in to swarmboard</title>"));
        assert_eq!(r.html.matches(&format!(r#"href="{LINK}""#)).count(), 2, "button + raw link");
        assert!(r.html.contains("-->Sign in<!--"), "button label");
        assert!(r.html.contains("#2c4a7c"));
        assert!(r.html.contains("Expires in 20 minutes."));
        assert!(r.html.contains("paul@&#8204;example&#8204;.com"));
        assert!(r.text.starts_with("swarmboard\n\n"));
        assert!(r.text.contains("Sign in as paul@example.com:"));
        assert!(r.text.contains(&format!("\n{LINK}\n")));
        assert!(r.text.contains("Expires in 20 minutes."));
        assert!(!r.text.contains('<'));
    }

    #[test]
    fn join_variant() {
        let r = render("amy@example.com", LINK, &join("paul's board", "paul"));
        assert_eq!(r.subject, "Join paul's board on swarmboard");
        assert!(r.html.contains("-->Join paul&#39;s board<!--"));
        assert!(r.html.contains("<b style=\"font-weight:600;\">paul</b> invited you to <b style=\"font-weight:600;\">paul&#39;s board</b>."));
        assert!(r.text.contains("paul invited you to paul's board. Join:"));
        assert!(r.text.contains(LINK));
    }

    #[test]
    fn escapes_interpolated_values() {
        let team = r#"<script>alert("x")</script> & co"#;
        let inviter = r#"<img src=x onerror=alert(1)>"#;
        let to = r#""><b>@evil.test"#;
        let link = "https://swarmboard.app/auth?token=a&b=\"c\"<d>";
        let r = render(to, link, &join(team, inviter));
        for raw in ["<script>", "<img", "\"><b>", "\"c\"<d>"] {
            assert!(!r.html.contains(raw), "unescaped {raw:?} in html");
        }
        assert!(r.html.contains("&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; &amp; co"));
        assert!(r.html.contains("&lt;img src=x onerror=alert(1)&gt;"));
        assert!(r.html.contains(r#"href="https://swarmboard.app/auth?token=a&amp;b=&quot;c&quot;&lt;d&gt;""#));
        // Subject and text are not HTML: raw characters are fine there, but never line breaks.
        assert!(r.subject.contains("<script>"));
        let r2 = render(to, LINK, &MagicLinkPurpose::SignIn);
        assert!(!r2.html.contains("\"><b>"));
        assert!(r2.html.contains("&quot;&gt;&lt;b&gt;@&#8204;evil&#8204;.test"));
    }

    #[test]
    fn header_and_label_sanitizing() {
        let r = render("a@b.c", LINK, &join("acme\r\nBcc: x@y.z\t  team", "bob\nsmith"));
        assert_eq!(r.subject, "Join acme Bcc: x@y.z team on swarmboard");
        assert!(!r.text.contains("bob\nsmith"));
        let long = "x".repeat(100);
        let r = render("a@b.c", LINK, &join(&long, ""));
        assert!(r.subject.chars().count() <= "Join  on swarmboard".len() + 60);
        assert!(r.subject.contains('…'));
        assert!(r.html.contains("<b style=\"font-weight:600;\">Someone</b>"));
        let r = render("a@b.c", LINK, &join("  ", "bob"));
        assert_eq!(r.subject, "Join swarmboard");
    }

    #[tokio::test]
    async fn without_key_logs_and_succeeds() {
        let mut cfg = Config::from_env();
        cfg.resend_api_key = None;
        let state = AppState::new(crate::db::test_pool().await, cfg).unwrap();
        send_magic_link(&state, "paul@example.com", LINK, MagicLinkPurpose::SignIn).await.unwrap();
    }

    /// Spin up a fake Resend that checks the request shape and answers with `status` + `reply`.
    async fn fake_resend(status: StatusCode, reply: serde_json::Value) -> String {
        let app = Router::new().route(
            "/emails",
            post(move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
                let reply = reply.clone();
                async move {
                    assert_eq!(headers.get("authorization").unwrap(), "Bearer re_test_key");
                    let idem = headers.get("idempotency-key").unwrap().to_str().unwrap();
                    assert!(idem.starts_with("magic-link-") && idem.len() > 20);
                    assert_eq!(body["from"], "swarmboard <login@swarmboard.app>");
                    assert_eq!(body["to"], json!(["paul@example.com"]));
                    assert_eq!(body["subject"], "Sign in to swarmboard");
                    assert!(body["html"].as_str().unwrap().contains(LINK));
                    assert!(body["text"].as_str().unwrap().contains(LINK));
                    (status, Json(reply))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/emails")
    }

    #[tokio::test]
    async fn posts_to_resend() {
        let url = fake_resend(StatusCode::OK, json!({"id": "em_123"})).await;
        let msg = render("paul@example.com", LINK, &MagicLinkPurpose::SignIn);
        let http = reqwest::Client::new();
        let id = post_email(&http, &url, "re_test_key", "swarmboard <login@swarmboard.app>", "paul@example.com", &msg)
            .await
            .unwrap();
        assert_eq!(id, "em_123");
    }

    #[tokio::test]
    async fn maps_resend_errors() {
        let url = fake_resend(
            StatusCode::FORBIDDEN,
            json!({"statusCode": 403, "name": "validation_error", "message": "The swarmboard.app domain is not verified."}),
        )
        .await;
        let msg = render("paul@example.com", LINK, &MagicLinkPurpose::SignIn);
        let http = reqwest::Client::new();
        let err = post_email(&http, &url, "re_test_key", "swarmboard <login@swarmboard.app>", "paul@example.com", &msg)
            .await
            .unwrap_err();
        let s = err.to_string();
        assert!(matches!(err, AppError::Internal(_)));
        assert_eq!(s, "resend 403: validation_error: The swarmboard.app domain is not verified.");
        assert!(!s.contains("re_test_key"));
    }

    #[tokio::test]
    async fn maps_network_errors() {
        // Nothing listens on port 9 (discard) on localhost in CI/dev.
        let msg = render("paul@example.com", LINK, &MagicLinkPurpose::SignIn);
        let http = reqwest::Client::new();
        let err = post_email(&http, "http://127.0.0.1:9/emails", "re_test_key", "f", "t", &msg).await.unwrap_err();
        let s = err.to_string();
        assert!(s.starts_with("resend request "), "{s}");
        assert!(!s.contains("re_test_key"));
    }

    /// `EMAIL_PREVIEW_DIR=/some/dir cargo test email::` writes both variants there for eyeballing.
    #[test]
    fn write_previews() {
        let Ok(dir) = std::env::var("EMAIL_PREVIEW_DIR") else { return };
        std::fs::create_dir_all(&dir).unwrap();
        let signin = render("paul@indexerlabs.com", LINK, &MagicLinkPurpose::SignIn);
        let joined = render("amy@example.com", LINK, &join("paul's board", "paul"));
        for (name, r) in [("signin", &signin), ("join", &joined)] {
            std::fs::write(format!("{dir}/preview-{name}.html"), &r.html).unwrap();
            std::fs::write(format!("{dir}/preview-{name}.txt"), format!("Subject: {}\n\n{}", r.subject, r.text)).unwrap();
        }
    }
}
