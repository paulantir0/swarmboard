#!/usr/bin/env bash
# Make the swarmboard.app zone at Porkbun match ops/dns/desired.sh (records.json + ops/email/dns-records.json).
#
#   ops/dns/porkbun-apply.sh             dry run (default): print the plan, change nothing
#   ops/dns/porkbun-apply.sh --apply     make the changes, then re-read the zone
#
# Credentials: PORKBUN_API_KEY and PORKBUN_SECRET_API_KEY from the environment, else from
# ~/.config/swarmboard/porkbun.env. The domain needs "API Access" switched on in the Porkbun dashboard.
#
# What it touches: the records listed in records.json (created, or edited in place), records at the apex and
# www that conflict with them (AAAA/ALIAS/CNAME/other A), Porkbun parking defaults (pixie.porkbun.com), and
# URL forwards at the apex/www. Everything else, including NS and any apex MX (email forwarding), is left
# alone and listed as "keep". Keys are never printed and never appear in argv.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
API=https://api.porkbun.com/api/json/v3
MODE=dry-run
case "${1-}" in
  "" | --dry-run) ;;
  --apply) MODE=apply ;;
  -h | --help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
  *) echo "usage: $0 [--dry-run|--apply]" >&2; exit 2 ;;
esac

command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }
command -v curl >/dev/null || { echo "curl is required" >&2; exit 1; }

ENV_FILE=${XDG_CONFIG_HOME:-$HOME/.config}/swarmboard/porkbun.env
envval() { [ -f "$ENV_FILE" ] && sed -nE "s/^(export )?$1=[\"']?([^\"' ]+).*/\2/p" "$ENV_FILE" | head -n1; }
PORKBUN_API_KEY=${PORKBUN_API_KEY:-$(envval PORKBUN_API_KEY || true)}
PORKBUN_SECRET_API_KEY=${PORKBUN_SECRET_API_KEY:-$(envval PORKBUN_SECRET_API_KEY || true)}
if [ -z "$PORKBUN_API_KEY" ] || [ -z "$PORKBUN_SECRET_API_KEY" ]; then
  echo "need PORKBUN_API_KEY and PORKBUN_SECRET_API_KEY (env, or $ENV_FILE)" >&2
  exit 1
fi
export PORKBUN_API_KEY PORKBUN_SECRET_API_KEY

# api PATH [JSON-FIELDS] -> response JSON on stdout. The keys are added by jq from the environment and sent in
# the request body on stdin. Non-SUCCESS prints Porkbun's message and returns 1.
api() {
  local path=$1 fields=${2:-} out
  [ -n "$fields" ] || fields='{}'
  if ! out=$(jq -nc --argjson f "$fields" '{apikey: env.PORKBUN_API_KEY, secretapikey: env.PORKBUN_SECRET_API_KEY} + $f' |
    curl -sS -m 30 -X POST -H 'Content-Type: application/json' --data-binary @- "$API/$path"); then
    echo "porkbun $path: request failed" >&2
    return 1
  fi
  if [ "$(jq -r '.status // empty' <<<"$out" 2>/dev/null)" != SUCCESS ]; then
    echo "porkbun $path: $(jq -r '.message // "unexpected response"' <<<"$out" 2>/dev/null || printf '%.200s' "$out")" >&2
    return 1
  fi
  printf '%s' "$out"
}

desired=$("$HERE/desired.sh")
DOMAIN=$(jq -r .domain <<<"$desired")

# Inputs: $desired {domain, ttl, records}, $zone (dns/retrieve records), $fwd (URL forwards).
# Output: ordered list of actions {op, w (desired), z (zone record), why}.
# shellcheck disable=SC2016
PLAN_JQ='
def host: ascii_downcase | rtrimstr(".");
def unquote: if length > 1 and startswith("\"") and endswith("\"") then .[1:-1] else . end;
def normc($t): if . == null then null
  elif ($t | IN("CNAME", "ALIAS", "MX", "NS")) then host
  elif $t == "TXT" then unquote
  else . end;
# TXT records that occupy the same "slot": one SPF, one DMARC, one DKIM key per name.
def fam: if . == null then "dkim"
  elif test("^v=spf1(\\s|$)"; "i") then "spf"
  elif test("^v=dmarc1"; "i") then "dmarc"
  elif test("^(v=dkim1|k=|p=)"; "i") then "dkim"
  else "txt:" + . end;
def addr: IN("A", "AAAA", "ALIAS", "CNAME");
# pixie = Porkbun parking page (default ALIAS @ and CNAME *); uixie = Porkbun URL forwarding.
def parking: (.type | IN("A", "ALIAS", "CNAME")) and (
    (.c | IN("pixie.porkbun.com", "207.207.210.107", "207.207.210.229"))
    or ((.c | IN("uixie.porkbun.com", "207.207.210.23", "207.207.210.36", "207.207.210.50")) and (.n | IN("", "www", "*"))));

$desired.domain as $d
| def rel: ascii_downcase | rtrimstr(".") | if . == $d or . == "@" then "" else rtrimstr("." + $d) end;
  [ $zone[] | .type as $t
    | . + {n: (.name | rel), c: (.content | normc($t)), p: ((.prio // "0") | tonumber? // 0)} ] as $z
| [ $desired.records[] | .type as $t
    | . + {n: .name, c: (.content | normc($t)), p: ((.prio // 0) | tonumber), ttl: (.ttl // $desired.ttl // 600)} ] as $want

# One desired record -> its main action (ok/edit/create/pending) plus conflict deletes/warnings.
| def plan($w):
    ($z | map(select(.n == $w.n and .type != "NS"))) as $at
    | ($w.n | IN("", "www")) as $vercel
    | (if $w.type == "TXT" then ($w.c | fam) else null end) as $f
    | ($at | map(select(.type == $w.type and (if $f then (.c | fam) == $f else true end)))) as $same
    | (if $w.type == "MX" then ($same | map(select(.c == $w.c and .p == $w.p))) else ($same | map(select(.c == $w.c))) end) as $exact
    | (if $w.c == null then {op: "pending", w: $w, z: $same[0]}
       elif ($exact | length) > 0 then {op: "ok", w: $w, z: $exact[0]}
       elif $w.type == "MX" then
         (($same | map(select(.c == $w.c)))[0]) as $m
         | if $m then {op: "edit", w: $w, z: $m, why: "priority"} else {op: "create", w: $w} end
       elif ($same | length) > 0 then {op: "edit", w: $w, z: $same[0]}
       else {op: "create", w: $w} end) as $main
    | ($main.z.id // "") as $keep
    | [ $main ]
    + [ $at[] | select(.id != $keep) as $r
        | if $w.type == "CNAME" or (($w.type == "A") and (.type | IN("A", "ALIAS", "CNAME"))) or ($vercel and $w.type == "A" and .type == "AAAA") then
            if $vercel and (.type | addr) then {op: "delete", z: ., why: "conflicts with \($w.type) \($w.c)"}
            elif parking then empty
            else {op: "warn", z: ., why: "conflicts with \($w.type) \($w.c); left in place, remove it by hand"} end
          elif $w.type == "A" and .type == "AAAA" then {op: "warn", z: ., why: "AAAA next to A \($w.c); check it is right"}
          elif $w.type == "TXT" and .type == "TXT" and $f and (.c | fam) == $f then {op: "warn", z: ., why: "second \($f) TXT at this name; remove it by hand"}
          else empty end ];

  [ $want[] | plan(.) ] | flatten as $acts
| [ $acts[] | select(.op | IN("ok", "edit", "pending")) | .z.id // empty ] as $claimed
| ( [ $acts[] | select(.op == "delete") ]
    + [ $z[] | select(parking and (.id | IN($claimed[]) | not)) | {op: "delete", z: ., why: "Porkbun parking default"} ]
  ) | unique_by(.z.id) as $dels
| [ $dels[].z.id ] as $deleted
| ([ $acts[] | select(.op == "warn" and (.z.id | IN($deleted[]) | not)) ] | unique_by(.z.id)) as $warns
| [ $warns[].z.id ] as $warned
| [ $fwd[] | select((.subdomain // "") | IN("", "www")) | {op: "delete_forward", f: ., why: "URL forward conflicts with the Vercel records"} ]
  + $dels
  + [ $acts[] | select(.op == "edit") ]
  + [ $acts[] | select(.op == "create") ]
  + [ $acts[] | select(.op == "ok") ]
  + [ $acts[] | select(.op == "pending") ]
  + $warns
  + [ $fwd[] | select((.subdomain // "") | IN("", "www") | not) | {op: "keep_forward", f: .} ]
  + [ $z[] | select(.type != "NS" and (.id | IN($claimed[], $deleted[], $warned[]) | not)) | {op: "keep", z: .} ]
'

# shellcheck disable=SC2016
SHOW_JQ='
def nm: if . == "" then "@" else . end;
def val: if .type == "MX" then "\(.p) \(.c)" else (.c // "") end;
def note: if . then "(\(.))" else "" end;
.[] | (if .op == "delete_forward" then ["delete", "FWD", (.f.subdomain // "" | nm), "-> \(.f.location)", (.why | note)]
  elif .op == "keep_forward" then ["keep", "FWD", (.f.subdomain // "" | nm), "-> \(.f.location)", ""]
  elif .op == "delete" then ["delete", .z.type, (.z.n | nm), (.z | val), (.why | note)]
  elif .op == "edit" then ["edit", .w.type, (.w.n | nm), "\(.z | val)  ->  \(.w | val)", ""]
  elif .op == "create" then ["create", .w.type, (.w.n | nm), (.w | val), ""]
  elif .op == "ok" then ["ok", .w.type, (.w.n | nm), (.w | val), ""]
  elif .op == "pending" then ["pending", .w.type, (.w.n | nm), (if .z then "zone has \(.z.c[0:24])..." else "no value yet" end), (.w.pending | note)]
  elif .op == "warn" then ["WARN", .z.type, (.z.n | nm), (.z | val), (.why | note)]
  else ["keep", .z.type, (.z.n | nm), (.z | val), ""] end) | "  " + join("\t")
'

make_plan() {
  local zone fwd
  zone=$(api "dns/retrieve/$DOMAIN" | jq -c '.records // []') || {
    echo "could not read the $DOMAIN zone (is API Access on for this domain at porkbun.com?)" >&2
    return 1
  }
  fwd=$(api "domain/getUrlForwarding/$DOMAIN" | jq -c '.forwards // []') || fwd='[]'
  jq -nc --argjson desired "$desired" --argjson zone "$zone" --argjson fwd "$fwd" "$PLAN_JQ"
}

show() { jq -r "$SHOW_JQ" <<<"$1" | column -t -s $'\t'; }
changes() { jq '[.[] | select(.op | IN("delete", "delete_forward", "edit", "create"))] | length' <<<"$1"; }

plan=$(make_plan)
echo "porkbun $DOMAIN ($MODE)"
show "$plan"
n=$(changes "$plan")

if [ "$MODE" = dry-run ]; then
  if [ "$n" -eq 0 ]; then echo "nothing to change"; else echo "$n change(s); run with --apply to make them"; fi
  exit 0
fi
[ "$n" -gt 0 ] || { echo "nothing to change"; exit 0; }

# Deletes first (Porkbun refuses an A next to an ALIAS/CNAME at the same name), then edits, then creates.
fail=0
while IFS= read -r a; do
  op=$(jq -r .op <<<"$a")
  body=$(jq -c '.w | {name: .n, type, content: .c, ttl: (.ttl | tostring)} + (if .type == "MX" then {prio: (.p | tostring)} else {} end)' <<<"$a" 2>/dev/null || echo '{}')
  case $op in
    delete_forward) path="domain/deleteUrlForward/$DOMAIN/$(jq -r .f.id <<<"$a")"; body='{}' ;;
    delete) path="dns/delete/$DOMAIN/$(jq -r .z.id <<<"$a")"; body='{}' ;;
    edit) path="dns/edit/$DOMAIN/$(jq -r .z.id <<<"$a")" ;;
    create) path="dns/create/$DOMAIN" ;;
  esac
  label=$(jq -r '[.op, (.w.type // .z.type // "FWD"), ((.w.n // .z.n // .f.subdomain // "") | if . == "" then "@" else . end)] | join(" ")' <<<"$a")
  if api "$path" "$body" >/dev/null; then echo "done: $label"; else echo "FAILED: $label" >&2; fail=1; fi
done < <(jq -c '.[] | select(.op | IN("delete_forward", "delete", "edit", "create"))' <<<"$plan")

echo
echo "zone after apply:"
plan=$(make_plan)
show "$plan"
n=$(changes "$plan")
if [ "$n" -eq 0 ] && [ "$fail" -eq 0 ]; then
  echo "zone matches the desired records (pending ones aside); run ops/dns/check.sh to watch propagation"
else
  echo "$n change(s) still outstanding" >&2
  exit 1
fi
