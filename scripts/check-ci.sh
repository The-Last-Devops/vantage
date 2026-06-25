#!/usr/bin/env bash
# Show the latest run of every GitHub Actions workflow + failed job names.
# Public repo, no auth needed.
#   bash scripts/check-ci.sh           # latest run per workflow
#   bash scripts/check-ci.sh 12        # scan the last N runs (default 12)
set -euo pipefail
REPO="${REPO:-The-Last-Devops/last-monitor}"
N="${1:-12}"
API="https://api.github.com/repos/$REPO/actions"

curl -fsS "$API/runs?per_page=$N" | python3 -c "
import sys, json, urllib.request
runs = json.load(sys.stdin).get('workflow_runs', [])
seen = set()
for r in runs:
    key = r['name']
    if key in seen: continue
    seen.add(key)
    concl = r.get('conclusion') or r['status']
    mark = {'success':'✓','failure':'✗','in_progress':'…','queued':'…'}.get(concl, '?')
    print(f\"{mark} {r['name']:<26} {concl:<12} {r['head_branch']}  {r['display_title'][:50]}\")
    print(f\"   {r['html_url']}\")
    if concl == 'failure':
        try:
            jobs = json.load(urllib.request.urlopen(r['jobs_url']))['jobs']
            for j in jobs:
                if j.get('conclusion') == 'failure':
                    print(f\"   ✗ job: {j['name']}\")
        except Exception as e:
            print(f\"   (jobs lookup failed: {e})\")
"
