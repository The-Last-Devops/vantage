#!/usr/bin/env bash
# Verify the audit log records a human-readable object name (not just the API
# path). Creates a uniquely-named channel, deletes it, then asserts the audit
# feed shows the DELETE with that name as the object. Self-cleaning.
#   bash scripts/check-audit.sh
set -uo pipefail
BASE="${BASE:-http://localhost:8080}"
EMAIL="${ADMIN_EMAIL:-admin@local}"
PASS="${ADMIN_PASSWORD:-admin123}"
NAME="audit-probe-channel"
JAR="$(mktemp)"; trap 'rm -f "$JAR"' EXIT
py() { python3 -c "$1"; }
say() { printf '%-30s ' "$1"; }

# wait for the hub (it may still be compiling/applying migrations)
for i in $(seq 1 60); do curl -s -o /dev/null -m 2 "$BASE/healthz" && break; sleep 1; done

curl -s -c "$JAR" -o /dev/null -X POST "$BASE/api/auth/login" \
  -H 'content-type: application/json' -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}"
WS=$(curl -s -b "$JAR" "$BASE/api/workspaces" | py "import sys,json;d=json.load(sys.stdin);print(d[0]['id'] if d else '')")
say "workspace0"; echo "$WS"; [ -n "$WS" ] || { echo "no workspace"; exit 1; }

CH=$(curl -s -b "$JAR" -X POST "$BASE/api/workspaces/$WS/channels" -H 'content-type: application/json' \
  -d "{\"name\":\"$NAME\",\"kind\":\"webhook\",\"config\":{\"url\":\"https://example.com/x\"}}" | py "import sys,json;print(json.load(sys.stdin))")
say "created channel"; echo "$CH"
curl -s -b "$JAR" -o /dev/null -X DELETE "$BASE/api/channels/$CH"
say "deleted channel"; echo "$CH"

# the DELETE row for that channel id must carry object_name == NAME
say "audit shows object name"
curl -s -b "$JAR" "$BASE/api/audit" | py "
import sys,json
rows=json.load(sys.stdin)['rows']
hit=[r for r in rows if r['method']=='DELETE' and '$CH' in r['path']]
ok = hit and hit[0].get('object_name')=='$NAME'
print('OK ('+hit[0].get('object_name','?')+')' if ok else 'FAIL: '+json.dumps(hit[:1]))
sys.exit(0 if ok else 1)
"
