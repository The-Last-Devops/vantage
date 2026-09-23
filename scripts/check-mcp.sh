#!/usr/bin/env bash
# Smoke-test the embedded MCP server: mint a PAT, then drive POST /mcp over
# JSON-RPC (initialize → tools/list → tools/call) authed by that token. Confirms
# unauthenticated access is rejected. Self-cleaning.
#   bash scripts/check-mcp.sh
set -uo pipefail
BASE="${BASE:-http://localhost:8080}"
EMAIL="${ADMIN_EMAIL:-admin@local}"
PASS="${ADMIN_PASSWORD:-admin123}"
JAR="$(mktemp)"; trap 'rm -f "$JAR"' EXIT
py() { python3 -c "$1"; }
say() { printf '%-40s ' "$1"; }
fail=0

for i in $(seq 1 60); do curl -s -o /dev/null -m 2 "$BASE/healthz" && break; sleep 1; done
curl -s -c "$JAR" -o /dev/null -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}"
RESP=$(curl -s -b "$JAR" -X POST "$BASE/api/pats" -H 'content-type: application/json' -d '{"name":"mcp-probe"}')
TOKEN=$(printf '%s' "$RESP" | py "import sys,json;print(json.load(sys.stdin).get('token',''))")
PID=$(printf '%s' "$RESP" | py "import sys,json;print(json.load(sys.stdin).get('id',''))")

mcp() { curl -s -X POST "$BASE/mcp" -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' -d "$1"; }

say "initialize -> serverInfo.name"
printf '%s' "$(mcp '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}')" \
  | py "import sys,json;d=json.load(sys.stdin);print(d['result']['serverInfo']['name'])" | grep -qx "vantage" \
  && echo "ok" || { echo "FAIL"; fail=1; }

say "tools/list -> tool count"
N=$(printf '%s' "$(mcp '{"jsonrpc":"2.0","id":2,"method":"tools/list"}')" | py "import sys,json;print(len(json.load(sys.stdin)['result']['tools']))")
[ "$N" -ge 25 ] && echo "ok ($N tools)" || { echo "FAIL ($N)"; fail=1; }

# Every advertised tool must carry a usable inputSchema — an assistant picks a
# tool by its schema, and a missing one is a tool it will call wrongly.
say "tools/list -> every tool has a schema"
printf '%s' "$(mcp '{"jsonrpc":"2.0","id":21,"method":"tools/list"}')" \
  | py "import sys,json;t=json.load(sys.stdin)['result']['tools'];print('ok' if all(x.get('inputSchema',{}).get('type')=='object' and x.get('description') for x in t) else 'no')" \
  | grep -qx ok && echo "ok" || { echo "FAIL"; fail=1; }

say "tools/call list_services -> content"
printf '%s' "$(mcp '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_services","arguments":{}}}')" \
  | py "import sys,json;d=json.load(sys.stdin);print('ok' if d['result']['content'][0]['type']=='text' else 'no')" | grep -qx ok \
  && echo "ok" || { echo "FAIL"; fail=1; }

# --- the passthrough layer: the whole API, reached in-process -----------------
call() { mcp "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}"; }
text() { py "import sys,json;d=json.load(sys.stdin)['result'];print(('ERR ' if d.get('isError') else '')+d['content'][0]['text'])"; }

say "list_endpoints -> route map"
E=$(printf '%s' "$(call list_endpoints '{}')" | text | py "import sys,json;print(len(json.load(sys.stdin)))")
[ "$E" -ge 80 ] && echo "ok ($E routes)" || { echo "FAIL ($E)"; fail=1; }

say "api_request GET /api/workspaces -> 200"
printf '%s' "$(call api_request '{"method":"GET","path":"/api/workspaces"}')" | text \
  | py "import sys,json;print(json.load(sys.stdin)['status'])" | grep -qx 200 \
  && echo "ok" || { echo "FAIL"; fail=1; }

# The SPA fallback answers 200 + index.html for anything unrouted, so an
# unguarded passthrough would report success and hand back HTML.
say "api_request /dashboard -> refused"
printf '%s' "$(call api_request '{"method":"GET","path":"/dashboard"}')" | text | grep -q "must start with" \
  && echo "ok" || { echo "FAIL"; fail=1; }

say "api_request /mcp -> refused (no recursion)"
printf '%s' "$(call api_request '{"method":"GET","path":"/mcp"}')" | grep -q "Error" \
  && echo "ok" || { echo "FAIL"; fail=1; }

# A curated tool and its endpoint must agree — the wrapper builds the same path.
say "list_workspaces == GET /api/workspaces"
A=$(printf '%s' "$(call list_workspaces '{}')" | text | py "import sys,json;print(json.dumps(json.load(sys.stdin)['body'],sort_keys=True))")
B=$(curl -s -b "$JAR" "$BASE/api/workspaces" | py "import sys,json;print(json.dumps(json.load(sys.stdin),sort_keys=True))")
[ "$A" = "$B" ] && echo "ok" || { echo "FAIL"; fail=1; }

# --- a write round-trip through MCP alone: create -> read back -> delete ------
# Arguments are built with printf, not nested backslash-escapes: a mis-escaped
# JSON body reaches the hub as a 400 with an empty response, which looks exactly
# like a broken tool.
WS=$(printf '%s' "$(call list_workspaces '{}')" | text | py "import sys,json;b=json.load(sys.stdin)['body'];print(b[0]['id'] if b else '')")
if [ -n "$WS" ]; then
  say "create_service -> new monitor id"
  ARGS=$(printf '{"workspace_id":"%s","name":"mcp-smoke","kind":"http","target":"https://example.com"}' "$WS")
  MON=$(printf '%s' "$(call create_service "$ARGS")" | text | py "import sys,json;print(json.load(sys.stdin)['body'])")
  [ -n "$MON" ] && echo "ok ($MON)" || { echo "FAIL"; fail=1; }

  if [ -n "$MON" ]; then
    ID=$(printf '{"monitor_id":"%s"}' "$MON")

    say "get_service reads it back"
    printf '%s' "$(call get_service "$ID")" | text \
      | py "import sys,json;print(json.load(sys.stdin)['body']['name'])" | grep -qx mcp-smoke \
      && echo "ok" || { echo "FAIL"; fail=1; }

    say "update_service patches one field"
    printf '%s' "$(call update_service "$(printf '{"monitor_id":"%s","enabled":false}' "$MON")")" >/dev/null
    printf '%s' "$(call get_service "$ID")" | text \
      | py "import sys,json;print(json.load(sys.stdin)['body']['enabled'])" | grep -qix false \
      && echo "ok" || { echo "FAIL"; fail=1; }

    # The audit middleware sits on the router, so a write dispatched in-process
    # must land in the audit log exactly like one from the browser.
    say "the write was audited"
    printf '%s' "$(call audit_log '{"limit":50}')" | text | grep -q "/api/monitors" \
      && echo "ok" || { echo "FAIL"; fail=1; }

    say "delete_service cleans up"
    printf '%s' "$(call delete_service "$ID")" | text | grep -q '"status": 204' \
      && echo "ok" || { echo "FAIL"; fail=1; }
  fi
fi

say "bad uuid -> isError, not a 500"
printf '%s' "$(call get_service '{"monitor_id":"not-a-uuid"}')" \
  | py "import sys,json;d=json.load(sys.stdin)['result'];print('err' if d.get('isError') else 'no')" | grep -qx err \
  && echo "ok" || { echo "FAIL"; fail=1; }

say "tools/call unknown -> isError"
printf '%s' "$(mcp '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope","arguments":{}}}')" \
  | py "import sys,json;d=json.load(sys.stdin);print('err' if d.get('error') or d['result'].get('isError') else 'no')" | grep -qx err \
  && echo "ok" || { echo "FAIL"; fail=1; }

# GET /mcp is the self-check you can open in a browser. It must answer WITHOUT a
# token — that is the case it exists for: it separates "hub down" from "blocked in
# front of the hub" from "bad token", which otherwise look identical.
say "GET /mcp without a token -> JSON"
printf '%s' "$(curl -s "$BASE/mcp")" \
  | py "import sys,json;d=json.load(sys.stdin);print('ok' if d['server']=='vantage' and d['authenticated'] is False else 'no')" \
  | grep -qx ok && echo "ok" || { echo "FAIL"; fail=1; }

say "GET /mcp anonymous leaks nothing"
printf '%s' "$(curl -s "$BASE/mcp")" \
  | py "import sys,json;d=json.load(sys.stdin);print('ok' if 'identity' not in d and 'version' not in d else 'LEAK')" \
  | grep -qx ok && echo "ok" || { echo "FAIL"; fail=1; }

say "GET /mcp with a token -> identity + tools"
printf '%s' "$(curl -s -H "Authorization: Bearer $TOKEN" "$BASE/mcp")" \
  | py "import sys,json;d=json.load(sys.stdin);print('ok' if d['authenticated'] and d.get('identity') and d.get('tools',0)>=25 else 'no')" \
  | grep -qx ok && echo "ok" || { echo "FAIL"; fail=1; }

say "no auth rejected (401)"
[ "$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/mcp" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"ping"}')" = 401 ] \
  && echo "ok" || { echo "FAIL"; fail=1; }

curl -s -b "$JAR" -o /dev/null -X DELETE "$BASE/api/pats/$PID"
[ "$fail" -eq 0 ] && echo "OK" || { echo "MCP regressions"; exit 1; }
