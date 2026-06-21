#!/usr/bin/env bash
# Run the NATIVE agent = real metrics of THIS machine (macOS/Linux), pushing to
# the local hub. (The docker-compose agent on macOS only sees the Docker VM, not
# the real Mac — so use this for real host data.)
#   bash scripts/run-agent.sh
set -e
REPO="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO"
export HUB_URL="${HUB_URL:-http://localhost:8080}"
export AGENT_TOKEN="${AGENT_TOKEN:-local-dev-token}"   # bootstrapped LOCAL key
export INTERVAL="${INTERVAL:-2}"
export HOSTNAME_OVERRIDE="${HOSTNAME_OVERRIDE:-$(hostname -s)}"
echo "native agent → $HUB_URL  (host=$HOSTNAME_OVERRIDE, every ${INTERVAL}s)"
exec cargo run -p agent
