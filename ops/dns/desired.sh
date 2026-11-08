#!/usr/bin/env bash
# Print the desired DNS state as JSON: ops/dns/records.json, except that every (name, type) defined in
# ops/email/dns-records.json (written by ops/email/setup-resend.sh) is taken from that file instead.
# Names come out relative to the domain ("" = apex).
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
BASE=${DNS_RECORDS:-$HERE/records.json}
EMAIL=${EMAIL_RECORDS:-$HERE/../email/dns-records.json}

command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }

email='[]'
if [ -f "$EMAIL" ]; then
  email=$(jq -c 'if type == "array" then . else .records end' "$EMAIL")
fi

jq -c --argjson email "$email" '
  .domain as $d
  | def rel: ascii_downcase | rtrimstr(".") | if . == "@" or . == $d then "" else rtrimstr("." + $d) end;
    def clean: . + {type: (.type | ascii_upcase), name: ((.name // "") | rel)};
    ($email | map(clean)) as $e
  | .records = ([.records[] | clean | select(. as $r | any($e[]; .name == $r.name and .type == $r.type) | not)] + $e)
' "$BASE"
