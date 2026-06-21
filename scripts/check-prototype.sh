#!/usr/bin/env bash
# Dev checks cho UI prototype — gom vào 1 lệnh để khỏi accept nhiều lần.
#   scripts/check-prototype.sh            syntax JS + trạng thái trang
#   scripts/check-prototype.sh grep <từ>  tìm <từ> trong ui-prototype/simple/
set -u
REPO="$(cd "$(dirname "$0")/.." && pwd)"
DIR="$REPO/ui-prototype/simple"
BASE="http://127.0.0.1:4000/simple"

if [ "${1:-}" = "grep" ]; then
  grep -rn "${2:?cần từ khoá}" "$DIR" || echo "(không thấy)"
  exit 0
fi

echo "== inline <script> syntax =="
node - "$DIR" <<'EOF'
const fs=require('fs'),path=require('path');
const dir=process.argv[2];let bad=0;
(function walk(d){for(const e of fs.readdirSync(d,{withFileTypes:true})){const p=path.join(d,e.name);
  if(e.isDirectory())walk(p);
  else if(e.name.endsWith('.html')){const h=fs.readFileSync(p,'utf8');
    [...h.matchAll(/<script>([\s\S]*?)<\/script>/g)].forEach((m,i)=>{try{new Function(m[1]);}catch(err){bad++;console.log('  FAIL',path.relative(dir,p),'#'+i,err.message);}});}}})(dir);
console.log(bad?`  ${bad} lỗi syntax`:'  OK');
EOF

echo "== external JS (node --check) =="
for f in "$DIR"/js/*.js; do node --check "$f" 2>&1 && echo "  ok $(basename "$f")"; done

echo "== served pages =="
for p in index.html login.html pages/server.html; do
  printf "  %-20s -> " "$p"; curl -s -o /dev/null -w "%{http_code}\n" "$BASE/$p" 2>/dev/null || echo "server chưa chạy?"
done
