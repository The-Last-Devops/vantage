#!/usr/bin/env bash
# Dev hub: run the hub with `cargo run` (incremental, ~seconds) against the
# dockerized DB — no Docker image rebuild. Frees :8080 by stopping the compose hub.
#
# Dev workflow (no rebuilds for UI work):
#   1) bash scripts/dev-hub.sh          # backend on :8080 (only needed for Rust changes)
#   2) bash scripts/frontend.sh dev     # Vite HMR on :5173, proxies /api -> :8080
#   open http://localhost:5173   (edit .vue files → instant hot reload)
set -e
REPO="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO"
docker compose up -d db
docker compose stop hub agent 2>/dev/null || true   # free :8080, avoid a duplicate agent
export CONFIG_DATABASE_URL="postgres://lastmon:lastmon@localhost:5432/lastmon_config"
export DATA_DATABASE_URL="postgres://lastmon:lastmon@localhost:5432/lastmon_data"
export BIND_ADDR="0.0.0.0:8080"
export ADMIN_EMAIL="${ADMIN_EMAIL:-admin@local}"
export ADMIN_PASSWORD="${ADMIN_PASSWORD:-admin123}"
export LOCAL_AGENT_TOKEN="${LOCAL_AGENT_TOKEN:-local-dev-token}"
export RUST_LOG="${RUST_LOG:-info,sqlx=warn}"
echo "hub (cargo run) → :8080 ; now run: bash scripts/frontend.sh dev  (→ http://localhost:5173)"
exec cargo run -p hub
