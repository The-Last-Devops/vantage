#!/usr/bin/env bash
# Assert the API rejects junk input (400) and accepts clean input. Server-side
# validation is the real gate — the SPA can be bypassed. Self-cleaning.
#   bash scripts/check-validation.sh
set -uo pipefail
BASE="${BASE:-http://localhost:8080}"
EMAIL="${ADMIN_EMAIL:-admin@local}"
PASS="${ADMIN_PASSWORD:-admin123}"
JAR="$(mktemp)"; trap 'rm -f "$JAR"' EXIT
fail=0

for i in $(seq 1 60); do curl -s -o /dev/null -m 2 "$BASE/healthz" && break; sleep 1; done
curl -s -c "$JAR" -o /dev/null -X POST "$BASE/api/auth/login" \
  -H 'content-type: application/json' -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}"
WS=$(curl -s -b "$JAR" "$BASE/api/workspaces" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d[0]['id'] if d else '')")

# code POST <path> <json>  → prints HTTP status
code() { curl -s -b "$JAR" -o /dev/null -w '%{http_code}' -X "$1" "$BASE$2" -H 'content-type: application/json' -d "$3"; }
exp() { # exp <label> <want> <got>
  printf '%-46s ' "$1"
  if [ "$2" = "$3" ]; then echo "ok ($3)"; else echo "FAIL want $2 got $3"; fail=1; fi
}

# --- email (the original bug) ---
exp "reject email with spaces"        400 "$(code POST /api/users '{"email":"kiên béo ngu dốt @gmail.com","password":"secret123"}')"
exp "reject email no domain dot"      400 "$(code POST /api/users '{"email":"bob@localhost","password":"secret123"}')"
exp "reject short password"           400 "$(code POST /api/users '{"email":"ok@example.com","password":"x"}')"

# --- display names ---
exp "reject empty channel name"       400 "$(code POST "/api/workspaces/$WS/channels" '{"name":"   ","kind":"webhook","config":{"url":"https://e.com/x"}}')"
exp "reject control-char channel name" 400 "$(code POST "/api/workspaces/$WS/channels" '{"name":"bad\nname","kind":"webhook","config":{"url":"https://e.com/x"}}')"
exp "reject empty monitor name"       400 "$(code POST "/api/workspaces/$WS/monitors" '{"name":"","kind":"http","target":"https://e.com"}')"
exp "reject http monitor w/o target"  400 "$(code POST "/api/workspaces/$WS/monitors" '{"name":"probe","kind":"http","target":"  "}')"

# --- clean input still works (then clean up) ---
CH=$(curl -s -b "$JAR" -X POST "$BASE/api/workspaces/$WS/channels" -H 'content-type: application/json' \
  -d '{"name":"valid-name probe","kind":"webhook","config":{"url":"https://e.com/x"}}' | python3 -c "import sys,json
try: print(json.load(sys.stdin))
except: print('')")
printf '%-46s ' "accept a clean channel name"
[ -n "$CH" ] && { echo "ok ($CH)"; curl -s -b "$JAR" -o /dev/null -X DELETE "$BASE/api/channels/$CH"; } || { echo "FAIL"; fail=1; }

[ "$fail" -eq 0 ] && echo "OK" || { echo "validation gaps found"; exit 1; }
