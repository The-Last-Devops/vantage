#!/usr/bin/env bash
# Run the test-agent simulator with defaults matching docker-compose.
# Requires the stack to be running (`docker compose up -d`).
#   scripts/sim-agents.sh                 # run forever, ~33 hosts
#   DURATION=20 NODES=50 scripts/sim-agents.sh   # customize via env
set -e
REPO="$(cd "$(dirname "$0")/.." && pwd)"
export HUB_URL="${HUB_URL:-http://localhost:8080}"
export ADMIN_EMAIL="${ADMIN_EMAIL:-admin@local}"
export ADMIN_PASSWORD="${ADMIN_PASSWORD:-admin123}"
export INTERVAL="${INTERVAL:-5}"
exec node "$REPO/scripts/sim-agents.mjs"
