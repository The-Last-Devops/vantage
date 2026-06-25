#!/usr/bin/env bash
# Print (and validate) the CHANGELOG.md section the Release workflow will use as
# the GitHub Release notes for a version. Fails if that section is empty/missing.
# Run before tagging a release.
#   bash scripts/check-changelog.sh            # uses the workspace version
#   bash scripts/check-changelog.sh 1.5.0      # a specific version
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
VER="${1:-$(awk -F\" '/^version = / {print $2; exit}' "$REPO/Cargo.toml")}"

SECTION="$(awk -v v="$VER" '
  $0 ~ "^## \\[" v "\\]" {f=1; next}
  f && /^## \[/ {exit}
  f {print}
' "$REPO/CHANGELOG.md")"

if [ -z "$(printf '%s' "$SECTION" | tr -d '[:space:]')" ]; then
  echo "FAIL: no CHANGELOG.md entry for $VER" >&2
  exit 1
fi
echo "── release notes for v$VER ──────────────────────────────"
printf '%s\n' "$SECTION"
echo "─────────────────────────────────────────────────────────"
echo "OK"
