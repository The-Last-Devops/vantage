#!/usr/bin/env bash
# Verify the running stack: SPA served at /, short sim push, systems summary.
#   bash scripts/verify-stack.sh
set -u
REPO="$(cd "$(dirname "$0")/.." && pwd)"
HUB_URL="${HUB_URL:-http://localhost:8080}"

echo "== SPA at / =="
printf "  GET / -> "; curl -s -o /dev/null -w "%{http_code}\n" "$HUB_URL/" 2>/dev/null || echo "hub down?"

echo "== short sim push (10s) =="
DURATION=10 NODES=12 DOCKER=4 CONTAINERS=5 K8S_CLUSTERS=2 K8S_NODES=4 \
  bash "$REPO/scripts/sim-agents.sh"

echo "== systems summary =="
bash "$REPO/scripts/check-systems.sh"
