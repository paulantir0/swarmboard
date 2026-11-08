#!/usr/bin/env bash
# Look up every desired record (ops/dns/desired.sh) on a Porkbun authoritative nameserver and on 1.1.1.1.
#
#   ops/dns/check.sh                 default servers: the zone's first Porkbun NS, then 1.1.1.1
#   ops/dns/check.sh 8.8.8.8 ...     use these servers instead
#
# Prints OK / MISSING / WRONG / PENDING / NOREPLY per record and server. Exits 1 if anything is MISSING, WRONG
# or NOREPLY; PENDING (no value known yet, e.g. the Resend DKIM key) does not fail.
# A public resolver can keep serving an old or missing answer until its cached TTL runs out (up to ~1h here).
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
command -v dig >/dev/null || { echo "dig is required (dnsutils / bind-utils)" >&2; exit 1; }
command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }

desired=$("$HERE/desired.sh")
DOMAIN=$(jq -r .domain <<<"$desired")

if [ $# -gt 0 ]; then
  servers=("$@")
else
  ns=$(dig +short +time=3 +tries=2 NS "$DOMAIN" @1.1.1.1 2>/dev/null | grep -v '^;' | sed 's/\.$//' | sort | head -n1 || true)
  servers=("${ns:-curitiba.ns.porkbun.com}" 1.1.1.1)
fi

# Same TXT "slot" rules as porkbun-apply.sh: one SPF / DMARC / DKIM value per name.
fam() {
  case $1 in
    [vV]=[sS][pP][fF]1 | [vV]=[sS][pP][fF]1\ *) echo spf ;;
    [vV]=[dD][mM][aA][rR][cC]1*) echo dmarc ;;
    [vV]=[dD][kK][iI][mM]1* | k=* | p=*) echo dkim ;;
    *) echo "txt:$1" ;;
  esac
}

# lookup SERVER FQDN TYPE -> normalised answers, one per line; returns 9 on no reply.
lookup() {
  local out
  out=$(dig +short +time=3 +tries=2 "$2" "$3" @"$1" 2>/dev/null) || return 9
  out=$(grep -v '^;' <<<"$out" || true)
  case $3 in
    TXT) sed -E 's/" "//g; s/^"//; s/"$//' <<<"$out" ;;
    MX) awk 'NF == 2 { sub(/\.$/, "", $2); print $1 " " tolower($2) }' <<<"$out" ;;
    # dig +short also prints the CNAME target for an A query; keep only answers of the asked type.
    A) grep -E '^[0-9.]+$' <<<"$out" || true ;;
    *) sed 's/\.$//' <<<"$out" | tr 'A-Z' 'a-z' ;;
  esac
}

printf '%-6s %-32s' TYPE NAME
for s in "${servers[@]}"; do printf ' %-26s' "$s"; done
printf ' %s\n' WANT

bad=0
details=()
while IFS=$'\t' read -r type name want; do
  fqdn=$DOMAIN
  [ "$name" = @ ] || fqdn="$name.$DOMAIN"
  [ "$type" = TXT ] || want=$(tr 'A-Z' 'a-z' <<<"$want" | sed 's/\.$//')
  printf '%-6s %-32s' "$type" "$fqdn"
  for s in "${servers[@]}"; do
    rc=0
    got=$(lookup "$s" "$fqdn" "$type") || rc=$?
    if [ "$rc" -ne 0 ]; then
      st=NOREPLY
    else
      if [ "$type" = TXT ]; then
        f=$( [ -n "$want" ] && fam "$want" || echo dkim)
        slot=$(while IFS= read -r v; do [ -n "$v" ] && [ "$(fam "$v")" = "$f" ] && printf '%s\n' "$v"; done <<<"$got" || true)
      else
        slot=$got
      fi
      if [ -z "$want" ]; then
        st=PENDING
        [ -z "$slot" ] || st="PENDING(live)"
      elif [ -z "$slot" ]; then
        st=MISSING
      elif grep -qxF -- "$want" <<<"$slot"; then
        st=OK
      else
        st=WRONG
        details+=("$type $fqdn @$s: want '$want', got '$(paste -sd ',' <<<"$slot")'")
      fi
    fi
    case $st in MISSING | WRONG | NOREPLY) bad=1 ;; esac
    printf ' %-26s' "$st"
  done
  if [ -n "$want" ]; then printf ' %s\n' "$want"; else printf ' %s\n' "(no value yet)"; fi
done < <(jq -r '.records[] | [.type, (if .name == "" then "@" else .name end), (if .type == "MX" then "\(.prio // 0) \(.content // "")" else (.content // "") end)] | @tsv' <<<"$desired" |
  sed -E $'s/\t(0|[0-9]+) $/\t/')

for d in "${details[@]}"; do echo "  $d"; done
exit "$bad"
