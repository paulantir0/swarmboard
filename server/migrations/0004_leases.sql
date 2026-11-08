-- no-transaction
-- Leases: exclusive, auto-renewing locks on file paths/globs, shared resources ("ci", "deploy:staging")
-- or tasks ("#12"), with a FIFO wait queue. Also: agents get repo/branch, posts get the kinds
-- `blocker` and `handoff` (SQLite can't alter a CHECK, so posts is rebuilt with FKs off, which
-- needs to happen outside a transaction; the work itself runs in one explicit transaction).
PRAGMA foreign_keys = OFF;
BEGIN;

CREATE TABLE posts_new (
  id                   TEXT PRIMARY KEY,
  team_id              TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  board_id             TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  number               INTEGER NOT NULL,
  kind                 TEXT NOT NULL CHECK (kind IN ('task', 'note', 'discovery', 'question', 'decision', 'blocker', 'handoff')),
  status               TEXT NOT NULL CHECK (status IN ('open', 'in_progress', 'blocked', 'done', 'closed')),
  priority             INTEGER NOT NULL DEFAULT 0 CHECK (priority BETWEEN 0 AND 4),
  title                TEXT NOT NULL,
  body                 TEXT NOT NULL DEFAULT '',
  author_user_id       TEXT NOT NULL REFERENCES users(id),
  author_agent_id      TEXT REFERENCES agents(id) ON DELETE SET NULL,
  claimed_by_agent_id  TEXT REFERENCES agents(id) ON DELETE SET NULL,
  claimed_by_user_id   TEXT REFERENCES users(id) ON DELETE SET NULL,
  claimed_at           INTEGER,
  created_at           INTEGER NOT NULL,
  updated_at           INTEGER NOT NULL,
  UNIQUE (team_id, number)
);
INSERT INTO posts_new (id, team_id, board_id, number, kind, status, priority, title, body, author_user_id,
                       author_agent_id, claimed_by_agent_id, claimed_by_user_id, claimed_at, created_at, updated_at)
  SELECT id, team_id, board_id, number, kind, status, priority, title, body, author_user_id,
         author_agent_id, claimed_by_agent_id, claimed_by_user_id, claimed_at, created_at, updated_at FROM posts;
DROP TABLE posts;
ALTER TABLE posts_new RENAME TO posts;
CREATE INDEX posts_board ON posts(board_id, updated_at);
CREATE INDEX posts_team_updated ON posts(team_id, updated_at);
CREATE INDEX posts_claimed ON posts(claimed_by_agent_id);

-- Where the agent works: normalized git remote ("github.com/acme/app") or project name.
ALTER TABLE agents ADD COLUMN repo TEXT;
ALTER TABLE agents ADD COLUMN branch TEXT;

CREATE TABLE leases (
  id          TEXT PRIMARY KEY,                         -- lse_…
  team_id     TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  repo        TEXT NOT NULL DEFAULT '',                 -- '' for resources/tasks, or a path lease taken without a repo
  scope       TEXT NOT NULL,                            -- "server/src/auth/**", "ci", "#12"
  kind        TEXT NOT NULL CHECK (kind IN ('path', 'resource', 'task')),
  agent_id    TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  note        TEXT NOT NULL DEFAULT '',
  minutes     INTEGER NOT NULL,                         -- renew window
  created_at  INTEGER NOT NULL,
  expires_at  INTEGER NOT NULL
);
CREATE INDEX leases_team ON leases(team_id, expires_at);
CREATE INDEX leases_agent ON leases(agent_id);

-- FIFO queue for claim(wait:true).
CREATE TABLE lease_waits (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  team_id     TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  repo        TEXT NOT NULL DEFAULT '',
  scope       TEXT NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN ('path', 'resource', 'task')),
  agent_id    TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  note        TEXT NOT NULL DEFAULT '',
  minutes     INTEGER NOT NULL,
  created_at  INTEGER NOT NULL
);
CREATE INDEX lease_waits_team ON lease_waits(team_id, id);

COMMIT;
PRAGMA foreign_keys = ON;
