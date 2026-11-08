#!/usr/bin/env bash
# Deploy the web app (web/) to Vercel production (project "swarmboard", team acta30n).
# Always deploys from web/: a stray .vercel link at the repo root makes the CLI upload the whole repo.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
rm -rf "$ROOT/.vercel"
cd "$ROOT/web"
npx -y vercel@latest --scope acta30n deploy --prod --yes
