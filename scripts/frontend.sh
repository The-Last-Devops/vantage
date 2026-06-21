#!/usr/bin/env bash
# Frontend helpers (Vite + Vue, npm). Chạy: bash scripts/frontend.sh [install|build|dev|clean]
set -e
DIR="$(cd "$(dirname "$0")/.." && pwd)/frontend"
case "${1:-build}" in
  install) npm --prefix "$DIR" install ;;
  build)   npm --prefix "$DIR" run build ;;
  dev)     npm --prefix "$DIR" run dev ;;
  clean)   rm -rf "$DIR/node_modules" "$DIR/pnpm-lock.yaml" "$DIR/package-lock.json" ;;
  *) echo "usage: frontend.sh [install|build|dev|clean]"; exit 1 ;;
esac
