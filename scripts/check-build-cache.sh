#!/usr/bin/env bash
# Prove the Docker layer cache works for the hub image: build twice against a throwaway
# local registry using the SAME exporter CI uses (type=registry,mode=max), with a
# code-only change in between, and assert the expensive layers (cargo-chef deps, npm)
# are reused instead of recompiled.
#
# Why: CI used type=gha,mode=max — a Rust mode=max cache exceeds the 10 GB per-repo GHA
# limit, so entries were evicted and builds ran cold; release.yml also used a different
# scope than ci.yml, so a tag never reused what main had just built. Both now share
# ghcr.io/<image>:buildcache-<arch>.
#
#   bash scripts/check-build-cache.sh        (cold first build ~10 min)
set -uo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
REG=vantage-cachereg-$$
PORT=${PORT:-5001}
CACHE="localhost:$PORT/buildcache:hub"
BUILDER=vantage-cachetest-$$
LOG1=$(mktemp); LOG2=$(mktemp)
cleanup() {
  docker rm -f "$REG" >/dev/null 2>&1
  docker buildx rm "$BUILDER" >/dev/null 2>&1
  git -C "$REPO" checkout -- crates/hub/src/main.rs 2>/dev/null
  echo "build logs kept: $LOG1 (cold), $LOG2 (warm)"
}
trap cleanup EXIT

# --progress plain prints a step's name and its CACHED marker on SEPARATE lines, tied
# only by the "#N" id — resolve id -> name first, then check every match is CACHED.
step_cached() { # <logfile> <awk regex for the step command>
  awk -v pat="$2" '
    /^#[0-9]+ \[/       { id = substr($1, 2); if ($0 ~ pat) want[id] = 1 }
    /^#[0-9]+ CACHED$/  { cached[substr($1, 2)] = 1 }
    END { n = 0; for (i in want) { n++; if (!(i in cached)) exit 1 } exit (n ? 0 : 1) }
  ' "$1"
}

echo "starting throwaway registry on port $PORT ..."
docker run -d --name "$REG" -p "$PORT":5000 registry:2 >/dev/null
docker buildx create --name "$BUILDER" --driver docker-container --driver-opt network=host >/dev/null

build() { # <logfile>
  docker buildx build --builder "$BUILDER" -f "$REPO/deploy/Dockerfile.hub" \
    --platform "linux/$(docker version -f '{{.Server.Arch}}')" \
    --cache-from "type=registry,ref=$CACHE,registry.insecure=true" \
    --cache-to "type=registry,ref=$CACHE,mode=max,image-manifest=true,oci-mediatypes=true,registry.insecure=true" \
    --output type=cacheonly --progress plain "$REPO" >"$1" 2>&1
}

echo "build #1 (cold - populates the cache) ..."
t0=$SECONDS; build "$LOG1" || { echo "FAIL: cold build failed"; tail -30 "$LOG1"; exit 1; }
t1=$((SECONDS - t0)); echo "  done in ${t1}s"

echo "making a code-only change (a comment in crates/hub/src/main.rs) ..."
printf '\n// cache-check touch\n' >> "$REPO/crates/hub/src/main.rs"

echo "build #2 (warm - must reuse the dependency layers) ..."
t0=$SECONDS; build "$LOG2" || { echo "FAIL: warm build failed"; tail -30 "$LOG2"; exit 1; }
t2=$((SECONDS - t0)); echo "  done in ${t2}s"

fail=0
say() { printf '%-44s ' "$1"; }

say "cargo chef cook layer cached"
step_cached "$LOG2" "cargo chef cook" && echo ok || { echo "FAIL (deps recompiled)"; fail=1; }
say "npm ci layer cached"
step_cached "$LOG2" "npm ci" && echo ok || { echo "FAIL"; fail=1; }
say "npm run build layer cached"
step_cached "$LOG2" "npm run build" && echo ok || { echo "FAIL"; fail=1; }
say "final cargo build re-ran (expected)"
step_cached "$LOG2" "cargo build --release" && echo "unexpected CACHED" || echo ok
say "warm build faster than cold"
[ "$t2" -lt "$t1" ] && echo "ok (${t2}s vs ${t1}s)" || { echo "FAIL (${t2}s vs ${t1}s)"; fail=1; }

echo "CACHED steps: cold=$(grep -c '^#[0-9]* CACHED$' "$LOG1") warm=$(grep -c '^#[0-9]* CACHED$' "$LOG2")"
echo "---------------------------------------------"
[ "$fail" = 0 ] && echo "OK - registry layer cache works" || echo "FAILURES"
exit "$fail"
