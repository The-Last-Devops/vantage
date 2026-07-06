#!/usr/bin/env bash
# Smoke-test the multi-channel alerting feature against a running dev hub.
# Logs in, checks the provider manifest, then exercises the full
# create -> list -> patch -> delete path for a channel + an alert rule,
# verifying the rule round-trips its channels[] list and renotify_secs.
# Self-cleaning + idempotent: removes any leftover "smoke-*" objects first.
#   bash scripts/check-alerts.sh
set -euo pipefail
BASE="${BASE:-http://localhost:8080}"
EMAIL="${ADMIN_EMAIL:-admin@local}"
PASS="${ADMIN_PASSWORD:-admin123}"
JAR="$(mktemp)"
trap 'rm -f "$JAR"' EXIT

py() { python3 -c "$1"; }                 # read JSON from stdin, run snippet
say() { printf '%-34s ' "$1"; }
g()  { curl -s -b "$JAR" "$BASE$1"; }
del() { curl -s -b "$JAR" -o /dev/null -w '%{http_code}' -X DELETE "$BASE$1"; }

code=$(curl -s -c "$JAR" -o /dev/null -w '%{http_code}' \
  -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}")
say "login"; echo "$code"; [ "$code" = 200 ] || { echo "login failed"; exit 1; }

say "channel-types (manifest)"; g /api/channel-types | py "import sys,json;print(len(json.load(sys.stdin)))"
say "systems"; g /api/systems | py "import sys,json;print(len(json.load(sys.stdin)))"

# --- pick a workspace that actually contains a system, so the alert we create
#     is visible in that workspace's alert list (list_alerts is target-scoped) ---
read WS SYS < <(python3 - "$(g /api/workspaces)" "$(g /api/systems)" <<'PY'
import sys, json
nss = json.loads(sys.argv[1]); systems = json.loads(sys.argv[2])
by_name = {n["name"]: n["id"] for n in nss}
for s in systems:
    nid = by_name.get(s.get("workspace"))
    if nid:
        print(nid, s["id"]); break
else:
    print("", "")
PY
)
say "workspace+system"; echo "${WS:-<none>} / ${SYS:-<none>}"
[ -n "$WS" ] || { echo "no workspace/system pair (run sim-agents.sh first)"; exit 1; }

# --- pre-clean any leftover smoke objects from a previous aborted run ---
for a in $(g "/api/workspaces/$WS/alerts" | py "import sys,json;[print(x['id']) for x in json.load(sys.stdin) if any(c['name'].startswith('smoke-') for c in x['channels'])]"); do del "/api/alerts/$a" >/dev/null; done
for c in $(g "/api/workspaces/$WS/channels" | py "import sys,json;[print(x['id']) for x in json.load(sys.stdin) if x['name'].startswith('smoke-')]"); do del "/api/channels/$c" >/dev/null; done

say "channels (before)"; g "/api/workspaces/$WS/channels" | py "import sys,json;print(len(json.load(sys.stdin)))"
say "alerts (before)";   g "/api/workspaces/$WS/alerts"   | py "import sys,json;print(len(json.load(sys.stdin)))"

# --- create two channels so we exercise the multi-channel fan-out ---
mkch() { curl -s -b "$JAR" -X POST "$BASE/api/workspaces/$WS/channels" -H 'content-type: application/json' \
  -d "{\"name\":\"$1\",\"kind\":\"webhook\",\"config\":{\"url\":\"https://example.com/$1\"}}" | py "import sys,json;print(json.load(sys.stdin))"; }
CH1=$(mkch smoke-a); CH2=$(mkch smoke-b)
say "created 2 channels"; echo "$CH1 $CH2"

# --- create an alert wired to BOTH channels, with a renotify cadence ---
AL=$(curl -s -b "$JAR" -X POST "$BASE/api/workspaces/$WS/alerts" -H 'content-type: application/json' \
  -d "{\"system_id\":\"$SYS\",\"channel_ids\":[\"$CH1\",\"$CH2\"],\"renotify_secs\":600,\"condition\":{\"kind\":\"metric\",\"metric\":\"cpu_percent\",\"op\":\">\",\"value\":90}}" \
  | py "import sys,json;print(json.load(sys.stdin))")
say "created alert"; echo "$AL"

row() { g "/api/workspaces/$WS/alerts" | py "import sys,json;d=[a for a in json.load(sys.stdin) if a['id']=='$AL'];print($1 if d else 'MISSING')"; }
say "alert channels (expect 2)"; row "len(d[0]['channels'])"
say "alert renotify_secs (600)"; row "d[0]['renotify_secs']"

# --- patch down to ONE channel + renotify off (the editor's save path) ---
curl -s -b "$JAR" -o /dev/null -X PATCH "$BASE/api/alerts/$AL" -H 'content-type: application/json' \
  -d "{\"channel_ids\":[\"$CH1\"],\"renotify_secs\":null}"
say "after patch channels (1)";    row "len(d[0]['channels'])"
say "after patch renotify (null)"; row "repr(d[0]['renotify_secs'])"

# --- cleanup ---
say "delete alert";    del "/api/alerts/$AL"; echo
say "delete channels"; echo "$(del "/api/channels/$CH1") $(del "/api/channels/$CH2")"
echo "OK"
