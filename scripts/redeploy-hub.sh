#!/usr/bin/env bash
# Rebuild + restart the hub container (applies new migrations + ingest code), wait until healthy.
set -e
REPO="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO"
echo "Rebuilding + restarting hub…"
docker compose up -d --build hub
echo "Waiting for hub to become healthy…"
for i in $(seq 1 60); do
  if curl -s -o /dev/null -m 2 http://localhost:8080/healthz 2>/dev/null; then
    echo "hub UP after ${i}s"; exit 0
  fi
  sleep 2
done
echo "hub did not come up within 120s — see 'docker compose logs hub'"; exit 1
