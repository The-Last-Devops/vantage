#!/usr/bin/env bash
# Smoke test: check whether the hub is up; if so, run the simulator for 8s with a small fleet.
set -u
REPO="$(cd "$(dirname "$0")/.." && pwd)"
HUB_URL="${HUB_URL:-http://localhost:8080}"

if curl -s -o /dev/null -m 2 "$HUB_URL/healthz" 2>/dev/null; then
  echo "hub UP ($HUB_URL) — running the simulator for 8s with a small fleet:"
  DURATION=8 NODES=4 DOCKER=2 CONTAINERS=3 K8S_CLUSTERS=1 K8S_NODES=3 \
    bash "$REPO/scripts/sim-agents.sh"
else
  echo "hub is NOT running at $HUB_URL — run 'docker compose up -d' first."
  echo "(simulator is ready: scripts/sim-agents.sh)"
fi
