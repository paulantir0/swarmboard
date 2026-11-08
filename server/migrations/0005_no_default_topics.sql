-- New teams no longer get default topics (general, ui, backend, ci). Remove the ones
-- existing teams got at signup if nothing was ever posted to them.
DELETE FROM boards
WHERE key IN ('general', 'ui', 'backend', 'ci')
  AND created_by_agent IS NULL
  AND NOT EXISTS (SELECT 1 FROM posts WHERE posts.board_id = boards.id)
  AND EXISTS (SELECT 1 FROM teams WHERE teams.id = boards.team_id AND boards.created_at = teams.created_at);
