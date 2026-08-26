#!/usr/bin/env bash
# "Test rule" must send THIS rule's real notification, in both shapes.
#
# It used to send Notification::test() — a generic "your channel is wired up correctly"
# — which proved the webhook worked and told you nothing about the rule. It now goes
# through the same target_info + condition_text the live alert engine uses, so the test
# is the real payload, and sends the DOWN and the UP variant so both can be eyeballed.
#
# Boots a hub on throwaway databases, points a webhook channel at a local sink, and
# asserts what actually arrives. Idempotent + self-cleaning.
#   bash scripts/check-rule-test.sh          (needs Docker)
set -uo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
CFG=vantage-rt-cfg-$$; DAT=vantage-rt-dat-$$
PORT=${PORT:-18093}; SINK=${SINK:-18094}
BASE="http://127.0.0.1:$PORT"
HUB=""; SINKPID=""
# The sink must NOT be on loopback: net_guard.rs blocks loopback webhook targets
# outright (SSRF), so a 127.0.0.1 sink yields 502 and receives nothing. RFC1918 is
# allowed by default, so the machine's LAN address works.
HOSTIP="${HOSTIP:-$(ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null || hostname -I 2>/dev/null | awk '{print $1}')}"
[ -n "$HOSTIP" ] || { echo "no non-loopback address found for the sink; set HOSTIP="; exit 1; }
SINKLOG=$(mktemp)
fail=0
say() { printf '%-56s ' "$1"; }
ok()  { echo "ok"; }
bad() { echo "FAIL ($1)"; fail=1; }
cleanup() {
  [ -n "$HUB" ] && { kill "$HUB" 2>/dev/null; wait "$HUB" 2>/dev/null; }
  # `disown` first so bash does not print "Terminated" noise after a clean PASS.
  [ -n "$SINKPID" ] && { disown "$SINKPID" 2>/dev/null; kill "$SINKPID" 2>/dev/null; }
  docker rm -f "$CFG" "$DAT" >/dev/null 2>&1
  rm -f "$SINKLOG"
}
trap cleanup EXIT

# A sink that appends every POST body to a file, one JSON per line.
python3 - "$SINK" "$SINKLOG" >/dev/null 2>&1 <<'PYEOF' &
import sys, json
from http.server import BaseHTTPRequestHandler, HTTPServer
port, path = int(sys.argv[1]), sys.argv[2]
class H(BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get('content-length') or 0)
        body = self.rfile.read(n).decode('utf-8', 'replace')
        with open(path, 'a') as f:
            f.write(body.replace('\n', ' ') + '\n')
        self.send_response(204); self.end_headers()
    def log_message(self, *a): pass
HTTPServer(('0.0.0.0', port), H).serve_forever()
PYEOF
SINKPID=$!

start_pg() {
  docker run -d --name "$1" -e POSTGRES_USER=vantage -e POSTGRES_PASSWORD=vantage \
    -e POSTGRES_DB="$3" -p "$4":5432 "$2" >/dev/null
  local o=0
  for _ in $(seq 1 90); do
    if docker exec "$1" pg_isready -U vantage >/dev/null 2>&1; then o=$((o+1)); [ $o -ge 4 ] && return 0; else o=0; fi
    sleep 1
  done
  echo "postgres $1 never became ready"; exit 1
}
echo "sink at http://$HOSTIP:$SINK/hook"
echo "starting throwaway databases…"
start_pg "$CFG" postgres:16.6-alpine vantage_config 55482
start_pg "$DAT" timescale/timescaledb:2.17.2-pg16 vantage_data 55483

echo "building + starting the hub…"
( cd "$REPO" && bash scripts/frontend.sh build >/dev/null 2>&1 )
cd "$REPO"
CONFIG_DATABASE_URL=postgres://vantage:vantage@localhost:55482/vantage_config \
DATA_DATABASE_URL=postgres://vantage:vantage@localhost:55483/vantage_data \
BIND_ADDR="127.0.0.1:$PORT" ADMIN_EMAIL=admin@local ADMIN_PASSWORD=admin123 \
INSECURE_COOKIES=1 RUST_LOG=warn cargo run -q --release -p vantage-hub >/tmp/ruletest-hub.log 2>&1 &
HUB=$!
for _ in $(seq 1 300); do curl -s -o /dev/null "$BASE/healthz" && break; kill -0 $HUB 2>/dev/null || { echo "hub died:"; tail -20 /tmp/ruletest-hub.log; exit 1; }; sleep 1; done

py() { python3 -c "$1"; }
JAR=$(mktemp); trap 'cleanup; rm -f "$JAR"' EXIT
curl -s -c "$JAR" -o /dev/null -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d '{"email":"admin@local","password":"admin123"}'
post() { curl -s -b "$JAR" -X POST "$BASE$1" -H 'content-type: application/json' -d "$2"; }

WS=$(post /api/workspaces '{"name":"ruletest"}' | py "import sys,json;print(json.load(sys.stdin)['id'])")
MON=$(post "/api/workspaces/$WS/monitors" \
  '{"name":"shop-api","kind":"http","target":"https://shop.example.com/health","interval_secs":60}' \
  | py "import sys,json;d=json.load(sys.stdin);print(d if isinstance(d,str) else d.get('id',''))")
CH=$(post "/api/workspaces/$WS/channels" \
  "{\"name\":\"sink\",\"kind\":\"webhook\",\"config\":{\"url\":\"http://$HOSTIP:$SINK/hook\"}}" \
  | py "import sys,json;d=json.load(sys.stdin);print(d if isinstance(d,str) else d.get('id',''))")
[ -n "$WS" ] && [ -n "$MON" ] && [ -n "$CH" ] || { echo "setup failed (ws=$WS mon=$MON ch=$CH)"; exit 1; }
RULE=$(post "/api/workspaces/$WS/alerts" "{\"monitor_id\":\"$MON\",\"channel_ids\":[\"$CH\"]}" \
  | py "import sys,json;d=json.load(sys.stdin);print(d if isinstance(d,str) else d.get('id',''))")
[ -n "$RULE" ] || { echo "could not create the rule"; exit 1; }

: > "$SINKLOG"
say "POST /api/alerts/:id/test succeeds"
c=$(curl -s -b "$JAR" -o /dev/null -w '%{http_code}' -X POST "$BASE/api/alerts/$RULE/test")
case "$c" in 2*) ok;; *) bad "$c";; esac
for _ in $(seq 1 20); do [ "$(wc -l < "$SINKLOG")" -ge 2 ] && break; sleep 0.5; done

say "the channel received exactly 2 messages (DOWN + UP)"
n=$(wc -l < "$SINKLOG" | tr -d ' '); [ "$n" = 2 ] && ok || bad "$n"

check() { # label jsonpath-ish python expr
  say "$1"
  r=$(python3 - "$SINKLOG" <<PY
import json,sys
msgs=[json.loads(l) for l in open(sys.argv[1]) if l.strip()]
# all() over an empty list is True, so without this every content assertion passed
# while nothing had been delivered at all.
print("nothing delivered" if len(msgs) != 2 else ($2))
PY
)
  [ "$r" = "True" ] && ok || bad "$r"
}
check "one DOWN and one UP" "sorted(m['status'] for m in msgs) == ['DOWN','UP']"
check "names the monitor, not a generic test" "all(m['target']=='shop-api' for m in msgs)"
check "carries the probed URL" "all(m.get('endpoint')=='https://shop.example.com/health' for m in msgs)"
check "carries the workspace" "all(m['workspace']=='ruletest' for m in msgs)"
check "carries the rule's condition" "all(m['condition']=='is DOWN' for m in msgs)"
check "says plainly that it is a test" "all('Test alert' in m['detail'] for m in msgs)"
check "titles match the real ones" "sorted(m['title'] for m in msgs) == ['\\u2705 shop-api \\u2014 UP','\\U0001f534 shop-api \\u2014 DOWN']"

# A channel-less rule cannot be created at all (create_alert rejects an empty
# channel_ids), so the guarantee worth pinning is that refusal — not a /test on a
# rule that can never exist.
say "creating a rule with no channels is refused"
c=$(curl -s -b "$JAR" -o /dev/null -w '%{http_code}' -X POST "$BASE/api/workspaces/$WS/alerts" \
  -H 'content-type: application/json' -d "{\"monitor_id\":\"$MON\",\"channel_ids\":[]}")
[ "$c" = 400 ] && ok || bad "$c"

say "testing a rule that does not exist is 4xx, not a 500"
c=$(curl -s -b "$JAR" -o /dev/null -w '%{http_code}' -X POST \
  "$BASE/api/alerts/00000000-0000-0000-0000-0000000000ff/test")
case "$c" in 4*) ok;; *) bad "$c";; esac

echo "─────────────────────────────────────────────────────────"
[ "$fail" = 0 ] && echo "PASS — Test rule sends the rule's own DOWN + UP payload" || echo "FAILED"
exit "$fail"
