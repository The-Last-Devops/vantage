#!/usr/bin/env bash
# A workspace VIEWER (non-admin) must be able to read a system's data. Regression guard
# for the 500 in can_view_system(): its `SELECT 1` is INT4 and was decoded as i64, so
# every system endpoint returned 500 — but only for a non-admin member (admins short-circuit
# on can_read_all, which is why it never showed in admin testing).
#   bash scripts/check-viewer-system-access.sh          (needs a running hub)
set -uo pipefail
BASE="${BASE:-http://localhost:8080}"
EMAIL="${ADMIN_EMAIL:-admin@local}"
PASS="${ADMIN_PASSWORD:-admin123}"
VMAIL="viewer-probe@example.com"; VPASS="viewer-probe-pw-123"
AJAR="$(mktemp)"; VJAR="$(mktemp)"; trap 'rm -f "$AJAR" "$VJAR"' EXIT
py() { python3 -c "$1"; }
say() { printf '%-48s ' "$1"; }
fail=0

for _ in $(seq 1 60); do curl -s -o /dev/null -m 2 "$BASE/healthz" && break; sleep 1; done
curl -s -c "$AJAR" -o /dev/null -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}"

# pick any reporting system, then resolve its workspace id by name (/api/systems
# returns the workspace NAME, not the id)
read -r SYS WSNAME <<<"$(curl -s -b "$AJAR" "$BASE/api/systems" \
  | py "import sys,json
d=json.load(sys.stdin)
print(*((d[0]['id'], d[0]['workspace']) if d else ('','')))")"
[ -n "$SYS" ] || { echo "no system reporting yet — run an agent (or scripts/sim-agents.sh) first"; exit 1; }
WS=$(curl -s -b "$AJAR" "$BASE/api/workspaces" \
  | py "import sys,json;d=[w for w in json.load(sys.stdin) if w['name']=='$WSNAME'];print(d[0]['id'] if d else '')")
[ -n "$WS" ] || { echo "could not resolve workspace '$WSNAME'"; exit 1; }

# a throwaway viewer of that workspace (add_member takes an email)
curl -s -b "$AJAR" -o /dev/null -X POST "$BASE/api/users" -H 'content-type: application/json' \
  -d "{\"email\":\"$VMAIL\",\"password\":\"$VPASS\"}"
UID_=$(curl -s -b "$AJAR" "$BASE/api/users" \
  | py "import sys,json;d=[u for u in json.load(sys.stdin) if u['email']=='$VMAIL'];print(d[0]['id'] if d else '')")
[ -n "$UID_" ] || { echo "could not create the probe user"; exit 1; }
say "add viewer to the workspace"
MCODE=$(curl -s -b "$AJAR" -o /dev/null -w '%{http_code}' -X POST "$BASE/api/workspaces/$WS/members" \
  -H 'content-type: application/json' -d "{\"email\":\"$VMAIL\",\"role\":\"viewer\"}")
case "$MCODE" in 2*) echo "ok";; *) echo "FAIL ($MCODE)"; fail=1;; esac

curl -s -c "$VJAR" -o /dev/null -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d "{\"email\":\"$VMAIL\",\"password\":\"$VPASS\"}"

for path in "metrics" "containers" "temps" "gpu"; do
  say "viewer GET /api/systems/{id}/$path"
  CODE=$(curl -s -b "$VJAR" -o /dev/null -w '%{http_code}' "$BASE/api/systems/$SYS/$path?range=1h")
  [ "$CODE" = 200 ] && echo "ok" || { echo "FAIL ($CODE)"; fail=1; }
done

# cleanup
curl -s -b "$AJAR" -o /dev/null -X DELETE "$BASE/api/users/$UID_"
echo "─────────────────────────────────────────────"
[ "$fail" = 0 ] && echo "OK" || echo "FAILURES"
exit "$fail"
