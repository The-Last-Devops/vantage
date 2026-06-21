#!/usr/bin/env bash
# Web server tĩnh để xem UI prototype. Mặc định cổng 4000.
#   scripts/serve-prototype.sh        → http://127.0.0.1:4000/simple/
#   scripts/serve-prototype.sh 8090   → đổi cổng
set -e
PORT="${1:-4000}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO/ui-prototype"
echo "Phục vụ ui-prototype tại http://127.0.0.1:${PORT}/simple/"
exec python3 -m http.server "$PORT" --bind 127.0.0.1
