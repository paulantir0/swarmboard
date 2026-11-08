#!/usr/bin/env bash
# Resend setup for swarmboard. Never prints API keys.
#
#   ops/email/setup-resend.sh          add/get the domain, turn tracking off, create the sending key,
#                                      write ops/email/dns-records.json
#   ops/email/setup-resend.sh verify   ask Resend to re-check DNS and print the domain status
#
# Needs a FULL-ACCESS Resend key in ~/.config/swarmboard/resend-admin.env:
#   RESEND_ADMIN_KEY=re_...
# Writes the sending-only key for the server to ~/.config/swarmboard/resend.env (mode 600):
#   RESEND_API_KEY=re_...
set -euo pipefail

DOMAIN=${DOMAIN:-swarmboard.app}
REGION=${REGION:-us-east-1}
KEY_NAME=${KEY_NAME:-swarmboard-api}
CFG=${XDG_CONFIG_HOME:-$HOME/.config}/swarmboard
ADMIN_ENV=$CFG/resend-admin.env
OUT_ENV=$CFG/resend.env
HERE=$(cd "$(dirname "$0")" && pwd)

command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }
[ -f "$ADMIN_ENV" ] || { echo "missing $ADMIN_ENV (RESEND_ADMIN_KEY=re_..., full access)" >&2; exit 1; }
KEY=$(sed -nE "s/^(export )?(RESEND_ADMIN_KEY|RESEND_API_KEY)=[\"']?([^\"' ]+).*/\3/p" "$ADMIN_ENV" | head -n1)
[ -n "$KEY" ] || { echo "no RESEND_ADMIN_KEY in $ADMIN_ENV" >&2; exit 1; }

# api METHOD PATH [JSON] -> response body on stdout; non-2xx prints Resend's message and fails.
# The key goes to curl via a config on stdin, so it never appears in argv or output.
api() {
  local method=$1 path=$2 data=${3-} out code
  local args=(-sS -m 30 -K - -X "$method" -w '\n%{http_code}')
  if [ -n "$data" ]; then args+=(-H 'Content-Type: application/json' --data-binary "$data"); fi
  out=$(printf 'header = "Authorization: Bearer %s"\n' "$KEY" | curl "${args[@]}" "https://api.resend.com$path")
  code=${out##*$'\n'}
  out=${out%$'\n'*}
  if [ "${code:0:1}" != 2 ]; then
    echo "resend $method $path -> $code: $(jq -r '[.name, .message] | map(select(. != null)) | join(": ")' <<<"$out" 2>/dev/null || echo "$out")" >&2
    return 1
  fi
  printf '%s' "$out"
}

domain_id() {
  api GET /domains | jq -r --arg d "$DOMAIN" '[.data[]? | select(.name == $d) | .id][0] // empty'
}

if [ "${1-}" = verify ]; then
  id=$(domain_id)
  [ -n "$id" ] || { echo "$DOMAIN is not in this Resend account yet; run without arguments first" >&2; exit 1; }
  api POST "/domains/$id/verify" >/dev/null
  api GET "/domains/$id" | jq '{id, name, status, records: [.records[] | {record, type, name, status}]}'
  exit 0
fi

# 1. Domain (create once, then reuse).
id=$(domain_id)
if [ -z "$id" ]; then
  dom=$(api POST /domains "$(jq -nc --arg n "$DOMAIN" --arg r "$REGION" '{name: $n, region: $r}')")
  id=$(jq -r .id <<<"$dom")
  echo "created domain $DOMAIN ($id)"
else
  echo "domain $DOMAIN already exists ($id)"
fi

# 2. Tracking off: open pixels and click rewriting would mangle magic links.
api PATCH "/domains/$id" '{"open_tracking": false, "click_tracking": false}' >/dev/null
echo "open/click tracking disabled"

# 3. Sending-only key restricted to this domain, for the server.
existing=$(api GET /api-keys | jq -r --arg n "$KEY_NAME" '[.data[]? | select(.name == $n) | .id][0] // empty')
if [ -n "$existing" ] && grep -q '^RESEND_API_KEY=re_' "$OUT_ENV" 2>/dev/null; then
  echo "reusing $KEY_NAME key ($existing) from $OUT_ENV"
else
  if [ -n "$existing" ]; then
    echo "note: a key named $KEY_NAME ($existing) exists but $OUT_ENV doesn't; creating a new one (delete the old one in the Resend dashboard)"
  fi
  created=$(api POST /api-keys "$(jq -nc --arg n "$KEY_NAME" --arg id "$id" '{name: $n, permission: "sending_access", domain_id: $id}')")
  mkdir -p "$CFG" && chmod 700 "$CFG"
  (umask 077 && jq -r '"RESEND_API_KEY=" + .token' <<<"$created" >"$OUT_ENV")
  chmod 600 "$OUT_ENV"
  echo "created $KEY_NAME key ($(jq -r .id <<<"$created")) -> $OUT_ENV"
fi

# 4. DNS records to publish at the registrar (names relative to $DOMAIN), plus DMARC.
dom=$(api GET "/domains/$id")
jq --arg d "$DOMAIN" '
  [ .records[]
    | {type, name: (.name | sub("\\.\($d | gsub("\\."; "\\."))$"; "")), content: .value, ttl: 600}
      + (if .priority != null then {prio: .priority} else {} end) ]
  + [ {type: "TXT", name: "_dmarc", content: "v=DMARC1; p=none;", ttl: 600} ]
' <<<"$dom" >"$HERE/dns-records.json"
echo "domain status: $(jq -r .status <<<"$dom")"
echo "wrote $HERE/dns-records.json:"
cat "$HERE/dns-records.json"
