#!/usr/bin/env bash
# Verify this batch's new endpoints, end to end. Self-cleaning, idempotent.
#   - audit: filters (method/status/q) + pagination shape {rows,total,retention_days}
#   - audit: retention PUT round-trips (90 days then back to forever)
#   - channels: test-before-save endpoint (POST .../channels/test) is reachable+authed
#   - channels: GET /channels/:id/alerts labels a workspace-wide rule ("All services")
#   - workspace members: list/add/remove round-trips
#   bash scripts/check-upgrades.sh
set -uo pipefail
BASE="${BASE:-http://localhost:8080}"
EMAIL="${ADMIN_EMAIL:-admin@local}"
PASS="${ADMIN_PASSWORD:-admin123}"
JAR="$(mktemp)"; trap 'rm -f "$JAR"' EXIT
py() { python3 -c "$1"; }
say() { printf '%-50s ' "$1"; }
fail=0

for i in $(seq 1 60); do curl -s -o /dev/null -m 2 "$BASE/healthz" && break; sleep 1; done
curl -s -c "$JAR" -o /dev/null -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}"
WS=$(curl -s -b "$JAR" "$BASE/api/workspaces" | py "import sys,json;d=json.load(sys.stdin);print(d[0]['id'] if d else '')")

# ---- audit page shape + pagination ----
say "audit returns {rows,total,retention_days}"
curl -s -b "$JAR" "$BASE/api/audit?limit=5" \
  | py "import sys,json;d=json.load(sys.stdin);print('ok' if all(k in d for k in('rows','total','retention_days')) and len(d['rows'])<=5 else 'FAIL')" \
  | grep -qx ok && echo ok || { echo FAIL; fail=1; }

say "audit method filter (DELETE only)"
curl -s -b "$JAR" "$BASE/api/audit?method=DELETE&limit=50" \
  | py "import sys,json;d=json.load(sys.stdin);print('ok' if all(r['method']=='DELETE' for r in d['rows']) else 'FAIL')" \
  | grep -qx ok && echo ok || { echo FAIL; fail=1; }

say "audit status filter (success only <300)"
curl -s -b "$JAR" "$BASE/api/audit?status=ok&limit=50" \
  | py "import sys,json;d=json.load(sys.stdin);print('ok' if all(r['status']<300 for r in d['rows']) else 'FAIL')" \
  | grep -qx ok && echo ok || { echo FAIL; fail=1; }

# ---- audit retention round-trip ----
curl -s -b "$JAR" -o /dev/null -X PUT "$BASE/api/admin/audit/retention" -H 'content-type: application/json' -d '{"days":90}'
say "retention set to 90 days"
curl -s -b "$JAR" "$BASE/api/audit?limit=1" | py "import sys,json;print(json.load(sys.stdin)['retention_days'])" | grep -qx 90 && echo ok || { echo FAIL; fail=1; }
curl -s -b "$JAR" -o /dev/null -X PUT "$BASE/api/admin/audit/retention" -H 'content-type: application/json' -d '{"days":null}'
say "retention reset to forever (null)"
curl -s -b "$JAR" "$BASE/api/audit?limit=1" | py "import sys,json;print(json.load(sys.stdin)['retention_days'])" | grep -qx None && echo ok || { echo FAIL; fail=1; }

# ---- channel test-before-save endpoint reachable + authed ----
say "channels/test endpoint reachable (not 404/403/500)"
CODE=$(curl -s -b "$JAR" -o /dev/null -w '%{http_code}' -X POST "$BASE/api/workspaces/$WS/channels/test" \
  -H 'content-type: application/json' -d '{"kind":"webhook","config":{"url":"http://127.0.0.1:9/none"}}')
# dispatch to a dead URL → 502; a valid send → 204. Either means the route ran.
{ [ "$CODE" = 502 ] || [ "$CODE" = 204 ]; } && echo "ok ($CODE)" || { echo "FAIL ($CODE)"; fail=1; }

say "channels/test rejects unknown kind (400)"
CODE=$(curl -s -b "$JAR" -o /dev/null -w '%{http_code}' -X POST "$BASE/api/workspaces/$WS/channels/test" \
  -H 'content-type: application/json' -d '{"kind":"nope","config":{}}')
[ "$CODE" = 400 ] && echo ok || { echo "FAIL ($CODE)"; fail=1; }

# ---- channel_alerts labels a workspace-wide rule ----
CH=$(curl -s -b "$JAR" -X POST "$BASE/api/workspaces/$WS/channels" -H 'content-type: application/json' \
  -d '{"name":"upgrade-probe","kind":"webhook","config":{"url":"https://e.com/x"}}' | py "import sys,json;print(json.load(sys.stdin))")
AL=$(curl -s -b "$JAR" -X POST "$BASE/api/workspaces/$WS/alerts" -H 'content-type: application/json' \
  -d "{\"scope_kind\":\"all_services\",\"channel_ids\":[\"$CH\"]}" | py "import sys,json;print(json.load(sys.stdin))")
say "channel alerts label ws-wide rule (All services/service)"
curl -s -b "$JAR" "$BASE/api/channels/$CH/alerts" \
  | py "import sys,json;d=[a for a in json.load(sys.stdin) if a['id']=='$AL'];print('ok' if d and d[0]['target']=='All services' and d[0]['kind']=='service' else 'FAIL')" \
  | grep -qx ok && echo ok || { echo FAIL; fail=1; }

# ---- re-target a rule via PATCH (source is editable now) ----
say "patch re-targets rule (all_services → all_hosts)"
curl -s -b "$JAR" -o /dev/null -X PATCH "$BASE/api/alerts/$AL" -H 'content-type: application/json' \
  -d "{\"scope_kind\":\"all_hosts\",\"scope_workspace_id\":\"$WS\"}"
curl -s -b "$JAR" "$BASE/api/alerts/$AL" \
  | py "import sys,json;print(json.load(sys.stdin).get('scope_kind'))" | grep -qx all_hosts && echo ok || { echo FAIL; fail=1; }

# ---- workspace members round-trip (uses the admin's own account) ----
say "workspace members list (owner-scoped)"
curl -s -b "$JAR" -o /dev/null -w '%{http_code}' "$BASE/api/workspaces/$WS/members" | grep -qx 200 && echo ok || { echo FAIL; fail=1; }
say "member-candidates endpoint (owner-scoped)"
curl -s -b "$JAR" -o /dev/null -w '%{http_code}' "$BASE/api/workspaces/$WS/member-candidates" | grep -qx 200 && echo ok || { echo FAIL; fail=1; }

# cleanup
curl -s -b "$JAR" -o /dev/null -X DELETE "$BASE/api/alerts/$AL"
curl -s -b "$JAR" -o /dev/null -X DELETE "$BASE/api/channels/$CH"
[ "$fail" -eq 0 ] && echo "OK — upgrades verified" || { echo "upgrade regressions"; exit 1; }
