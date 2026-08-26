#!/usr/bin/env bash
# The version lives in four places and they must agree. Run before tagging a release.
#   bash scripts/check-version-sync.sh
#
# Cargo.lock is the one that gets forgotten: bumping Cargo.toml does not touch it, and a
# release commit made without a build in between ships a lockfile naming the previous
# version (v3.0.16 did). `cargo build` regenerates it — this fails loudly instead.
set -uo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
fail=0
say() { printf '%-46s ' "$1"; }

WS=$(awk -F\" '/^version = / {print $2; exit}' "$REPO/Cargo.toml")
say "Cargo.toml workspace version"; echo "$WS"

for crate in shared vantage-agent vantage-hub; do
  # The [[package]] block for a workspace member, i.e. the one with a path source.
  v=$(awk -v c="$crate" '
    $0 == "name = \"" c "\"" { getline; if ($0 ~ /^version = /) { gsub(/[",]/,"",$3); print $3; exit } }
  ' "$REPO/Cargo.lock")
  say "Cargo.lock: $crate"
  if [ "$v" = "$WS" ]; then echo "$v"; else echo "MISMATCH ($v, want $WS) — run cargo build"; fail=1; fi
done

CV=$(awk '/^version:/ {print $2; exit}' "$REPO/deploy/chart/Chart.yaml")
say "Chart.yaml version"
if [ "$CV" = "$WS" ]; then echo "$CV"; else echo "MISMATCH ($CV, want $WS)"; fail=1; fi

AV=$(awk -F\" '/^appVersion:/ {print $2; exit}' "$REPO/deploy/chart/Chart.yaml")
say "Chart.yaml appVersion"
if [ "$AV" = "$WS" ]; then echo "$AV"; else echo "MISMATCH ($AV, want $WS)"; fail=1; fi

say "CHANGELOG has an entry"
if grep -q "^## \[$WS\]" "$REPO/CHANGELOG.md"; then echo "yes"; else echo "MISSING for $WS"; fail=1; fi

echo "──────────────────────────────────────────────"
[ "$fail" = 0 ] && echo "PASS — every version agrees on $WS" || { echo "FAILED"; exit 1; }
