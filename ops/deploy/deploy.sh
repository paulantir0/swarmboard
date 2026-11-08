#!/usr/bin/env bash
# Deploy the swarmboard API to 0phone-runtime. Idempotent: safe to re-run.
#
#   ops/deploy/deploy.sh            # sync, build, (re)start, health check
#
# Remote layout:
#   /opt/swarmboard/.env          secrets + config (create once from server/.env.example; never synced)
#   /opt/swarmboard/compose.yml   from ops/deploy/compose.yml
#   /opt/swarmboard/src/          from server/ (minus target/, data/, .env)
#
# One-time on the host (not automated; shared proxy config):
#   append ops/deploy/Caddyfile.snippet to /opt/paulee/Caddyfile and reload Caddy.
set -euo pipefail

HOST="${DEPLOY_HOST:-0phone-runtime}"
REMOTE_DIR="${REMOTE_DIR:-/opt/swarmboard}"
NETWORK="${EDGE_NETWORK:-swarmboard-edge}"
PROXY_CONTAINER="${PROXY_CONTAINER:-paulee-proxy-1}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Deploy a clean worktree of the committed HEAD (or $1), so uncommitted edits in server/ never ship.
if [ -z "${SWARMBOARD_DEPLOY_WT:-}" ]; then
  WT="$(mktemp -d)"
  git -C "$REPO" worktree add -q --detach "$WT" "${1:-HEAD}"
  trap 'git -C "$REPO" worktree remove --force "$WT" >/dev/null 2>&1 || true; git -C "$REPO" worktree prune' EXIT
  SWARMBOARD_DEPLOY_WT=1 bash "$WT/ops/deploy/deploy.sh"
  exit $?
fi
ROOT="$REPO"

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }

say "checking $HOST:$REMOTE_DIR"
ssh "$HOST" "mkdir -p '$REMOTE_DIR/src'"
if ! ssh "$HOST" "test -s '$REMOTE_DIR/.env'"; then
  echo "error: $HOST:$REMOTE_DIR/.env is missing." >&2
  echo "       create it from server/.env.example (WEB_URL=https://swarmboard.app," >&2
  echo "       PUBLIC_API_URL=https://api.swarmboard.app, COOKIE_SECURE=true, RESEND_API_KEY=…)" >&2
  exit 1
fi

say "syncing server/ -> $HOST:$REMOTE_DIR/src"
rsync -az --delete \
  --exclude 'target/' \
  --exclude 'data/' \
  --include '.env.example' \
  --exclude '.env' \
  --exclude '.env.*' \
  --exclude '*.db' --exclude '*.db-wal' --exclude '*.db-shm' \
  "$ROOT/server/" "$HOST:$REMOTE_DIR/src/"
rsync -az "$ROOT/ops/deploy/compose.yml" "$HOST:$REMOTE_DIR/compose.yml"

say "ensuring docker network $NETWORK (and that $PROXY_CONTAINER is on it)"
ssh "$HOST" "
  set -e
  docker network inspect '$NETWORK' >/dev/null 2>&1 || docker network create '$NETWORK' >/dev/null
  if docker inspect '$PROXY_CONTAINER' >/dev/null 2>&1; then
    docker inspect -f '{{range \$k, \$v := .NetworkSettings.Networks}}{{\$k}} {{end}}' '$PROXY_CONTAINER' | grep -qw '$NETWORK' \
      || docker network connect '$NETWORK' '$PROXY_CONTAINER'
  else
    echo 'warning: proxy container $PROXY_CONTAINER not found; connect it to $NETWORK yourself' >&2
  fi
"

say "building and starting swarmboard-api"
ssh "$HOST" "cd '$REMOTE_DIR' && docker compose -f compose.yml build && docker compose -f compose.yml up -d --remove-orphans"

say "waiting for /healthz"
for _ in $(seq 1 30); do
  if ssh "$HOST" "docker exec swarmboard-api curl -fsS http://127.0.0.1:8191/healthz" >/dev/null 2>&1; then
    say "swarmboard-api is healthy"
    if ! ssh "$HOST" "grep -q 'api.swarmboard.app' /opt/paulee/Caddyfile 2>/dev/null"; then
      echo "note: api.swarmboard.app is not in /opt/paulee/Caddyfile yet; add ops/deploy/Caddyfile.snippet and reload Caddy." >&2
    fi
    exit 0
  fi
  sleep 2
done
echo "error: swarmboard-api did not become healthy; recent logs:" >&2
ssh "$HOST" "docker logs --tail 50 swarmboard-api" >&2 || true
exit 1
