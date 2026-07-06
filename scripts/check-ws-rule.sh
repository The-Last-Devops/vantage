#!/usr/bin/env bash
# Verify workspace-wide alert rules: create an "all services in workspace" rule
# (no specific target) and confirm it lists back with scope_kind set. Self-cleaning.
#   bash scripts/check-ws-rule.sh
set -uo pipefail
BASE="${BASE:-http://localhost:8080}"
EMAIL="${ADMIN_EMAIL:-admin@local}"
PASS="${ADMIN_PASSWORD:-admin123}"
JAR="$(mktemp)"; trap 'rm -f "$JAR"' EXIT
py() { python3 -c "$1"; }
say() { printf '%-44s ' "$1"; }
fail=0

for i in $(seq 1 60); do curl -s -o /dev/null -m 2 "$BASE/healthz" && break; sleep 1; done
curl -s -c "$JAR" -o /dev/null -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}"
WS=$(curl -s -b "$JAR" "$BASE/api/workspaces" | py "import sys,json;d=json.load(sys.stdin);print(d[0]['id'] if d else '')")

# a channel to attach
CH=$(curl -s -b "$JAR" -X POST "$BASE/api/workspaces/$WS/channels" -H 'content-type: application/json' \
  -d '{"name":"ws-rule-probe","kind":"webhook","config":{"url":"https://e.com/x"}}' | py "import sys,json;print(json.load(sys.stdin))")

# workspace-wide rule: all services, no monitor_id/system_id
AL=$(curl -s -b "$JAR" -X POST "$BASE/api/workspaces/$WS/alerts" -H 'content-type: application/json' \
  -d "{\"scope_kind\":\"all_services\",\"channel_ids\":[\"$CH\"]}" | py "import sys,json;print(json.load(sys.stdin))")
say "created workspace-wide rule"; [ -n "$AL" ] && echo "ok ($AL)" || { echo "FAIL"; fail=1; }

say "lists back with scope_kind=all_services"
curl -s -b "$JAR" "$BASE/api/workspaces/$WS/alerts" \
  | py "import sys,json;d=[a for a in json.load(sys.stdin) if a['id']=='$AL'];print(d[0].get('scope_kind') if d else 'MISSING')" \
  | grep -qx all_services && echo "ok" || { echo "FAIL"; fail=1; }

say "reject bad scope_kind (400)"
BADCODE=$(curl -s -b "$JAR" -o /dev/null -w '%{http_code}' -X POST "$BASE/api/workspaces/$WS/alerts" \
  -H 'content-type: application/json' -d "{\"scope_kind\":\"nope\",\"channel_ids\":[\"$CH\"]}")
[ "$BADCODE" = 400 ] && echo "ok" || { echo "FAIL ($BADCODE)"; fail=1; }

# cleanup
curl -s -b "$JAR" -o /dev/null -X DELETE "$BASE/api/alerts/$AL"
curl -s -b "$JAR" -o /dev/null -X DELETE "$BASE/api/channels/$CH"
[ "$fail" -eq 0 ] && echo "OK" || { echo "ws-rule regressions"; exit 1; }
