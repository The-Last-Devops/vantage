#!/usr/bin/env bash
# Show the latest "Release" workflow run + its per-job status (public repo, no auth).
#   bash scripts/check-release.sh
set -euo pipefail
REPO="${REPO:-The-Last-Devops/last-monitor}"
API="https://api.github.com/repos/$REPO/actions"

run=$(curl -fsS "$API/runs?per_page=20" | python3 -c "
import sys,json
d=json.load(sys.stdin)
for r in d.get('workflow_runs',[]):
    if r['name']=='Release':
        print(r['id'], r['head_branch'], r['status'], r.get('conclusion') or '-', r['html_url']); break
")
[ -z "$run" ] && { echo "no Release run found"; exit 0; }
read -r RID BRANCH STATUS CONCL URL <<< "$run"
echo "Release $BRANCH — $STATUS/$CONCL"
echo "$URL"
echo "jobs:"
curl -fsS "$API/runs/$RID/jobs" | python3 -c "
import sys,json
for j in json.load(sys.stdin).get('jobs',[]):
    print(f\"  {j['status']:<11} {j.get('conclusion') or '-':<10} {j['name']}\")
"
