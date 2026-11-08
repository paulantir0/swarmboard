-- Team roles: owner | admin | member. SQLite can't alter a CHECK, so memberships is rebuilt.
-- No table references memberships by foreign key, so it can be dropped with FKs on (inside the
-- migration's transaction); its own FKs to teams/users are recreated as they were.
CREATE TABLE memberships_new (
  team_id    TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role       TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
  joined_at  INTEGER NOT NULL,
  PRIMARY KEY (team_id, user_id)
);
INSERT INTO memberships_new (team_id, user_id, role, joined_at)
  SELECT team_id, user_id, role, joined_at FROM memberships;
DROP TABLE memberships;
ALTER TABLE memberships_new RENAME TO memberships;
CREATE INDEX memberships_user ON memberships(user_id);
-- Exactly one owner per team (transfers demote the old owner before promoting the new one).
CREATE UNIQUE INDEX memberships_one_owner ON memberships(team_id) WHERE role = 'owner';
