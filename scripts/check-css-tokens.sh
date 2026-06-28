#!/usr/bin/env bash
# check-css-tokens.sh — design-system fidelity guard.
#
# Hand-written CSS (frontend/src/style.css) must reference design tokens
# (CSS vars), never raw hex colour literals — so a theme/token change in one
# place flows everywhere. Tailwind utilities live in .vue templates (not CSS)
# and map to tokens via tailwind.config.js, so they are fine.
#
# uPlot charts and the brand logo legitimately need literal colours (uPlot
# consumes concrete strings, the logo is brand art). Those .vue hex usages are
# reported as INFO so drift stays visible, but they do not fail the check.
#
# Idempotent, no side effects. Run: bash scripts/check-css-tokens.sh
set -euo pipefail
cd "$(dirname "$0")/.."

css="frontend/src/style.css"
fail=0

# Raw hex in a CSS *value* (after a `:`), ignoring `/* … */` token comments.
# `sed` strips block comments first so the documented `--bg: 11 14 20; /* #0B0E14 */`
# annotations don't trip the guard.
hits=$(sed 's:/\*[^*]*\*/::g' "$css" | grep -nE ':[^;{]*#[0-9a-fA-F]{3,8}\b' || true)
if [ -n "$hits" ]; then
  echo "FAIL: raw hex colour in $css — use a CSS var token instead:"
  echo "$hits"
  fail=1
else
  echo "OK: $css uses only token vars (no raw hex in values)."
fi

# Informational: literal hex in .vue (charts/logo). Not a failure.
echo
echo "INFO: literal hex in .vue (chart/brand colours — review on drift):"
grep -rnE '#[0-9a-fA-F]{3,8}\b' frontend/src --include='*.vue' | sed 's/^/  /' || true

exit $fail
