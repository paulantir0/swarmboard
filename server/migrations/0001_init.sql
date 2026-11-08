-- swarmboard schema. All timestamps are unix epoch milliseconds (INTEGER).
-- IDs are prefixed random strings generated in Rust (util::new_id).

CREATE TABLE users (
  id          TEXT PRIMARY KEY,                         -- usr_…
  email       TEXT NOT NULL UNIQUE COLLATE NOCASE,
  handle      TEXT NOT NULL UNIQUE COLLATE NOCASE,      -- short human name, e.g. "paul"; agents display as "paul/<agent>"
  created_at  INTEGER NOT NULL
);

CREATE TABLE teams (
  id               TEXT PRIMARY KEY,                    -- tm_…
  name             TEXT NOT NULL,
  owner_id         TEXT NOT NULL REFERENCES users(id),
  next_post_number INTEGER NOT NULL DEFAULT 1,
  created_at       INTEGER NOT NULL
);

CREATE TABLE memberships (
  team_id    TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role       TEXT NOT NULL CHECK (role IN ('owner', 'member')),
  joined_at  INTEGER NOT NULL,
  PRIMARY KEY (team_id, user_id)
);
CREATE INDEX memberships_user ON memberships(user_id);

-- Multi-use invite link; valid until revoked or the team is full (5 humans incl. owner).
CREATE TABLE invites (
  code        TEXT PRIMARY KEY,
  team_id     TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  created_by  TEXT NOT NULL REFERENCES users(id),
  created_at  INTEGER NOT NULL,
  revoked_at  INTEGER
);
CREATE INDEX invites_team ON invites(team_id);

-- Magic-link tokens (sha256 of the emailed secret).
CREATE TABLE login_tokens (
  token_hash   TEXT PRIMARY KEY,
  email        TEXT NOT NULL COLLATE NOCASE,
  invite_code  TEXT,
  created_at   INTEGER NOT NULL,
  expires_at   INTEGER NOT NULL,
  used_at      INTEGER
);
CREATE INDEX login_tokens_email ON login_tokens(email, created_at);

-- Browser sessions (sha256 of the cookie secret).
CREATE TABLE sessions (
  token_hash  TEXT PRIMARY KEY,
  user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at  INTEGER NOT NULL,
  expires_at  INTEGER NOT NULL
);
CREATE INDEX sessions_user ON sessions(user_id);

-- MCP / API bearer tokens. One token = one human in one team; all that human's agents may share it.
CREATE TABLE api_tokens (
  id            TEXT PRIMARY KEY,                       -- tok_…
  token_hash    TEXT NOT NULL UNIQUE,
  prefix        TEXT NOT NULL,                          -- first chars of the plaintext, for display
  user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  team_id       TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  label         TEXT NOT NULL,
  created_at    INTEGER NOT NULL,
  last_used_at  INTEGER,
  revoked_at    INTEGER
);
CREATE INDEX api_tokens_user_team ON api_tokens(user_id, team_id);

-- A named agent identity. Many MCP sessions over time may adopt the same agent (same human, same name).
CREATE TABLE agents (
  id                  TEXT PRIMARY KEY,                 -- agt_…
  team_id             TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  user_id             TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name                TEXT NOT NULL,                    -- agent-chosen, e.g. "ui-fixer"
  client              TEXT,                             -- e.g. "claude-code", "codex"
  working_on          TEXT,                             -- free-text status line
  client_session_key  TEXT,                             -- e.g. Claude Code session_id, linked via hello(session=…) for hooks
  created_at          INTEGER NOT NULL,
  last_seen_at        INTEGER NOT NULL,
  UNIQUE (team_id, user_id, name)
);
CREATE INDEX agents_team_seen ON agents(team_id, last_seen_at);
CREATE INDEX agents_session_key ON agents(client_session_key);

-- Streamable-HTTP MCP sessions (Mcp-Session-Id), persisted so restarts don't drop agents.
CREATE TABLE mcp_sessions (
  id              TEXT PRIMARY KEY,
  team_id         TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  token_id        TEXT REFERENCES api_tokens(id) ON DELETE CASCADE,
  agent_id        TEXT REFERENCES agents(id) ON DELETE SET NULL,
  client_name     TEXT,
  client_version  TEXT,
  protocol        TEXT,
  created_at      INTEGER NOT NULL,
  last_seen_at    INTEGER NOT NULL,
  closed_at       INTEGER
);
CREATE INDEX mcp_sessions_agent ON mcp_sessions(agent_id);

-- Boards are topic channels/tags: ui, backend, auth, llm, ci …
CREATE TABLE boards (
  id                TEXT PRIMARY KEY,                   -- brd_…
  team_id           TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  key               TEXT NOT NULL,                      -- lowercase slug [a-z0-9-]{1,24}
  name              TEXT NOT NULL,
  description       TEXT NOT NULL DEFAULT '',
  created_by_user   TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_by_agent  TEXT REFERENCES agents(id) ON DELETE SET NULL,
  created_at        INTEGER NOT NULL,
  archived_at       INTEGER,
  UNIQUE (team_id, key)
);

CREATE TABLE posts (
  id                   TEXT PRIMARY KEY,                -- pst_…
  team_id              TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  board_id             TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  number               INTEGER NOT NULL,                -- per-team sequence, shown as #12
  kind                 TEXT NOT NULL CHECK (kind IN ('task', 'note', 'discovery', 'question', 'decision')),
  status               TEXT NOT NULL CHECK (status IN ('open', 'in_progress', 'blocked', 'done', 'closed')),
  priority             INTEGER NOT NULL DEFAULT 0 CHECK (priority BETWEEN 0 AND 4), -- 0 none, 1 urgent, 2 high, 3 medium, 4 low
  title                TEXT NOT NULL,
  body                 TEXT NOT NULL DEFAULT '',         -- markdown (links, images)
  author_user_id       TEXT NOT NULL REFERENCES users(id),
  author_agent_id      TEXT REFERENCES agents(id) ON DELETE SET NULL,  -- NULL = written by the human in the dashboard
  claimed_by_agent_id  TEXT REFERENCES agents(id) ON DELETE SET NULL,
  claimed_by_user_id   TEXT REFERENCES users(id) ON DELETE SET NULL,   -- human claim from dashboard
  claimed_at           INTEGER,
  created_at           INTEGER NOT NULL,
  updated_at           INTEGER NOT NULL,
  UNIQUE (team_id, number)
);
CREATE INDEX posts_board ON posts(board_id, updated_at);
CREATE INDEX posts_team_updated ON posts(team_id, updated_at);
CREATE INDEX posts_claimed ON posts(claimed_by_agent_id);

CREATE TABLE comments (
  id               TEXT PRIMARY KEY,                    -- cmt_…
  team_id          TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  post_id          TEXT NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
  body             TEXT NOT NULL,
  author_user_id   TEXT NOT NULL REFERENCES users(id),
  author_agent_id  TEXT REFERENCES agents(id) ON DELETE SET NULL,
  created_at       INTEGER NOT NULL
);
CREATE INDEX comments_post ON comments(post_id, created_at);

-- A ping as sent. `target` is what the sender wrote: "paul/ui-fixer", "@paul", "paul/*", "@all".
CREATE TABLE pings (
  id              TEXT PRIMARY KEY,                     -- png_…
  team_id         TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  from_user_id    TEXT NOT NULL REFERENCES users(id),
  from_agent_id   TEXT REFERENCES agents(id) ON DELETE SET NULL,
  target          TEXT NOT NULL,
  post_id         TEXT REFERENCES posts(id) ON DELETE SET NULL,
  body            TEXT NOT NULL,
  created_at      INTEGER NOT NULL
);

-- Fan-out: one row per recipient (an agent, or a human's dashboard inbox).
CREATE TABLE inbox (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  ping_id       TEXT NOT NULL REFERENCES pings(id) ON DELETE CASCADE,
  team_id       TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  agent_id      TEXT REFERENCES agents(id) ON DELETE CASCADE,
  user_id       TEXT REFERENCES users(id) ON DELETE CASCADE,
  delivered_at  INTEGER,                                -- surfaced to the agent (tool piggyback / hook / inbox)
  read_at       INTEGER,
  CHECK ((agent_id IS NULL) <> (user_id IS NULL))
);
CREATE INDEX inbox_agent ON inbox(agent_id, delivered_at);
CREATE INDEX inbox_user ON inbox(user_id, team_id, read_at);

CREATE TABLE files (
  id                 TEXT PRIMARY KEY,                  -- unguessable; served at /f/<id>/<filename>
  team_id            TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  uploader_user_id   TEXT NOT NULL REFERENCES users(id),
  uploader_agent_id  TEXT REFERENCES agents(id) ON DELETE SET NULL,
  filename           TEXT NOT NULL,
  content_type       TEXT NOT NULL,
  size               INTEGER NOT NULL,
  created_at         INTEGER NOT NULL
);

-- One-time upload URLs (PUT /u/<code>) so agents can `curl -T file` without the bearer token.
CREATE TABLE upload_links (
  code        TEXT PRIMARY KEY,
  team_id     TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  agent_id    TEXT REFERENCES agents(id) ON DELETE SET NULL,
  expires_at  INTEGER NOT NULL,
  used_at     INTEGER
);

-- Activity log for the dashboard feed.
CREATE TABLE events (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  team_id         TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  at              INTEGER NOT NULL,
  actor_user_id   TEXT REFERENCES users(id) ON DELETE SET NULL,
  actor_agent_id  TEXT REFERENCES agents(id) ON DELETE SET NULL,
  kind            TEXT NOT NULL,   -- post_created | post_updated | status_changed | claimed | released | commented | pinged | board_created | member_joined | agent_joined
  post_id         TEXT REFERENCES posts(id) ON DELETE CASCADE,
  data            TEXT NOT NULL DEFAULT '{}'  -- small JSON payload
);
CREATE INDEX events_team ON events(team_id, id);
