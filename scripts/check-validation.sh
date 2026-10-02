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

# --- alert rule conditions ---
# `condition` is stored as raw JSON, so an unknown metric used to be accepted and then
# silently never evaluated: the rule rendered perfectly, offered a Test button, and did
# nothing. A rule that is believed and does not fire is worse than no rule. These need a
# channel to attach to, since a rule without one is rejected first.
VCH=$(curl -s -b "$JAR" -X POST "$BASE/api/workspaces/$WS/channels" -H 'content-type: application/json' \
  -d '{"name":"cond probe","kind":"webhook","config":{"url":"https://e.com/x"}}' | python3 -c "import sys,json
try: print(json.load(sys.stdin))
except: print('')")
SYS=$(curl -s -b "$JAR" "$BASE/api/systems" | python3 -c "import sys,json
try:
    d=json.load(sys.stdin); print(d[0]['id'] if d else '')
except Exception: print('')")
if [ -n "$VCH" ] && [ -n "$SYS" ]; then
  mk() { printf '{"system_id":"%s","channel_ids":["%s"],"condition":%s}' "$SYS" "$VCH" "$1"; }
  exp "reject unknown alert metric"     400 "$(code POST "/api/workspaces/$WS/alerts" "$(mk '{"metric":"nonsense_percent","op":">","value":9}')")"
  exp "reject misspelled disk metric"   400 "$(code POST "/api/workspaces/$WS/alerts" "$(mk '{"metric":"disk_usage","op":">","value":9}')")"
  AID=$(curl -s -b "$JAR" -X POST "$BASE/api/workspaces/$WS/alerts" -H 'content-type: application/json' \
    -d "$(mk '{"metric":"disk_percent","op":">","value":85}')" | python3 -c "import sys,json
try: print(json.load(sys.stdin))
except: print('')")
  printf '%-46s ' "accept disk_percent"
  [ -n "$AID" ] && { echo "ok ($AID)"; curl -s -b "$JAR" -o /dev/null -X DELETE "$BASE/api/alerts/$AID"; } || { echo "FAIL"; fail=1; }
  curl -s -b "$JAR" -o /dev/null -X DELETE "$BASE/api/channels/$VCH"
else
  echo "skip alert-condition checks (no channel or no system to attach to)"
fi

# --- clean input still works (then clean up) ---
CH=$(curl -s -b "$JAR" -X POST "$BASE/api/workspaces/$WS/channels" -H 'content-type: application/json' \
  -d '{"name":"valid-name probe","kind":"webhook","config":{"url":"https://e.com/x"}}' | python3 -c "import sys,json
try: print(json.load(sys.stdin))
except: print('')")
printf '%-46s ' "accept a clean channel name"
[ -n "$CH" ] && { echo "ok ($CH)"; curl -s -b "$JAR" -o /dev/null -X DELETE "$BASE/api/channels/$CH"; } || { echo "FAIL"; fail=1; }

[ "$fail" -eq 0 ] && echo "OK" || { echo "validation gaps found"; exit 1; }
