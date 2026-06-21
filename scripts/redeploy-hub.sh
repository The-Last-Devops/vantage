#!/usr/bin/env bash
# Rebuild + restart hub container (áp migration mới + code ingest mới), chờ healthy.
set -e
REPO="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO"
echo "Rebuild + restart hub…"
docker compose up -d --build hub
echo "Chờ hub healthy…"
for i in $(seq 1 60); do
  if curl -s -o /dev/null -m 2 http://localhost:8080/healthz 2>/dev/null; then
    echo "hub UP sau ${i}s"; exit 0
  fi
  sleep 2
done
echo "hub chưa lên sau 120s — xem 'docker compose logs hub'"; exit 1
