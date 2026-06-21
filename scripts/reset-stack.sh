#!/usr/bin/env bash
# Wipe DB volume + rebuild & restart the whole stack (hub embeds the Vue SPA),
# then wait for the hub to be healthy. Use after schema/name changes.
set -e
REPO="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO"
echo "down -v (wipes db volume)…"
docker compose down -v
echo "up --build…"
docker compose up -d --build
echo "wait for hub…"
for i in $(seq 1 120); do
  if curl -s -o /dev/null -m 2 http://localhost:8080/healthz 2>/dev/null; then
    echo "hub UP after ${i}s"; exit 0
  fi
  sleep 2
done
echo "hub not up after 240s — see 'docker compose logs hub'"; exit 1
