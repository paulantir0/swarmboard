#!/usr/bin/env bash
# Deploy the web app to Vercel production (project "swarmboard", team acta30n).
# Deploys a clean worktree of the committed HEAD, so uncommitted edits in web/ never ship,
# and always from web/ (a .vercel link at the repo root makes the CLI upload the whole repo).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REF="${1:-HEAD}"
WT="$(mktemp -d)"
trap 'git -C "$ROOT" worktree remove --force "$WT" >/dev/null 2>&1 || true; git -C "$ROOT" worktree prune' EXIT
git -C "$ROOT" worktree add -q --detach "$WT" "$REF"
mkdir -p "$WT/web/.vercel"
cp "$ROOT/web/.vercel/project.json" "$WT/web/.vercel/project.json"
cd "$WT/web"
npx -y vercel@latest --scope acta30n deploy --prod --yes
