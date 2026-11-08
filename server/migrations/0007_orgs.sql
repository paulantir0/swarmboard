-- Orgs: every team gets a URL slug (swarmboard.app/<slug>), unique ignoring case.
-- teams is referenced by nearly every table, so instead of a rebuild the column is added
-- nullable, backfilled from the team name, and then enforced by a unique index (NOT NULL and the
-- basic shape via triggers). Format and reserved words are validated in Rust (accounts::validate_slug).
ALTER TABLE teams ADD COLUMN slug TEXT COLLATE NOCASE;

-- Slugify each name: ASCII-lowercase, runs of anything outside [a-z0-9] become one '-'.
CREATE TEMP TABLE org_slugs (id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, base TEXT NOT NULL, slug TEXT);
WITH RECURSIVE s(id, rest, out) AS (
  SELECT id, lower(name), '' FROM teams
  UNION ALL
  SELECT id, substr(rest, 2),
         CASE WHEN substr(rest, 1, 1) GLOB '[a-z0-9]' THEN out || substr(rest, 1, 1)
              WHEN out = '' OR out GLOB '*-' THEN out
              ELSE out || '-' END
  FROM s WHERE rest <> ''
)
INSERT INTO org_slugs (id, created_at, base)
  SELECT s.id, t.created_at, trim(substr(trim(s.out, '-'), 1, 28), '-')
  FROM s JOIN teams t ON t.id = s.id WHERE s.rest = '';
-- Too short or reserved bases get a usable stand-in.
UPDATE org_slugs SET base = 'team' WHERE length(base) < 2;
UPDATE org_slugs SET base = base || '-org' WHERE base IN (
  'app', 'api', 'auth', 'join', 'new', 'orgs', 'org', 'f', 'u', 'p', 'mcp', 'healthz', 'assets', 'brand', 'fonts',
  'crew', 'login', 'logout', 'signup', 'settings', 'admin', 'www', 'docs', 'help', 'about', 'terms', 'privacy',
  'static', 'invite', 'connect', 'github', 'slug');
-- Dedupe: the oldest team keeps the base, later ones get -2, -3, …
UPDATE org_slugs SET slug = (
  SELECT CASE WHEN r.n = 1 THEN r.base ELSE r.base || '-' || r.n END
  FROM (SELECT id, base, row_number() OVER (PARTITION BY base ORDER BY created_at, id) AS n FROM org_slugs) r
  WHERE r.id = org_slugs.id
);
-- A suffixed slug can still collide with another team's plain base ("a" x2 -> "a-2" vs a team named "a-2"):
-- the later of each colliding pair falls back to a piece of its id.
UPDATE org_slugs SET slug = substr(base, 1, 20) || '-' || lower(substr(id, 4, 8))
WHERE EXISTS (
  SELECT 1 FROM org_slugs o
  WHERE o.slug = org_slugs.slug AND (o.created_at < org_slugs.created_at OR (o.created_at = org_slugs.created_at AND o.id < org_slugs.id))
);
UPDATE teams SET slug = (SELECT slug FROM org_slugs WHERE org_slugs.id = teams.id);
DROP TABLE org_slugs;

CREATE UNIQUE INDEX teams_slug ON teams(slug);

CREATE TRIGGER teams_slug_insert BEFORE INSERT ON teams
WHEN NEW.slug IS NULL OR length(NEW.slug) NOT BETWEEN 2 AND 32 OR NEW.slug GLOB '*[^a-z0-9-]*'
BEGIN
  SELECT RAISE(ABORT, 'teams.slug must be 2-32 chars of [a-z0-9-]');
END;

CREATE TRIGGER teams_slug_update BEFORE UPDATE OF slug ON teams
WHEN NEW.slug IS NULL OR length(NEW.slug) NOT BETWEEN 2 AND 32 OR NEW.slug GLOB '*[^a-z0-9-]*'
BEGIN
  SELECT RAISE(ABORT, 'teams.slug must be 2-32 chars of [a-z0-9-]');
END;
