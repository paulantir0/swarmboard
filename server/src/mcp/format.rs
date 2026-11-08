//! Compact, agent-readable text renderings of domain values (tool results + hook context).

use crate::{
    domain::collab::{ago, leases::LeaseRow, truncate_chars},
    models::{Agent, Board, Comment, Ping, Post},
};

/// "in 28m", "in 40s", "now".
pub fn until(t: i64, now: i64) -> String {
    let s = (t - now) / 1000;
    match s {
        i64::MIN..=0 => "now".into(),
        1..=59 => format!("in {s}s"),
        60..=7199 => format!("in {}m", s / 60),
        _ => format!("in {}h", s / 3600),
    }
}

/// `server/src/auth/** [github.com/acme/app] — paul/api "refactor" · expires in 28m`
pub fn lease_line(l: &LeaseRow, me: Option<&str>, now: i64) -> String {
    let mut s = l.scope.clone();
    if !l.repo.is_empty() {
        s.push_str(&format!(" [{}]", l.repo));
    }
    s.push_str(&format!(" — {}", l.display()));
    if me == Some(l.agent_id.as_str()) {
        s.push_str(" (you)");
    }
    if !l.note.is_empty() {
        s.push_str(&format!(" \"{}\"", truncate_chars(&l.note, 100)));
    }
    s.push_str(&format!(" · expires {}", until(l.expires_at, now)));
    s
}

pub fn repo_label(repo: Option<&str>, branch: Option<&str>) -> Option<String> {
    match (repo.filter(|r| !r.is_empty()), branch.filter(|b| !b.is_empty())) {
        (Some(r), Some(b)) => Some(format!("{r}@{b}")),
        (Some(r), None) => Some(r.to_string()),
        (None, Some(b)) => Some(format!("@{b}")),
        _ => None,
    }
}

pub fn priority_label(p: i64) -> Option<&'static str> {
    match p {
        1 => Some("urgent"),
        2 => Some("high"),
        3 => Some("medium"),
        4 => Some("low"),
        _ => None,
    }
}

/// `#12 [in_progress] task  Fix login redirect — paul/ui-fixer (claimed by paul/ui-fixer) · ui · 3m ago`
/// (the board key is the post's topic)
pub fn post_line(p: &Post, now: i64) -> String {
    let mut s = format!("#{} [{}] {}", p.number, p.status, p.kind);
    if let Some(l) = priority_label(p.priority) {
        s.push_str(&format!(" !{l}"));
    }
    s.push_str(&format!("  {} — {}", p.title, p.author.display));
    if let Some(c) = &p.claimed_by {
        s.push_str(&format!(" (claimed by {})", c.display));
    }
    s.push_str(&format!(" · {} · {}", p.board_key, ago(p.updated_at, now)));
    if p.comment_count > 0 {
        s.push_str(&format!(" · {} comment{}", p.comment_count, if p.comment_count == 1 { "" } else { "s" }));
    }
    s
}

pub fn post_full(p: &Post, comments: &[Comment], now: i64) -> String {
    let mut head = format!("#{} [{}] {} · board {}", p.number, p.status, p.kind, p.board_key);
    if let Some(l) = priority_label(p.priority) {
        head.push_str(&format!(" · {l} priority"));
    }
    let mut out = vec![head, p.title.clone()];
    let mut meta = format!("by {} · created {} · updated {}", p.author.display, ago(p.created_at, now), ago(p.updated_at, now));
    match (&p.claimed_by, p.claimed_at) {
        (Some(c), Some(at)) => meta.push_str(&format!(" · claimed by {} {}", c.display, ago(at, now))),
        (Some(c), None) => meta.push_str(&format!(" · claimed by {}", c.display)),
        _ => meta.push_str(" · unclaimed"),
    }
    out.push(meta);
    out.push(String::new());
    out.push(if p.body.trim().is_empty() { "(no body)".to_string() } else { p.body.trim_end().to_string() });
    if !comments.is_empty() {
        out.push(String::new());
        out.push(format!("Comments ({}):", comments.len()));
        for c in comments {
            out.push(format!("— {} · {}", c.author.display, ago(c.created_at, now)));
            out.push(c.body.trim_end().to_string());
        }
    }
    out.join("\n")
}

pub fn board_line(b: &Board, now: i64) -> String {
    let mut s = format!("{} · {} open / {} total", b.key, b.open_count, b.total_count);
    if b.name != b.key {
        s = format!("{} ({}) · {} open / {} total", b.key, b.name, b.open_count, b.total_count);
    }
    if let Some(t) = b.last_activity_at {
        s.push_str(&format!(" · last post activity {}", ago(t, now)));
    }
    if !b.description.is_empty() {
        s.push_str(&format!(" — {}", truncate_chars(&b.description, 120)));
    }
    s
}

pub fn agent_line(a: &Agent, me: Option<&str>, now: i64) -> String {
    let mut s = a.display.clone();
    if me == Some(a.id.as_str()) {
        s.push_str(" (you)");
    }
    if !a.active {
        s.push_str(" (inactive)");
    }
    if let Some(c) = &a.client {
        s.push_str(&format!(" · {c}"));
    }
    if let Some(r) = repo_label(a.repo.as_deref(), a.branch.as_deref()) {
        s.push_str(&format!(" · {r}"));
    }
    if let Some(w) = a.working_on.as_deref().filter(|w| !w.is_empty()) {
        s.push_str(&format!(" · on: {}", truncate_chars(w, 120)));
    }
    let mut held: Vec<String> = a.leases.clone();
    for n in &a.claimed {
        let tag = format!("#{n}");
        if !held.contains(&tag) {
            held.push(tag);
        }
    }
    if !held.is_empty() {
        s.push_str(&format!(" · holds {}", held.join(", ")));
    }
    s.push_str(&format!(" · seen {}", ago(a.last_seen_at, now)));
    s
}

/// `- bob/api → you (#12, 3m ago): can you check the redirect?`
pub fn ping_line(p: &Ping, me_display: Option<&str>, now: i64) -> String {
    let target = match me_display {
        Some(me) if p.target.trim_start_matches('@') == me || me.ends_with(&format!("/{}", p.target)) => "you".to_string(),
        _ => p.target.clone(),
    };
    let mut ctx = Vec::new();
    if let Some(n) = p.post_number {
        ctx.push(format!("#{n}"));
    }
    ctx.push(ago(p.created_at, now));
    format!("- {} → {} ({}): {}", p.from.display, target, ctx.join(", "), truncate_chars(p.body.trim(), 1500))
}

/// Block appended to tool results / injected by hooks.
pub fn pings_block(pings: &[Ping], me_display: Option<&str>, now: i64) -> String {
    let n = pings.len();
    let mut out = vec![format!(
        "[swarmboard: {n} new ping{} for {} — answer with send]",
        if n == 1 { "" } else { "s" },
        me_display.unwrap_or("you")
    )];
    out.extend(pings.iter().map(|p| ping_line(p, me_display, now)));
    out.join("\n")
}

/// Humans render as `@handle`, agents as `handle/agent`.
pub fn recipient_label(display: &str) -> String {
    if display.contains('/') { display.to_string() } else { format!("@{display}") }
}
