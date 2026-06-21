#!/usr/bin/env bash
# Smoke test: kiểm tra hub có chạy không, nếu có thì chạy simulator 8s với fleet nhỏ.
set -u
REPO="$(cd "$(dirname "$0")/.." && pwd)"
HUB_URL="${HUB_URL:-http://localhost:8080}"

if curl -s -o /dev/null -m 2 "$HUB_URL/healthz" 2>/dev/null; then
  echo "hub UP ($HUB_URL) — chạy thử simulator 8s, fleet nhỏ:"
  DURATION=8 NODES=4 DOCKER=2 CONTAINERS=3 K8S_CLUSTERS=1 K8S_NODES=3 \
    bash "$REPO/scripts/sim-agents.sh"
else
  echo "hub CHƯA chạy ở $HUB_URL — cần 'docker compose up -d' trước."
  echo "(simulator đã sẵn sàng: scripts/sim-agents.sh)"
fi
