#!/usr/bin/env bash
# Exercise the backup/restore endpoints: download a config snapshot, restore it
# back, and round-trip the S3 settings (no live bucket needed).
#   bash scripts/check-backup.sh
set -euo pipefail
BASE="${BASE:-http://localhost:8080}"
EMAIL="${ADMIN_EMAIL:-admin@local}"
PASS="${ADMIN_PASSWORD:-admin123}"
JAR="$(mktemp)"; TMP="$(mktemp -d)"; trap 'rm -rf "$JAR" "$TMP"' EXIT
curl -fsS -c "$JAR" -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}" >/dev/null

echo "download config backup…"
curl -fsS -b "$JAR" "$BASE/api/admin/backup?metrics=false" -o "$TMP/b.json.gz"
python3 - "$TMP/b.json.gz" <<'PY'
import gzip,json,sys
d=json.load(gzip.open(sys.argv[1]))
assert d.get("format")=="last-monitor-backup", d.get("format")
print("  format ok, tables:", ", ".join(f"{k}={len(v)}" for k,v in d["config"].items()))
PY

echo "restore that backup…"
curl -fsS -b "$JAR" -X POST "$BASE/api/admin/restore" --data-binary "@$TMP/b.json.gz" \
  -H 'content-type: application/gzip' -o /dev/null -w "  restore → %{http_code}\n"

# Restore wipes+reloads users, cascade-deleting sessions → re-login.
echo "re-login after restore…"
curl -fsS -c "$JAR" -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}" -o /dev/null -w "  login → %{http_code}\n"

echo "download WITH metrics…"
curl -fsS -b "$JAR" "$BASE/api/admin/backup?metrics=true" -o "$TMP/m.json.gz"
python3 - "$TMP/m.json.gz" <<'PY'
import gzip,json,sys
d=json.load(gzip.open(sys.argv[1]))
m=d.get("metrics",{})
print("  metrics tables:", ", ".join(f"{k}={len(v)}" for k,v in m.items()))
PY

echo "S3 settings round-trip…"
curl -fsS -b "$JAR" "$BASE/api/admin/backup/s3" | python3 -c "import sys,json;print('  before:',json.load(sys.stdin))"
curl -fsS -b "$JAR" -X PUT "$BASE/api/admin/backup/s3" -H 'content-type: application/json' \
  -d '{"endpoint":"https://s3.example.com","region":"us-east-1","bucket":"demo","access_key":"AK","secret_key":"SK","prefix":"lm/"}' -o /dev/null -w "  save → %{http_code}\n"
curl -fsS -b "$JAR" "$BASE/api/admin/backup/s3" | python3 -c "import sys,json;d=json.load(sys.stdin);print('  after: configured',d['configured'],'secret_set',d['secret_set'],'bucket',d['bucket'])"
echo "OK"
