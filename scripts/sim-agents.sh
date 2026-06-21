#!/usr/bin/env bash
# Chạy test-agent simulator với mặc định khớp docker-compose.
# Yêu cầu: stack đã chạy (`docker compose up -d`).
#   scripts/sim-agents.sh                 # chạy mãi, ~33 host
#   DURATION=20 NODES=50 scripts/sim-agents.sh   # tuỳ biến qua env
set -e
REPO="$(cd "$(dirname "$0")/.." && pwd)"
export HUB_URL="${HUB_URL:-http://localhost:8080}"
export ADMIN_EMAIL="${ADMIN_EMAIL:-admin@local}"
export ADMIN_PASSWORD="${ADMIN_PASSWORD:-admin123}"
export INTERVAL="${INTERVAL:-5}"
exec node "$REPO/scripts/sim-agents.mjs"
