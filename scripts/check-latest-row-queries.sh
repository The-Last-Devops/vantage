#!/usr/bin/env bash
# Validate the "latest row per id" reads behind /api/systems and /api/monitors against a
# throwaway TimescaleDB. These are the hottest queries in the product (every dashboard
# polls them), and all three used to scan/sort the whole hypertable:
#   * systems list  — DISTINCT ON (system_id) over `= ANY($1)`, unbounded
#   * monitors list — DISTINCT ON (monitor_id) over `= ANY($1)`, unbounded
#   * uptime bar    — row_number() OVER (PARTITION BY monitor_id ORDER BY time DESC) <= 40
# They are now LATERAL LIMIT 1 / LIMIT 40 per id, which walks the (id, time DESC) index.
# This script asserts the new shape returns the SAME answers as the old one AND touches
# strictly fewer chunks / does no full sort. Idempotent + self-cleaning.
#   bash scripts/check-latest-row-queries.sh
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
CID="vantage-latestrow-$$"
cleanup() { docker rm -f "$CID" >/dev/null 2>&1 || true; }
trap cleanup EXIT

echo "starting throwaway TimescaleDB…"
docker run -d --name "$CID" \
  -e POSTGRES_USER=vantage -e POSTGRES_PASSWORD=vantage -e POSTGRES_DB=vantage_data \
  timescale/timescaledb:latest-pg18 >/dev/null

echo "waiting for readiness…"
ok=0
for i in $(seq 1 90); do
  if docker exec "$CID" psql -tAqX -U vantage -d vantage_data -c "SELECT 1" >/dev/null 2>&1; then
    ok=$((ok + 1)); [ "$ok" -ge 4 ] && break
  else ok=0; fi
  sleep 1
  [ "$i" = 90 ] && { echo "postgres never became stably ready"; exit 1; }
done

echo "applying data migrations…"
for f in "$REPO"/migrations/data/*.sql; do
  if ! docker exec -i "$CID" psql -v ON_ERROR_STOP=1 -q -U vantage -d vantage_data < "$f" >/dev/null 2>/tmp/lr_err; then
    echo "FAIL applying $(basename "$f"):"; cat /tmp/lr_err; exit 1
  fi
done
psql() { docker exec -i "$CID" psql -v ON_ERROR_STOP=1 -tAqX -U vantage -d vantage_data "$@"; }

S1="00000000-0000-0000-0000-0000000000a1"
S2="00000000-0000-0000-0000-0000000000a2"
M1="00000000-0000-0000-0000-0000000000b1"
M2="00000000-0000-0000-0000-0000000000b2"

# Seed history that spans several chunks (default chunk interval is 7 days), so a
# whole-table scan is measurably different from reading only the newest chunk.
echo "seeding 20 days of samples across 2 systems + 2 monitors…"
docker exec -i "$CID" psql -v ON_ERROR_STOP=1 -q -U vantage -d vantage_data >/dev/null <<SQL
INSERT INTO system_metrics (time, system_id, cpu_percent, mem_used, mem_total, swap_used,
  swap_total, disk_used, disk_total, net_rx, net_tx, load1, uptime, disk_util)
SELECT g, sid, (extract(epoch FROM g)::bigint % 97)::float8, 1, 2, 0, 0, 3, 4, 0, 0, 0.1, 1, 5
FROM generate_series(now() - interval '20 days', now(), interval '30 minutes') g,
     unnest(ARRAY['$S1','$S2']::uuid[]) sid;
INSERT INTO heartbeats (time, monitor_id, up, latency_ms, message)
SELECT g, mid, (extract(epoch FROM g)::bigint % 7) <> 0, 12, 'ok'
FROM generate_series(now() - interval '20 days', now(), interval '30 minutes') g,
     unnest(ARRAY['$M1','$M2']::uuid[]) mid;
SQL
echo "  rows: system_metrics=$(psql -c 'SELECT count(*) FROM system_metrics;') heartbeats=$(psql -c 'SELECT count(*) FROM heartbeats;')"

fail=0
assert() { if [ "$2" = "$3" ]; then echo "  ok: $1"; else echo "  FAIL: $1 (got '$2', want '$3')"; fail=1; fi; }
# EXPLAIN-based cost probes: how many chunks a plan reads, and whether it sorts.
chunks() { psql -c "EXPLAIN (COSTS OFF) $1" | grep -c '_hyper_[0-9]*_[0-9]*_chunk' || true; }
# Count real Sort / WindowAgg plan NODES — not the "Sort Key:" detail lines an ordered
# MergeAppend/ChunkAppend also prints (those mean an index walk, not a sort).
sorts()  { psql -c "EXPLAIN (COSTS OFF) $1" | grep -cE '(^|-> +)(Incremental )?(Sort|WindowAgg)' || true; }
cheaper() { # name, new_sql, old_sql
  local n o; n=$(chunks "$2"); o=$(chunks "$3")
  if [ "$n" -lt "$o" ]; then echo "  ok: $1 chunk scans $n (was $o)"
  else echo "  FAIL: $1 reads $n chunks, old read $o — no improvement"; fail=1; fi
}

echo
echo "1) /api/systems — latest sample per system"
NEW_SYS="SELECT s.sid, m.cpu_percent FROM unnest(ARRAY['$S1','$S2']::uuid[]) AS s(sid) \
  JOIN LATERAL (SELECT cpu_percent FROM system_metrics WHERE system_id = s.sid \
                AND time > now() - interval '6 hours' ORDER BY time DESC LIMIT 1) m ON true \
  ORDER BY s.sid"
OLD_SYS="SELECT DISTINCT ON (system_id) system_id, cpu_percent FROM system_metrics \
  WHERE system_id = ANY(ARRAY['$S1','$S2']::uuid[]) ORDER BY system_id, time DESC"
assert "same rows as DISTINCT ON" "$(psql -c "$NEW_SYS;")" "$(psql -c "$OLD_SYS;")"
cheaper "systems latest" "$NEW_SYS;" "$OLD_SYS;"

echo
echo "2) /api/monitors — latest heartbeat per monitor (no time bound: state must always show)"
NEW_HB="SELECT s.mid, h.up, h.latency_ms, h.message FROM unnest(ARRAY['$M1','$M2']::uuid[]) AS s(mid) \
  JOIN LATERAL (SELECT time, up, latency_ms, message FROM heartbeats WHERE monitor_id = s.mid \
                ORDER BY time DESC LIMIT 1) h ON true ORDER BY s.mid"
OLD_HB="SELECT DISTINCT ON (monitor_id) monitor_id, up, latency_ms, message FROM heartbeats \
  WHERE monitor_id = ANY(ARRAY['$M1','$M2']::uuid[]) ORDER BY monitor_id, time DESC"
assert "same rows as DISTINCT ON" "$(psql -c "$NEW_HB;")" "$(psql -c "$OLD_HB;")"
# Both plans keep ONE top-level Sort (the outer ORDER BY over the 2-row id list), so a
# raw sort count proves nothing. What matters is what sits below it: the new plan must
# reach the hypertable through an ordered ChunkAppend under a Limit (lazy — it stops at
# the newest chunk that has a row) with NO sort of the data itself.
assert "reads via ordered ChunkAppend under a Limit" \
  "$(psql -c "EXPLAIN (COSTS OFF) $NEW_HB;" | grep -c 'ChunkAppend')" "1"
assert "no sort below the per-id loop" \
  "$(psql -c "EXPLAIN (COSTS OFF) $NEW_HB;" | sed -n '/Nested Loop/,$p' | grep -cE '(^|-> +)(Incremental )?Sort')" "0"
assert "old plan sorted the hypertable to de-duplicate" \
  "$(psql -c "EXPLAIN (COSTS OFF) $OLD_HB;" | grep -c 'Unique')" "1"
# A monitor quiet for 20 days must still report a state (why there is no time bound here).
old_only=$(psql -c "SELECT count(*) FROM unnest(ARRAY['$M1']::uuid[]) AS s(mid) \
  JOIN LATERAL (SELECT up FROM heartbeats WHERE monitor_id = s.mid ORDER BY time DESC LIMIT 1) h ON true;")
assert "long-quiet monitor still reports state" "$old_only" "1"

echo
echo "3) uptime bar — last 40 beats per monitor"
NEW_BAR="SELECT s.mid, b.up FROM unnest(ARRAY['$M1','$M2']::uuid[]) AS s(mid) \
  JOIN LATERAL (SELECT up, time FROM heartbeats WHERE monitor_id = s.mid \
                AND time > now() - interval '7 days' \
                ORDER BY time DESC LIMIT 40) b ON true ORDER BY s.mid, b.time ASC"
OLD_BAR="SELECT monitor_id, up FROM (SELECT monitor_id, up, time, \
    row_number() OVER (PARTITION BY monitor_id ORDER BY time DESC) AS rn \
  FROM heartbeats WHERE monitor_id = ANY(ARRAY['$M1','$M2']::uuid[])) t \
  WHERE rn <= 40 ORDER BY monitor_id, time ASC"
assert "same 40 beats as row_number()" "$(psql -c "$NEW_BAR;" | md5sum)" "$(psql -c "$OLD_BAR;" | md5sum)"
assert "returns 40 per monitor" "$(psql -c "SELECT count(*) FROM ($NEW_BAR) x;")" "80"
w=$(psql -c "EXPLAIN (COSTS OFF) $OLD_BAR;" | grep -ci 'WindowAgg' || true)
assert "old plan had a WindowAgg over the whole table" "$w" "1"
assert "new plan has none" "$(psql -c "EXPLAIN (COSTS OFF) $NEW_BAR;" | grep -ci 'WindowAgg' || true)" "0"
cheaper "uptime bar" "$NEW_BAR;" "$OLD_BAR;"

echo
[ "$fail" = 0 ] && echo "PASS — latest-row reads are index-driven and answer identically" || { echo "FAILED"; exit 1; }
