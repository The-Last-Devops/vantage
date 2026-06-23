#!/usr/bin/env bash
# Broad read-side smoke test: log in and hit every main API endpoint, printing a
# count/sample so you can eyeball that each feature works.
#   bash scripts/check-features.sh
set -euo pipefail
BASE="${BASE:-http://localhost:8080}"
EMAIL="${ADMIN_EMAIL:-admin@local}"
PASS="${ADMIN_PASSWORD:-admin123}"
JAR="$(mktemp)"; trap 'rm -f "$JAR"' EXIT
curl -fsS -c "$JAR" -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}" >/dev/null

g() { curl -fsS -b "$JAR" "$BASE$1"; }
count() { python3 -c "import sys,json; d=json.load(sys.stdin); print(len(d) if isinstance(d,list) else 'ok')"; }

printf '%-28s ' "systems";        g /api/systems | count
printf '%-28s ' "fleet?range=6h"; g "/api/fleet?range=6h" | python3 -c "import sys,json;d=json.load(sys.stdin);print(len(d.get('hosts',d) if isinstance(d,dict) else d))"
printf '%-28s ' "monitors";       g /api/monitors | count
printf '%-28s ' "namespaces";     g /api/namespaces | count
printf '%-28s ' "channels(ns0)";  NS=$(g /api/namespaces | python3 -c "import sys,json;d=json.load(sys.stdin);print(d[0]['id'] if d else '')"); [ -n "$NS" ] && g "/api/namespaces/$NS/channels" | count || echo "no ns"
printf '%-28s ' "alerts(ns0)";    [ -n "$NS" ] && g "/api/namespaces/$NS/alerts" | count || echo "no ns"
printf '%-28s ' "audit";          g /api/audit | count
printf '%-28s ' "about";          g /api/about | python3 -c "import sys,json;d=json.load(sys.stdin);print(d['version'],d['git_sha'])"
printf '%-28s ' "admin/data";     g /api/admin/data | python3 -c "import sys,json;d=json.load(sys.stdin);print('size',d['db_size'],'| tiers',len(d['retention']))"
printf '%-28s ' "users";          g /api/users | count

MID=$(g /api/monitors | python3 -c "import sys,json;d=json.load(sys.stdin);print(d[0]['id'] if d else '')")
if [ -n "$MID" ]; then
  printf '%-28s ' "monitor detail"; g "/api/monitors/$MID" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d['name'],'| up',d['up'],'| 24h',d['uptime_24h'])"
  printf '%-28s ' "monitor heartbeats"; g "/api/monitors/$MID/heartbeats?range=24h" | python3 -c "import sys,json;d=json.load(sys.stdin);print('points',len(d['t']))"
fi
SID=$(g /api/systems | python3 -c "import sys,json;d=json.load(sys.stdin);print(d[0]['id'] if d else '')")
if [ -n "$SID" ]; then
  printf '%-28s ' "system metrics 6h"; g "/api/systems/$SID/metrics?range=6h" | python3 -c "import sys,json;d=json.load(sys.stdin);print('points',len(d['t']))"
  printf '%-28s ' "system containers"; g "/api/systems/$SID/containers" | python3 -c "import sys,json;d=json.load(sys.stdin);print('series', len(d.get('cpu',[])))"
fi
echo "OK"
