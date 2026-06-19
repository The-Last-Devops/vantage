//! Server-rendered web UI (Maud + Tailwind + HTMX + uPlot).
//!
//! Pages require a session (redirect to /login otherwise). Live tables are HTMX
//! fragments polled on an interval. Charts use uPlot fed by the JSON metrics API.
//! All static assets are embedded into the binary (no external CDN).

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::CookieJar;
use maud::{html, Markup, PreEscaped, DOCTYPE};
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::AppState;

/// The namespace selected in the header switcher (cookie `ns`); None = all.
fn selected_ns(jar: &CookieJar) -> Option<Uuid> {
    jar.get("ns").and_then(|c| c.value().parse::<Uuid>().ok())
}

/// The teal app logo mark (gradient square with a cut-out center).
fn brand_mark() -> PreEscaped<String> {
    PreEscaped(r#"<span style="display:inline-grid;place-items:center;width:22px;height:22px;border-radius:6px;background:conic-gradient(from 140deg,#34E1C4,#1c8f9e 55%,#34E1C4)"><span style="width:9px;height:9px;border-radius:3px;background:#0B0E14"></span></span>"#.to_string())
}

/// Inline Lucide-style SVG icon (16px, currentColor). Crisp + theme-aware.
fn icon(name: &str) -> PreEscaped<String> {
    let body = match name {
        "sort" => r#"<path d="m7 15 5 5 5-5"/><path d="m7 9 5-5 5 5"/>"#,
        "copy" => {
            r#"<rect width="14" height="14" x="8" y="8" rx="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>"#
        }
        "more" => {
            r#"<circle cx="12" cy="12" r="1"/><circle cx="12" cy="5" r="1"/><circle cx="12" cy="19" r="1"/>"#
        }
        "sun" => {
            r#"<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M6.3 17.7l-1.4 1.4M19.1 4.9l-1.4 1.4"/>"#
        }
        "search" => r#"<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>"#,
        "plus" => r#"<path d="M5 12h14M12 5v14"/>"#,
        "trash" => {
            r#"<path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>"#
        }
        "layout" => {
            r#"<rect width="7" height="7" x="3" y="3" rx="1"/><rect width="7" height="7" x="14" y="3" rx="1"/><rect width="7" height="7" x="14" y="14" rx="1"/><rect width="7" height="7" x="3" y="14" rx="1"/>"#
        }
        "chevron" => r#"<path d="m6 9 6 6 6-6"/>"#,
        "external" => {
            r#"<path d="M15 3h6v6M10 14 21 3M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>"#
        }
        _ => r#"<circle cx="12" cy="12" r="9"/>"#,
    };
    PreEscaped(format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="inline-block shrink-0 align-[-0.125em]">{body}</svg>"#
    ))
}

// ---- embedded assets --------------------------------------------------------

pub async fn app_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css")],
        include_str!("../static/app.css"),
    )
}
pub async fn htmx_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("../static/htmx.min.js"),
    )
}
pub async fn uplot_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("../static/uPlot.iife.min.js"),
    )
}
pub async fn uplot_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css")],
        include_str!("../static/uPlot.min.css"),
    )
}

// ---- layout -----------------------------------------------------------------

fn layout(title: &str, user: Option<&CurrentUser>, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" class="h-full" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "Last Monitor · " (title) }
                link rel="stylesheet" href="/static/app.css";
                link rel="stylesheet" href="/static/uPlot.min.css";
                // apply saved theme before paint to avoid flash
                script { (PreEscaped("try{if(localStorage.theme==='light')document.documentElement.classList.add('light')}catch(e){}")) }
                script src="/static/htmx.min.js" {}
                script src="/static/uPlot.iife.min.js" {}
            }
            body class="h-full bg-ink text-slate-100" {
                @if let Some(u) = user { (navbar(u)) }
                main class="mx-auto max-w-6xl px-4 py-6" { (body) }
                @if user.is_some() {
                    div id="palette" class="hidden fixed inset-0 z-[100] bg-black/50 p-4" onclick="if(event.target===this)closePalette()" {
                        div class="mx-auto mt-24 max-w-lg overflow-hidden rounded-xl border border-line bg-panel shadow-2xl" {
                            input id="palInput" class="w-full bg-transparent px-4 py-3 text-sm text-slate-100 outline-none" placeholder="Search servers & monitors…" oninput="palSearch()" {}
                            div id="palResults" class="max-h-80 overflow-y-auto border-t border-line p-2" {}
                        }
                    }
                    div id="modal" class="hidden fixed inset-0 z-[90] bg-black/50 p-4" onclick="if(event.target===this)closeModal()" {
                        div class="mx-auto mt-20 max-w-lg rounded-xl border border-line bg-panel p-5 shadow-2xl" {
                            div id="modalBody" {}
                        }
                    }
                    script { (PreEscaped(APP_JS)) }
                }
            }
        }
    }
}

fn navbar(user: &CurrentUser) -> Markup {
    html! {
        nav class="border-b border-line bg-panel" {
            div class="mx-auto flex max-w-6xl items-center gap-2 px-4 py-3" {
                a href="/" class="mr-4 flex items-center gap-2 text-sm font-bold tracking-tight text-white" { (brand_mark()) "Last Monitor" }
                a href="/" class="btn-ghost" { "Dashboard" }
                a href="/monitors" class="btn-ghost" { "Monitors" }
                // Manage with a hover-revealed submenu.
                div class="group relative" {
                    a href="/manage" class="btn-ghost" { "Manage ▾" }
                    div class="absolute left-0 top-full z-50 hidden min-w-[170px] pt-1 group-hover:block" {
                        div class="rounded-lg border border-line bg-panel p-1 shadow-xl" {
                            @for (slug, label) in MANAGE_TABS {
                                @if !admin_only_tab(slug) || user.is_admin {
                                    a href={"/manage/"(slug)}
                                      class="block rounded-md px-3 py-2 text-sm text-slate-300 hover:bg-line hover:text-white" { (label) }
                                }
                            }
                        }
                    }
                }
                div class="ml-auto flex items-center gap-2" {
                    button class="flex items-center gap-2 rounded-md border border-line px-2 py-1 text-xs text-slate-400 hover:text-slate-200" onclick="openPalette()" {
                        (icon("search")) "Search" span class="rounded bg-line px-1 text-[10px]" { "⌘K" }
                    }
                    button class="btn-ghost" title="Toggle theme" onclick="toggleTheme()" { (icon("sun")) }
                    // Rancher-style namespace switcher.
                    select id="nsSwitch" class="input max-w-[160px] py-1 text-xs" {
                        option { "All namespaces" }
                    }
                    span class="text-xs text-slate-400" { (user.email) }
                    @if user.is_admin { span class="pill bg-teal/20 text-teal" { "admin" } }
                    button class="btn-ghost" onclick="logout()" { "Logout" }
                }
            }
        }
        script { (PreEscaped(NAV_JS)) }
    }
}

const NAV_JS: &str = r#"
function logout(){ fetch('/api/auth/logout',{method:'POST'}).then(()=>location.href='/login'); }
function getCookie(n){const m=document.cookie.match('(^|;)\\s*'+n+'=([^;]*)');return m?decodeURIComponent(m.pop()):'';}
async function initNs(){
  const sel=document.getElementById('nsSwitch'); if(!sel) return;
  try{
    const r=await fetch('/api/namespaces'); if(!r.ok) return;
    const ns=await r.json(); const cur=getCookie('ns')||'all';
    sel.innerHTML='<option value="all">All namespaces</option>'+ns.map(n=>`<option value="${n.id}">${n.name}</option>`).join('');
    sel.value=cur;
    sel.onchange=()=>{document.cookie='ns='+encodeURIComponent(sel.value)+';path=/;max-age=31536000';location.reload();};
  }catch(e){}
}
initNs();
// theme
function toggleTheme(){ const on=document.documentElement.classList.toggle('light'); try{localStorage.theme=on?'light':'dark';}catch(e){} }
// command palette (Ctrl/Cmd-K)
let PAL=[];
async function jg(u){ try{const r=await fetch(u);return r.ok?await r.json():null;}catch(e){return null;} }
async function palLoad(){ const [s,m]=await Promise.all([jg('/api/servers'),jg('/api/monitors')]); PAL=[];
  (s||[]).forEach(x=>PAL.push({t:x.name,d:'server',u:'/server/'+x.id}));
  (m||[]).forEach(x=>PAL.push({t:x.name,d:'monitor',u:'/monitors'})); }
function openPalette(){ const p=document.getElementById('palette'); if(!p)return; p.classList.remove('hidden');
  const i=document.getElementById('palInput'); i.value=''; palLoad().then(palSearch); i.focus(); }
function closePalette(){ document.getElementById('palette')?.classList.add('hidden'); }
function palSearch(){ const q=(document.getElementById('palInput').value||'').toLowerCase();
  const items=PAL.filter(x=>x.t.toLowerCase().includes(q)).slice(0,12);
  document.getElementById('palResults').innerHTML = items.length ? items.map(x=>
    `<a href="${x.u}" class="flex items-center justify-between rounded px-3 py-2 text-sm text-slate-200 hover:bg-line"><span>${x.t}</span><span class="text-xs text-slate-500">${x.d}</span></a>`).join('')
    : '<div class="px-3 py-2 text-sm text-slate-500">No results</div>'; }
addEventListener('keydown',e=>{
  if((e.metaKey||e.ctrlKey)&&e.key.toLowerCase()==='k'){ e.preventDefault(); openPalette(); }
  if(e.key==='Escape'){ closePalette(); closeModal(); }
});
"#;

const APP_JS: &str = r#"
function openModal(html){ const m=document.getElementById('modal'); if(!m)return; document.getElementById('modalBody').innerHTML=html; m.classList.remove('hidden'); }
function closeModal(){ document.getElementById('modal')?.classList.add('hidden'); }
function copyTxt(t){ try{navigator.clipboard.writeText(t);}catch(e){} }
function esc(s){ return (s||'').replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c])); }

// --- Add server(s): create a reusable token, then show install snippets ---
async function addToken(){
  const nss = await jg('/api/namespaces');
  if(!nss||!nss.length){ alert('Create a namespace first (Manage → Namespaces).'); return; }
  const opts = nss.map(n=>`<option value="${n.id}">${esc(n.name)}</option>`).join('');
  openModal(`
    <h3 class="mb-1 text-base font-semibold">Add server(s)</h3>
    <p class="mb-4 text-xs text-slate-400">Create a reusable enrollment token, then run the agent anywhere with it. Servers register themselves automatically (great for k8s DaemonSets).</p>
    <div class="space-y-2">
      <select id="tkNs" class="input">${opts}</select>
      <input id="tkName" class="input" maxlength="32" placeholder="token name (e.g. prod-nodes)">
      <div class="flex justify-end gap-2 pt-1"><button class="btn-ghost" onclick="closeModal()">Cancel</button><button class="btn" onclick="createToken()">Create token</button></div>
    </div>`);
}
async function createToken(){
  const ns=document.getElementById('tkNs').value, name=document.getElementById('tkName').value||'token';
  const r=await fetch(`/api/namespaces/${ns}/tokens`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({name})});
  if(!r.ok){ alert('Error '+r.status); return; }
  const t=await r.json(); showInstall(t.token);
}
function instSnippet(t,token,hub){
  if(t==='binary') return `# install the binary, then run (or set these in /etc/last-monitor/agent.env for systemd):\nHUB_URL=${hub} AGENT_TOKEN=${token} DISK_PATH=/ ./last-agent`;
  if(t==='docker') return `docker run -d --restart=unless-stopped --pid=host \\\n  -e HUB_URL=${hub} -e AGENT_TOKEN=${token} -e DISK_PATH=/host \\\n  -v /:/host:ro -v /var/run/docker.sock:/var/run/docker.sock:ro \\\n  last-monitor-agent:latest`;
  if(t==='compose') return `services:\n  agent:\n    image: last-monitor-agent:latest\n    restart: unless-stopped\n    pid: host\n    environment:\n      HUB_URL: ${hub}\n      AGENT_TOKEN: ${token}\n      DISK_PATH: /host\n    volumes:\n      - /:/host:ro\n      - /var/run/docker.sock:/var/run/docker.sock:ro`;
  return `apiVersion: apps/v1\nkind: DaemonSet\nmetadata:\n  name: last-agent\n  namespace: monitoring\nspec:\n  selector: { matchLabels: { app: last-agent } }\n  template:\n    metadata: { labels: { app: last-agent } }\n    spec:\n      hostPID: true\n      containers:\n      - name: agent\n        image: last-monitor-agent:latest\n        env:\n        - { name: HUB_URL, value: "${hub}" }\n        - { name: AGENT_TOKEN, value: "${token}" }\n        - { name: DISK_PATH, value: "/host" }\n        volumeMounts:\n        - { name: host, mountPath: /host, readOnly: true }\n      volumes:\n      - { name: host, hostPath: { path: / } }`;
}
function showInstall(token){
  window._tok=token; window._hub=location.origin;
  const tabs=['binary','docker','compose','k8s'];
  openModal(`
    <h3 class="mb-1 text-base font-semibold">Install the agent</h3>
    <p class="mb-3 text-xs text-slate-400">Token <code class="text-teal">${esc(token)}</code> — reuse on any number of hosts.</p>
    <div class="mb-2 flex gap-1">${tabs.map(t=>`<button class="insttab btn-ghost text-xs" data-t="${t}" onclick="instTab('${t}')">${t}</button>`).join('')}</div>
    <pre id="instCode" class="max-h-72 overflow-auto rounded-lg border border-line bg-ink p-3 text-xs leading-relaxed text-slate-200"></pre>
    <div class="flex justify-end gap-2 pt-3"><button class="btn-ghost" onclick="location.reload()">Done</button><button class="btn" onclick="copyTxt(document.getElementById('instCode').textContent)">Copy</button></div>`);
  instTab('docker');
}
function instTab(t){ document.getElementById('instCode').textContent=instSnippet(t,window._tok,window._hub);
  document.querySelectorAll('.insttab').forEach(b=>b.classList.toggle('!text-teal',b.dataset.t===t)); }

// --- per-server actions (modal, survives table polling) ---
function openServerMenu(id,tok,host,name){
  openModal(`
    <h3 class="mb-3 text-base font-semibold">${esc(name)}</h3>
    <div class="space-y-1">
      <a class="block rounded-md px-3 py-2 text-sm text-slate-200 hover:bg-line" href="/server/${id}">Open details</a>
      <button class="block w-full rounded-md px-3 py-2 text-left text-sm text-rose-400 hover:bg-line" onclick="delToken('${tok}')">Delete…</button>
    </div>`);
}
async function delToken(tokenId){
  const info=await jg(`/api/tokens/${tokenId}/servers`);
  if(!info){ alert('Error loading token'); return; }
  const n=info.servers.length;
  if(n<=1){
    const what = n===1 ? `its server <b>${esc(info.servers[0])}</b>` : 'its (currently no) servers';
    openModal(`
      <h3 class="mb-2 text-base font-semibold text-rose-400">Delete token</h3>
      <p class="mb-4 text-sm text-slate-300">This deletes token <b>${esc(info.token_name)}</b> and ${what}, stopping enrollment.</p>
      <div class="flex justify-end gap-2"><button class="btn-ghost" onclick="closeModal()">Cancel</button>
      <button class="btn !bg-rose-600 hover:!bg-rose-500" onclick="doDelToken('${tokenId}')">Delete</button></div>`);
  } else {
    const phrase=`delete ${n} servers`;
    const list=info.servers.map(s=>`<li>${esc(s)}</li>`).join('');
    openModal(`
      <h3 class="mb-2 text-base font-semibold text-rose-400">Delete ${n} servers</h3>
      <p class="mb-2 text-sm text-slate-300">Token <b>${esc(info.token_name)}</b> is shared by <b>${n}</b> servers. Deleting it removes <b>all of them</b>:</p>
      <ul class="mb-3 max-h-32 list-disc overflow-y-auto rounded border border-line bg-ink p-2 pl-6 text-xs text-slate-400">${list}</ul>
      <p class="mb-1 text-xs text-slate-400">Type <code class="text-rose-300">${phrase}</code> to confirm:</p>
      <input id="delc" class="input mb-3" autocomplete="off" oninput="document.getElementById('delb').disabled=(this.value!=='${phrase}')">
      <div class="flex justify-end gap-2"><button class="btn-ghost" onclick="closeModal()">Cancel</button>
      <button id="delb" disabled class="btn !bg-rose-600 hover:!bg-rose-500 disabled:opacity-40" onclick="doDelToken('${tokenId}')">Delete all</button></div>`);
  }
}
async function doDelToken(id){ const r=await fetch(`/api/tokens/${id}`,{method:'DELETE'}); if(r.ok){closeModal();location.reload();} else alert('Error '+r.status); }

// --- Add monitor (popup) ---
async function addMonitor(){
  const nss=await jg('/api/namespaces'); if(!nss||!nss.length){ alert('Create a namespace first (Manage → Namespaces).'); return; }
  const opts=nss.map(n=>`<option value="${n.id}">${esc(n.name)}</option>`).join('');
  openModal(`
    <h3 class="mb-3 text-base font-semibold">Add monitor</h3>
    <div class="space-y-2">
      <select id="mNs" class="input">${opts}</select>
      <input id="mName" class="input" placeholder="name">
      <select id="mKind" class="input"><option value="http">http</option><option value="tcp">tcp</option><option value="ping">ping</option><option value="keyword">keyword</option></select>
      <input id="mTarget" class="input" placeholder="url / host:port / host">
      <input id="mInt" class="input" type="number" value="60" placeholder="interval (s)">
      <div class="flex justify-end gap-2 pt-1"><button class="btn-ghost" onclick="closeModal()">Cancel</button><button class="btn" onclick="createMonitor()">Create</button></div>
    </div>`);
}
async function createMonitor(){
  const ns=document.getElementById('mNs').value;
  const body={ name:document.getElementById('mName').value, kind:document.getElementById('mKind').value,
    target:document.getElementById('mTarget').value, interval_secs:Number(document.getElementById('mInt').value||60) };
  const r=await fetch(`/api/namespaces/${ns}/monitors`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)});
  if(r.ok){ closeModal(); location.reload(); } else alert('Error '+r.status+': '+(await r.text()));
}
"#;

// ---- login ------------------------------------------------------------------

pub async fn login_page() -> Markup {
    layout(
        "Sign in",
        None,
        html! {
            div class="mx-auto mt-24 max-w-sm" {
                div class="card p-7" {
                    div class="mb-4 inline-grid h-11 w-11 place-items-center rounded-xl"
                        style="background:conic-gradient(from 140deg,#34E1C4,#1c8f9e 55%,#34E1C4)" {
                        span class="h-4 w-4 rounded-md bg-panel" {}
                    }
                    h1 class="text-xl font-semibold tracking-tight" { "Last Monitor" }
                    p class="mb-5 mt-1 text-sm text-slate-400" { "Sign in to continue" }
                    form id="login" onsubmit="return doLogin(event)" class="space-y-3" {
                        input class="input" name="email" type="email" placeholder="email" required {}
                        input class="input" name="password" type="password" placeholder="password" required {}
                        button class="btn w-full" type="submit" { "Sign in" }
                        p id="err" class="text-sm text-rose-400" {}
                    }
                }
            }
            script { (PreEscaped(LOGIN_JS)) }
        },
    )
}

const LOGIN_JS: &str = r#"
async function doLogin(e){
  e.preventDefault();
  const f=e.target;
  const r=await fetch('/api/auth/login',{method:'POST',headers:{'content-type':'application/json'},
    body:JSON.stringify({email:f.email.value,password:f.password.value})});
  if(r.ok){location.href='/';} else {document.getElementById('err').textContent='Invalid credentials';}
  return false;
}
"#;

// ---- pages ------------------------------------------------------------------

pub async fn dashboard(State(_s): State<AppState>, user: Option<CurrentUser>) -> Response {
    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };
    layout(
        "Dashboard",
        Some(&user),
        html! {
            div class="mb-6" hx-get="/ui/hero" hx-trigger="load, every 5s" hx-swap="innerHTML" {
                div class="card text-sm text-slate-500" { "loading…" }
            }
            div class="mb-3 flex flex-wrap items-center justify-between gap-2" {
                h2 class="text-sm font-semibold uppercase tracking-wide text-slate-400" { "Servers" }
                div class="flex items-center gap-2" {
                    input id="srvFilter" class="input max-w-[200px] py-1 text-xs" placeholder="Filter…" oninput="applyView()" {}
                    div class="group relative" {
                        button class="btn-ghost text-xs" { "Columns ▾" }
                        div class="absolute right-0 top-full z-50 hidden min-w-[150px] pt-1 group-hover:block" {
                            div class="rounded-lg border border-line bg-panel p-2 text-xs text-slate-300 shadow-xl" {
                                @for (key, label) in [("cpu","CPU"),("mem","Memory"),("disk","Disk"),("net","Network"),("trend","Trend"),("agent","Agent")] {
                                    label class="flex items-center gap-2 px-1 py-1" {
                                        input type="checkbox" checked onchange=(format!("toggleCol('{key}',this.checked)")) {}
                                        (label)
                                    }
                                }
                            }
                        }
                    }
                    button class="btn text-xs" onclick="addToken()" { (icon("plus")) "Add server" }
                }
            }
            div class="card overflow-x-auto p-0" {
                table class="w-full" {
                    thead { tr class="border-b border-line" {
                        th class="th cursor-pointer select-none" onclick="sortBy('name')" { span class="inline-flex items-center gap-1" { "Server" (icon("sort")) } }
                        th class="th col-cpu cursor-pointer select-none" onclick="sortBy('cpu')" { span class="inline-flex items-center gap-1" { "CPU" (icon("sort")) } }
                        th class="th col-mem cursor-pointer select-none" onclick="sortBy('mem')" { span class="inline-flex items-center gap-1" { "Memory" (icon("sort")) } }
                        th class="th col-disk cursor-pointer select-none" onclick="sortBy('disk')" { span class="inline-flex items-center gap-1" { "Disk" (icon("sort")) } }
                        th class="th col-net cursor-pointer select-none" onclick="sortBy('net')" { span class="inline-flex items-center gap-1" { "Network" (icon("sort")) } }
                        th class="th col-trend" { "Trend" }
                        th class="th col-agent" { "Agent" }
                        th class="th" { "" }
                    } }
                    tbody id="srvBody" hx-get="/ui/servers" hx-trigger="load, every 2s" hx-swap="innerHTML" {
                        tr { td class="td" colspan="7" { "loading…" } }
                    }
                }
            }
            script { (PreEscaped(DASH_JS)) }
        },
    )
    .into_response()
}

const DASH_JS: &str = r#"
window._sortKey=null; window._sortDir=1;
function hiddenCols(){ try{ return JSON.parse(localStorage.getItem('hideCols')||'[]'); }catch(e){ return []; } }
function applyCols(){
  const hide=hiddenCols();
  ['cpu','mem','disk','net','trend','agent'].forEach(k=>{
    document.querySelectorAll('.col-'+k).forEach(el=>{ el.style.display = hide.includes(k)?'none':''; });
  });
  document.querySelectorAll('#srvFilter ~ .group input[type=checkbox]').forEach(()=>{});
}
function toggleCol(k,on){ let h=hiddenCols(); h=h.filter(x=>x!==k); if(!on)h.push(k); localStorage.setItem('hideCols',JSON.stringify(h)); applyCols(); }
function applyView(){
  const q=(document.getElementById('srvFilter')?.value||'').toLowerCase();
  const tb=document.getElementById('srvBody'); if(!tb) return;
  const rows=[...tb.querySelectorAll('tr[data-name]')];
  rows.forEach(r=>{ r.style.display = r.dataset.name.toLowerCase().includes(q)?'':'none'; });
  if(window._sortKey){
    const k=window._sortKey, dir=window._sortDir;
    rows.sort((a,b)=>{ const av=a.dataset[k]||'', bv=b.dataset[k]||'';
      return k==='name' ? dir*av.localeCompare(bv) : dir*((+av)-(+bv)); });
    rows.forEach(r=>tb.appendChild(r));
  }
  applyCols();
}
function sortBy(k){ if(window._sortKey===k){window._sortDir*=-1;}else{window._sortKey=k;window._sortDir=1;} applyView(); }
function drawSpark(cv){
  const pts=(cv.dataset.spark||'').split(',').filter(x=>x!=='').map(Number);
  const fill=cv.dataset.fill==='1';
  const w=cv.width=Math.max(40,cv.clientWidth)*2, h=cv.height=Math.max(20,cv.clientHeight)*2;
  const x=cv.getContext('2d'); x.clearRect(0,0,w,h); if(pts.length<2) return;
  const cap=100, X=i=>i/(pts.length-1)*w, Y=v=>h-4-(Math.min(Math.max(v,0),cap)/cap)*(h-8);
  if(fill){ const g=x.createLinearGradient(0,0,0,h); g.addColorStop(0,'#34E1C455'); g.addColorStop(1,'#34E1C400');
    x.beginPath(); x.moveTo(0,h); pts.forEach((p,i)=>x.lineTo(X(i),Y(p))); x.lineTo(w,h); x.fillStyle=g; x.fill(); }
  x.beginPath(); pts.forEach((p,i)=>i?x.lineTo(X(i),Y(p)):x.moveTo(X(i),Y(p)));
  x.strokeStyle='#34E1C4'; x.lineWidth=fill?2.4:2; x.lineJoin='round'; x.stroke();
}
function drawAll(){ document.querySelectorAll('canvas[data-spark]').forEach(drawSpark); }
document.addEventListener('htmx:afterSwap', ()=>{ applyView(); drawAll(); });
addEventListener('load', drawAll);
let _rt; addEventListener('resize', ()=>{ clearTimeout(_rt); _rt=setTimeout(drawAll,150); });
"#;

/// GET /ui/hero — fleet-health hero: overall status, counts, and a cluster CPU
/// sparkline aggregated across the in-scope servers.
pub async fn frag_hero(
    State(state): State<AppState>,
    user: CurrentUser,
    jar: CookieJar,
) -> Result<Markup, StatusCode> {
    let ns = selected_ns(&jar);
    let ns_filter =
        "($1 OR namespace_id IN (SELECT namespace_id FROM memberships WHERE user_id = $2)) \
         AND ($3::uuid IS NULL OR namespace_id = $3::uuid)";

    let servers: Vec<(Uuid, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(&format!(
        "SELECT id, last_seen FROM servers WHERE {ns_filter}"
    ))
    .bind(user.is_admin)
    .bind(user.id)
    .bind(ns)
    .fetch_all(&state.config)
    .await
    .map_err(ise)?;
    let now = chrono::Utc::now();
    let srv_total = servers.len() as i64;
    let srv_online = servers
        .iter()
        .filter(|(_, t)| t.map(|t| (now - t).num_seconds() < 60).unwrap_or(false))
        .count() as i64;
    let ids: Vec<Uuid> = servers.into_iter().map(|(id, _)| id).collect();

    let mon_ids: Vec<Uuid> =
        sqlx::query_as::<_, (Uuid,)>(&format!("SELECT id FROM monitors WHERE {ns_filter}"))
            .bind(user.is_admin)
            .bind(user.id)
            .bind(ns)
            .fetch_all(&state.config)
            .await
            .map_err(ise)?
            .into_iter()
            .map(|(id,)| id)
            .collect();
    let mut mon_up = 0i64;
    for id in &mon_ids {
        let up: Option<(bool,)> = sqlx::query_as(
            "SELECT up FROM heartbeats WHERE monitor_id = $1 ORDER BY time DESC LIMIT 1",
        )
        .bind(id)
        .fetch_optional(&state.data)
        .await
        .map_err(ise)?;
        if matches!(up, Some((true,))) {
            mon_up += 1;
        }
    }
    let mon_total = mon_ids.len() as i64;
    let down = mon_total - mon_up;

    // Cluster CPU per-minute over the last hour + current avg cpu/mem.
    let mut cpu_series: Vec<f64> = Vec::new();
    let (mut avg_cpu, mut avg_mem) = (0.0f64, 0.0f64);
    if !ids.is_empty() {
        let rows: Vec<(chrono::DateTime<chrono::Utc>, f64)> = sqlx::query_as(
            "SELECT time_bucket('1 minute', time) b, avg(cpu_percent) \
             FROM metrics WHERE server_id = ANY($1) AND time > now() - interval '1 hour' \
             GROUP BY b ORDER BY b",
        )
        .bind(&ids)
        .fetch_all(&state.data)
        .await
        .map_err(ise)?;
        cpu_series = rows.into_iter().map(|(_, c)| c).collect();

        let cur: Option<(Option<f64>, Option<f64>)> = sqlx::query_as(
            "SELECT avg(cpu_percent), \
                    avg(CASE WHEN mem_total > 0 THEN mem_used::float8 / mem_total * 100 ELSE 0 END) \
             FROM metrics WHERE server_id = ANY($1) AND time > now() - interval '2 minutes'",
        )
        .bind(&ids)
        .fetch_optional(&state.data)
        .await
        .map_err(ise)?;
        if let Some((c, m)) = cur {
            avg_cpu = c.unwrap_or(0.0);
            avg_mem = m.unwrap_or(0.0);
        }
    }
    let series_str = cpu_series
        .iter()
        .map(|v| format!("{v:.1}"))
        .collect::<Vec<_>>()
        .join(",");
    let healthy = down == 0 && srv_total > 0 && srv_online == srv_total;

    Ok(html! {
        div class="grid gap-px overflow-hidden rounded-2xl border border-line bg-line md:grid-cols-[300px_1fr]" {
            // health
            div class="flex flex-col justify-center gap-3 bg-panel p-6"
                style="background:linear-gradient(160deg,rgba(16,33,33,0),rgba(14,43,42,.28))" {
                span class="inline-flex items-center gap-2.5 text-sm font-semibold" {
                    span class={"dot pulse-dot " (if healthy { "bg-teal" } else { "bg-amber-400" })} style="width:10px;height:10px" {}
                    (if healthy { "All systems operational" } else { "Degraded" })
                }
                div class="num text-[44px] font-semibold leading-none tracking-tight" {
                    (srv_online) span class="text-2xl text-slate-500" { " / " (srv_total) }
                }
                div class="text-[13px] text-slate-400" {
                    "systems reporting" (PreEscaped(" · ")) (down) " incident" (if down == 1 { "" } else { "s" })
                }
            }
            // cluster chart + stats
            div class="bg-panel p-6" {
                div class="mb-1 flex items-baseline justify-between" {
                    span class="text-[11px] font-semibold uppercase tracking-[.18em] text-slate-500" { "Cluster CPU · last hour" }
                    span class="num text-[13px] text-teal" { (format!("{avg_cpu:.0}%")) span class="text-slate-500" { " avg" } }
                }
                canvas class="block w-full" data-spark=(series_str) data-fill="1" style="height:120px" {}
                div class="mt-3 grid grid-cols-4 gap-2 border-t border-line pt-3" {
                    (hero_stat("Systems online", &format!("{srv_online}"), Some(&format!("/{srv_total}")), "text-white"))
                    (hero_stat("Monitors up", &format!("{mon_up}"), Some(&format!("/{mon_total}")), "text-teal"))
                    (hero_stat("Down", &format!("{down}"), None, if down > 0 { "text-rose-400" } else { "text-white" }))
                    (hero_stat("Avg memory", &format!("{avg_mem:.0}%"), None, "text-white"))
                }
            }
        }
    })
}

fn hero_stat(label: &str, value: &str, suffix: Option<&str>, color: &str) -> Markup {
    html! {
        div {
            div class="text-[11px] text-slate-400" { (label) }
            div class={"num mt-0.5 text-[22px] font-semibold " (color)} {
                (value)
                @if let Some(s) = suffix { span class="text-[15px] text-slate-500" { (s) } }
            }
        }
    }
}

pub async fn monitors_page(State(_s): State<AppState>, user: Option<CurrentUser>) -> Response {
    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };
    layout(
        "Monitors",
        Some(&user),
        html! {
            div class="mb-3 flex items-center justify-between" {
                h2 class="text-sm font-semibold uppercase tracking-wide text-slate-400" { "Service monitors" }
                button class="btn text-xs" onclick="addMonitor()" { (icon("plus")) "Add monitor" }
            }
            div class="space-y-3" hx-get="/ui/monitors" hx-trigger="load, every 3s" hx-swap="innerHTML" {
                div class="card text-sm text-slate-400" { "loading…" }
            }
        },
    )
    .into_response()
}

// ---- live fragments ---------------------------------------------------------

pub async fn frag_servers(
    State(state): State<AppState>,
    user: CurrentUser,
    jar: CookieJar,
) -> Result<Markup, StatusCode> {
    let ns = selected_ns(&jar);
    let servers: Vec<(Uuid, String, Option<String>, Option<chrono::DateTime<chrono::Utc>>, Option<String>, Uuid)> =
        sqlx::query_as(
            "SELECT s.id, s.name, s.hostname, s.last_seen, s.agent_version, s.token_id FROM servers s \
             WHERE ($1 OR s.namespace_id IN (SELECT namespace_id FROM memberships WHERE user_id = $2)) \
             AND ($3::uuid IS NULL OR s.namespace_id = $3::uuid) \
             ORDER BY s.name",
        )
        .bind(user.is_admin)
        .bind(user.id)
        .bind(ns)
        .fetch_all(&state.config)
        .await
        .map_err(ise)?;

    type Sample = (
        chrono::DateTime<chrono::Utc>,
        f64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    );
    #[allow(clippy::type_complexity)]
    let mut rows: Vec<(
        Uuid,
        String,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<String>,
        Uuid,
        Vec<Sample>,
        String,
    )> = Vec::new();
    for (id, name, hostname, last_seen, ver, token_id) in servers {
        let last2: Vec<Sample> = sqlx::query_as(
            "SELECT time, cpu_percent, mem_used, mem_total, disk_used, disk_total, net_rx, net_tx \
             FROM metrics WHERE server_id = $1 ORDER BY time DESC LIMIT 2",
        )
        .bind(id)
        .fetch_all(&state.data)
        .await
        .map_err(ise)?;
        // last ~32 CPU points (oldest→newest) for the trend sparkline.
        let mut spark: Vec<f64> = sqlx::query_as::<_, (f64,)>(
            "SELECT cpu_percent FROM metrics WHERE server_id = $1 ORDER BY time DESC LIMIT 32",
        )
        .bind(id)
        .fetch_all(&state.data)
        .await
        .map_err(ise)?
        .into_iter()
        .map(|(c,)| c)
        .collect();
        spark.reverse();
        let spark_str = spark
            .iter()
            .map(|v| format!("{v:.0}"))
            .collect::<Vec<_>>()
            .join(",");
        rows.push((
            id, name, hostname, last_seen, ver, token_id, last2, spark_str,
        ));
    }

    Ok(html! {
        @if rows.is_empty() {
            tr { td class="td text-slate-400" colspan="8" { "No servers yet — click “Add server”." } }
        }
        @for (id, name, hostname, last_seen, ver, token_id, s, spark_str) in rows {
            @let host = hostname.clone().unwrap_or_else(|| "—".into());
            @let first = s.first();
            @let cpu = first.map(|x| x.1).unwrap_or(0.0);
            @let memp = first.map(|x| pct(x.2, x.3)).unwrap_or(0.0);
            @let diskp = first.map(|x| pct(x.4, x.5)).unwrap_or(0.0);
            @let netrate = match (s.first(), s.get(1)) {
                (Some(n), Some(o)) if n.0 > o.0 => {
                    let dt = (n.0 - o.0).num_seconds().max(1) as f64;
                    ((n.6 - o.6).max(0) + (n.7 - o.7).max(0)) as f64 / dt
                }
                _ => 0.0,
            };
            tr class="hover:bg-line/40" data-name=(name) data-cpu=(format!("{cpu:.1}")) data-mem=(format!("{memp:.1}")) data-disk=(format!("{diskp:.1}")) data-net=(format!("{netrate:.0}")) {
                td class="td" {
                    div class="flex items-center gap-2" {
                        span class={"dot " (online_dot(last_seen))} {}
                        div {
                            div class="flex items-center gap-1.5" {
                                a class="font-medium text-slate-100 hover:text-teal" href={"/server/"(id)} { (name) }
                                button class="text-slate-500 hover:text-slate-300" title="Copy name" onclick=(format!("copyTxt('{name}')")) { (icon("copy")) }
                            }
                            div class="text-xs text-slate-500" { (host) }
                        }
                    }
                }
                @match first {
                    Some(_) => {
                        td class="td col-cpu" { (gauge(cpu)) }
                        td class="td col-mem" { (gauge(memp)) }
                        td class="td col-disk" { (gauge(diskp)) }
                        td class="td col-net whitespace-nowrap text-xs text-slate-300" { (net_rate(&s)) }
                    }
                    None => {
                        td class="td col-cpu text-slate-500" { "—" } td class="td col-mem text-slate-500" { "—" }
                        td class="td col-disk text-slate-500" { "—" } td class="td col-net text-slate-500" { "—" }
                    }
                }
                td class="td col-trend" {
                    canvas data-spark=(spark_str) style="width:104px;height:30px;display:block" {}
                }
                td class="td col-agent whitespace-nowrap text-xs text-slate-400" { (ver.unwrap_or_else(|| "—".into())) }
                td class="td text-right" {
                    button class="btn-ghost px-2" title="Actions"
                        onclick=(format!("openServerMenu('{id}','{token_id}','{host}','{name}')")) { (icon("more")) }
                }
            }
        }
    })
}

pub async fn frag_monitors(
    State(state): State<AppState>,
    user: CurrentUser,
    jar: CookieJar,
) -> Result<Markup, StatusCode> {
    let ns = selected_ns(&jar);
    let monitors: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT m.id, m.name, m.kind::text, m.target FROM monitors m \
         WHERE ($1 OR m.namespace_id IN (SELECT namespace_id FROM memberships WHERE user_id = $2)) \
         AND ($3::uuid IS NULL OR m.namespace_id = $3::uuid) \
         ORDER BY m.name",
    )
    .bind(user.is_admin)
    .bind(user.id)
    .bind(ns)
    .fetch_all(&state.config)
    .await
    .map_err(ise)?;

    // Per monitor: last ~40 heartbeats (newest first) for the heartbeat bar + uptime%.
    type Beat = (bool, Option<i32>, Option<String>);
    let mut rows: Vec<(String, String, String, Vec<Beat>)> = Vec::new();
    for (id, name, kind, target) in monitors {
        let beats: Vec<Beat> = sqlx::query_as(
            "SELECT up, latency_ms, message FROM heartbeats \
             WHERE monitor_id = $1 ORDER BY time DESC LIMIT 40",
        )
        .bind(id)
        .fetch_all(&state.data)
        .await
        .map_err(ise)?;
        rows.push((name, kind, target, beats));
    }

    Ok(html! {
        @if rows.is_empty() {
            div class="card text-sm text-slate-400" { "No monitors yet — add one in Manage." }
        }
        @for (name, kind, target, beats) in rows {
            @let latest = beats.first();
            @let up = latest.map(|(u, _, _)| *u);
            @let latency = latest.and_then(|(_, l, _)| *l);
            @let msg = latest.and_then(|(_, _, m)| m.clone());
            @let total = beats.len();
            @let ups = beats.iter().filter(|(u, _, _)| *u).count();
            @let uptime = if total > 0 { ups as f64 / total as f64 * 100.0 } else { 0.0 };
            div class="card" {
                div class="grid items-center gap-5 md:grid-cols-[236px_1fr_240px]" {
                    // info (left)
                    div class="flex min-w-0 items-center gap-3" {
                        span class={"dot " (match up { Some(true)=>"bg-teal", Some(false)=>"bg-rose-500", None=>"bg-slate-600" })} {}
                        div class="min-w-0" {
                            div class="truncate font-medium text-slate-100" { (name) }
                            div class="truncate text-xs text-slate-500" { (kind) " · " (target) }
                        }
                    }
                    // tall heartbeat (center)
                    div class="min-w-0" {
                        div class="flex h-[50px] items-stretch gap-[3px]" {
                            @for (u, _, _) in beats.iter().rev() {
                                span class={"min-w-[2px] flex-1 rounded-sm " (if *u { "bg-teal" } else { "bg-rose-500" })}
                                     title=(if *u { "up" } else { "down" }) {}
                            }
                        }
                        div class="mt-1.5 flex justify-between text-[10px] text-slate-500" {
                            span { (format!("{total} checks ago")) } span { "now" }
                        }
                    }
                    // stats (right)
                    div class="flex items-center justify-end gap-5" {
                        div class="text-right" { div class="text-[10px] uppercase tracking-wide text-slate-500" { "Uptime" }
                              div class="num text-sm font-semibold text-slate-200" { (format!("{uptime:.0}%")) } }
                        div class="text-right" { div class="text-[10px] uppercase tracking-wide text-slate-500" { "Latency" }
                              div class="num text-sm font-semibold text-slate-200" { (latency.map(|l| format!("{l}ms")).unwrap_or_else(|| "—".into())) } }
                        (status_pill(up))
                    }
                }
                @if let Some(m) = msg { @if up == Some(false) {
                    div class="mt-3 rounded-md bg-rose-500/10 px-2.5 py-1.5 text-xs text-rose-300" { (m) }
                } }
            }
        }
    })
}

// ---- server detail (chart) --------------------------------------------------

pub async fn server_detail(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<Uuid>,
) -> Response {
    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };
    match crate::web::can_view_server(&state, &user, id).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::FORBIDDEN.into_response(),
        Err(e) => return e.into_response(),
    }
    let meta: Option<(String, Option<String>, Option<String>, Option<String>, Option<i32>, Option<String>)> =
        sqlx::query_as(
            "SELECT name, hostname, kernel, cpu_model, cpu_cores, agent_version FROM servers WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&state.config)
        .await
        .ok()
        .flatten();
    let (name, hostname, kernel, cpu_model, cpu_cores, agent_version) =
        meta.unwrap_or_else(|| ("server".into(), None, None, None, None, None));

    // Latest uptime + online status from the newest sample.
    let latest: Option<(i64, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT uptime, time FROM metrics WHERE server_id = $1 ORDER BY time DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&state.data)
    .await
    .ok()
    .flatten();
    let uptime = latest.as_ref().map(|(u, _)| *u);
    let online = latest
        .as_ref()
        .map(|(_, t)| (chrono::Utc::now() - *t).num_seconds() < 60)
        .unwrap_or(false);

    let badge = |label: String| html! { span class="rounded-md border border-line bg-ink px-2 py-1 text-xs text-slate-300" { (label) } };

    layout(
        &name,
        Some(&user),
        html! {
            div class="mb-2 flex flex-wrap items-center gap-3" {
                a href="/" class="btn-ghost text-xs" { "← Servers" }
                span class={"dot " (if online { "bg-teal" } else { "bg-rose-500" })} {}
                h1 class="text-xl font-semibold" { (name) }
                span class={"pill " (if online { "bg-teal/20 text-teal" } else { "bg-rose-500/20 text-rose-300" })} {
                    (if online { "Up" } else { "Down" })
                }
                div class="ml-auto flex items-center gap-2" {
                    button class="btn-ghost" title="Toggle layout" onclick="toggleLayout()" { (icon("layout")) }
                    div class="flex items-center gap-1 rounded-lg border border-line bg-panel p-1" {
                        button class="rng btn-ghost text-xs" data-range="1h" onclick="setRange('1h')" { "1h" }
                        button class="rng btn-ghost text-xs" data-range="6h" onclick="setRange('6h')" { "6h" }
                        button class="rng btn-ghost text-xs" data-range="24h" onclick="setRange('24h')" { "24h" }
                    }
                }
            }
            // metadata badges (Beszel-style)
            div class="mb-5 flex flex-wrap gap-2" {
                @if let Some(h) = &hostname { (badge(format!("🖥 {h}"))) }
                @if let Some(u) = uptime { (badge(format!("⏱ {}", uptime_str(u)))) }
                @if let Some(k) = &kernel { @if !k.is_empty() { (badge(format!("🐧 {k}"))) } }
                @if let Some(c) = &cpu_model { @if !c.is_empty() {
                    (badge(format!("⚙ {c}{}", cpu_cores.map(|n| format!(" ({n} cores)")).unwrap_or_default())))
                } }
                @if let Some(v) = &agent_version { @if !v.is_empty() { (badge(format!("agent {v}"))) } }
            }
            div class="cgrid grid gap-4 lg:grid-cols-2" {
                (chart_card("cpu", "CPU", "System-wide CPU utilization"))
                (chart_card("mem", "Memory", "Used memory at the recorded time"))
                (chart_card("disk", "Disk", "Usage of the monitored filesystem"))
                (chart_card("dio", "Disk I/O", "Read / write throughput"))
                (chart_card("net", "Network", "↓ download · ↑ upload"))
            }
            // Docker container stats — stacked per container.
            div id="docker-section" class="hidden" {
                div class="mb-3 mt-4 flex items-center justify-between" {
                    h2 class="text-sm font-semibold uppercase tracking-wide text-slate-400" { "Docker containers" }
                    input id="dfilter" class="input max-w-[200px] py-1 text-xs" placeholder="Filter containers…" oninput="filterDocker(this.value)" {}
                }
                div class="cgrid grid gap-4 lg:grid-cols-2" {
                    (chart_card("dcpu", "Docker CPU", "CPU per container"))
                    (chart_card("dmem", "Docker Memory", "Memory per container"))
                    (chart_card("dnet", "Docker Network", "Network I/O per container"))
                }
            }
            // GPU section (hidden when no GPU reported).
            div id="gpu-section" class="hidden" {
                h2 class="mb-3 mt-4 text-sm font-semibold uppercase tracking-wide text-slate-400" { "GPU" }
                div class="cgrid grid gap-4 lg:grid-cols-2" {
                    (chart_card("gusage", "GPU Usage", "Utilization per GPU"))
                    (chart_card("gvram", "GPU VRAM", "Memory used per GPU"))
                    (chart_card("gpower", "GPU Power", "Power draw per GPU"))
                }
            }
            // Temperature sensors.
            div class="mt-4 card" id="temp-card" {
                h2 class="mb-1 text-sm font-medium text-slate-200" { "Temperature" } p class="mb-3 text-xs text-slate-500" { "System sensors (°C)" } div id="temp" {}
            }
            script { (PreEscaped(format!("const SID='{id}';{}", CHART_JS))) }
        },
    )
    .into_response()
}

/// Masks a token for display: first 8 + last 4 chars (full value only in the copy
/// button). 8 leading chars so a shared prefix doesn't hide the distinguishing part.
fn mask_token(t: &str) -> String {
    let n = t.chars().count();
    if n <= 14 {
        let keep = n.saturating_sub(4).min(4);
        let first: String = t.chars().take(keep).collect();
        return format!("{first}{}", "•".repeat(n - keep));
    }
    let first: String = t.chars().take(8).collect();
    let last: String = t.chars().skip(n - 4).collect();
    format!("{first}…{last}")
}

fn chart_card(id: &str, title: &str, desc: &str) -> Markup {
    html! {
        div class="card" id={(id) "-card"} {
            h2 class="mb-1 text-sm font-medium text-slate-200" { (title) }
            p class="mb-3 text-xs text-slate-500" { (desc) }
            div id=(id) {}
        }
    }
}

fn uptime_str(secs: i64) -> String {
    let d = secs / 86400;
    if d > 0 {
        format!("{d} days")
    } else {
        format!("{}h", secs / 3600)
    }
}

const CHART_JS: &str = r#"
let RANGE='1h';
const charts={};  // id -> { u, sig }
const palette=['#34E1C4','#34d399','#f59e0b','#38bdf8','#fb7185','#a78bfa','#f472b6','#4ade80','#facc15','#22d3ee','#c084fc','#fca5a5'];
function fmtB(v){ if(v==null) return '–'; const u=['B','K','M','G']; let i=0; while(v>=1024&&i<3){v/=1024;i++;} return v.toFixed(i?1:0)+u[i]; }
const pct=(u,v)=>v==null?'–':v.toFixed(0)+'%';
const dpct=(u,v)=>v==null?'–':v.toFixed(1)+'%';
const tempf=(u,v)=>v==null?'–':v.toFixed(0)+'°C';
function w(id){ const e=document.getElementById(id); return Math.max(260, (e&&e.clientWidth||520)); }
const tfmt=(u,v)=>v==null?'--':new Date(v*1000).toLocaleTimeString();
function base(id, capPct){
  return {width: w(id), height: 190,
    legend:{show:true},
    cursor:{points:{size:5}, focus:{prox:30}},
    scales:{x:{time:true}, y: capPct ? {range:(u,mn,mx)=>[0, Math.max(mx||1, 100)]} : {}},
    axes:[{stroke:'#64748b',grid:{stroke:'#1f2a44',width:1},ticks:{stroke:'#1f2a44'}},
          {stroke:'#64748b',grid:{stroke:'#1f2a44',width:1},ticks:{stroke:'#1f2a44'}}]};
}
const wattf=(u,v)=>v==null?'–':v.toFixed(0)+'W';
function show(id,on){ const e=document.getElementById(id); if(e) e.classList.toggle('hidden', !on); }
// Create the chart once; afterwards only update data (keeps hover + avoids layout jumps).
function upsert(id, sig, optsFn, data){
  const el=document.getElementById(id); if(!el) return;
  const c=charts[id];
  if(c && c.sig===sig){ c.u.setData(data); return; }
  if(c){ c.u.destroy(); }
  charts[id]={ u:new uPlot(optsFn(), data, el), sig };
}
function lineOpts(id, capPct, sers){ return ()=>{ const o=base(id,capPct); o.series=[{value:tfmt}].concat(sers); return o; }; }
function stack(t, series){
  const n=t.length; const cum=series.map(()=>new Array(n).fill(null));
  for(let x=0;x<n;x++){ let acc=0; for(let i=0;i<series.length;i++){ const v=series[i].data[x]; if(v!=null){acc+=v;} cum[i][x]=acc; } }
  return cum;
}
function renderSystem(d){
  if(!d) return;
  upsert('cpu','cpu', lineOpts('cpu',true,[{label:'CPU',stroke:'#34E1C4',fill:'#34E1C422',width:2,points:{show:false},value:pct}]), [d.t,d.cpu]);
  upsert('mem','mem', lineOpts('mem',true,[{label:'Memory',stroke:'#34d399',fill:'#34d39922',width:2,points:{show:false},value:pct}]), [d.t,d.mem_pct]);
  upsert('disk','disk', lineOpts('disk',true,[{label:'Disk',stroke:'#f59e0b',fill:'#f59e0b22',width:2,points:{show:false},value:pct}]), [d.t,d.disk_pct]);
  upsert('dio','dio', lineOpts('dio',false,[
      {label:'Read',stroke:'#a78bfa',fill:'#a78bfa22',width:2,points:{show:false},value:(u,v)=>fmtB(v)},
      {label:'Write',stroke:'#fbbf24',width:2,points:{show:false},value:(u,v)=>fmtB(v)}]), [d.t,d.dr,d.dw]);
  upsert('net','net', lineOpts('net',false,[
      {label:'Download',stroke:'#38bdf8',fill:'#38bdf822',width:2,points:{show:false},value:(u,v)=>fmtB(v)},
      {label:'Upload',stroke:'#fb7185',width:2,points:{show:false},value:(u,v)=>fmtB(v)}]), [d.t,d.net_rx,d.net_tx]);
}
function renderStack(id, t, series, fmt){
  if(!series||!series.length) return;
  const names=series.map(s=>s.name); const sig=id+':'+names.join('|');
  const cum=stack(t, series);
  const sers=[], data=[t];
  for(let i=series.length-1;i>=0;i--){ const col=palette[i%palette.length];
    sers.push({label:series[i].name,stroke:col,fill:col+'33',width:1,points:{show:false},value:fmt}); data.push(cum[i]); }
  upsert(id, sig, lineOpts(id,false,sers), data);
}
function renderLines(id, capPct, t, series, fmt){
  if(!series||!series.length) return;
  const sig=id+':'+series.map(s=>s.name).join('|');
  const sers=[], data=[t];
  series.forEach((s,i)=>{ const col=palette[i%palette.length]; sers.push({label:s.name,stroke:col,width:2,points:{show:false},value:fmt}); data.push(s.data); });
  upsert(id, sig, lineOpts(id,capPct,sers), data);
}
async function jget(u){ try{ const r=await fetch(u); return r.ok? await r.json():null; }catch(e){ return null; } }
async function draw(){
  const [d,c,tp,g] = await Promise.all([
    jget(`/api/servers/${SID}/metrics?range=${RANGE}`),
    jget(`/api/servers/${SID}/containers?range=${RANGE}`),
    jget(`/api/servers/${SID}/temps?range=${RANGE}`),
    jget(`/api/servers/${SID}/gpu?range=${RANGE}`),
  ]);
  renderSystem(d);
  const hasDocker = c && c.cpu && c.cpu.length>0;
  show('docker-section', hasDocker);
  if(hasDocker){ renderStack('dcpu', c.t, c.cpu, dpct); renderStack('dmem', c.t, c.mem, (u,v)=>fmtB(v)); renderStack('dnet', c.t, c.net, (u,v)=>fmtB(v)); }
  const hasTemp = tp && tp.series && tp.series.length>0;
  show('temp-card', hasTemp);
  if(hasTemp){ renderLines('temp', false, tp.t, tp.series, tempf); }
  const hasGpu = g && g.usage && g.usage.length>0;
  show('gpu-section', hasGpu);
  if(hasGpu){ renderLines('gusage', true, g.t, g.usage, pct); renderLines('gvram', true, g.t, g.vram, pct); renderLines('gpower', false, g.t, g.power, wattf); }
}
function setRange(r){ RANGE=r; document.querySelectorAll('.rng').forEach(b=>b.classList.toggle('!text-teal', b.dataset.range===r)); draw(); }
let SINGLE=false;
function toggleLayout(){ SINGLE=!SINGLE;
  document.querySelectorAll('.cgrid').forEach(g=>{ g.classList.toggle('lg:grid-cols-2', !SINGLE); g.classList.toggle('lg:grid-cols-1', SINGLE); });
  setTimeout(()=>{ for(const id in charts){ charts[id].u.setSize({width:w(id),height:190}); } }, 60);
}
function filterDocker(q){ q=(q||'').toLowerCase();
  ['dcpu','dmem','dnet'].forEach(id=>{ const c=charts[id]; if(!c)return;
    for(let i=1;i<c.u.series.length;i++){ const lbl=(c.u.series[i].label||'').toLowerCase(); c.u.setSeries(i,{show: lbl.includes(q)}); } });
}
let rt; addEventListener('resize',()=>{clearTimeout(rt); rt=setTimeout(()=>{ for(const id in charts){ charts[id].u.setSize({width:w(id),height:190}); } },150);});
setRange('1h'); setInterval(draw, 5000);
"#;

// ---- public status page -----------------------------------------------------

pub async fn public_status(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let page: Option<(Uuid, String, sqlx::types::Json<serde_json::Value>)> = sqlx::query_as(
        "SELECT namespace_id, title, config FROM status_pages \
         WHERE slug = $1 AND is_public = true",
    )
    .bind(&slug)
    .fetch_optional(&state.config)
    .await
    .ok()
    .flatten();

    let Some((ns, title, config)) = page else {
        return (StatusCode::NOT_FOUND, "status page not found").into_response();
    };

    // Optional monitor_ids filter; otherwise all monitors in the namespace.
    let ids: Vec<Uuid> = config
        .0
        .get("monitor_ids")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().and_then(|s| s.parse().ok()))
                .collect()
        })
        .unwrap_or_default();

    let monitors: Vec<(Uuid, String)> = if ids.is_empty() {
        sqlx::query_as("SELECT id, name FROM monitors WHERE namespace_id = $1 ORDER BY name")
            .bind(ns)
            .fetch_all(&state.config)
            .await
    } else {
        sqlx::query_as("SELECT id, name FROM monitors WHERE id = ANY($1) ORDER BY name")
            .bind(&ids)
            .fetch_all(&state.config)
            .await
    }
    .unwrap_or_default();

    // Per monitor: recent beats for the bar + uptime% + latest state.
    let mut rows: Vec<(String, Option<bool>, f64, Vec<bool>)> = Vec::new();
    for (id, name) in monitors {
        let beats: Vec<(bool,)> = sqlx::query_as(
            "SELECT up FROM heartbeats WHERE monitor_id = $1 ORDER BY time DESC LIMIT 32",
        )
        .bind(id)
        .fetch_all(&state.data)
        .await
        .unwrap_or_default();
        let up = beats.first().map(|(u,)| *u);
        let total = beats.len();
        let ups = beats.iter().filter(|(u,)| *u).count();
        let uptime = if total > 0 {
            ups as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        let bars: Vec<bool> = beats.iter().rev().map(|(u,)| *u).collect();
        rows.push((name, up, uptime, bars));
    }

    let all_up = !rows.is_empty() && rows.iter().all(|(_, u, _, _)| matches!(u, Some(true)));
    layout(
        &title,
        None,
        html! {
            div class="mx-auto max-w-3xl px-4 py-14" {
                div class="mb-7 text-center" {
                    @if rows.is_empty() {
                        span class="pill bg-slate-500/20 text-slate-300" { "No monitors" }
                    } @else if all_up {
                        span class="inline-flex items-center gap-2.5 rounded-full border border-teal/30 bg-teal/10 px-4 py-2 text-sm font-semibold text-teal" {
                            span class="dot pulse-dot bg-teal" style="width:9px;height:9px" {} "All systems operational"
                        }
                    } @else {
                        span class="inline-flex items-center gap-2.5 rounded-full border border-rose-500/30 bg-rose-500/10 px-4 py-2 text-sm font-semibold text-rose-300" {
                            span class="dot bg-rose-500" {} "Partial outage"
                        }
                    }
                    h1 class="mt-4 text-2xl font-semibold tracking-tight" { (title) }
                    p class="mt-1 text-sm text-slate-400" { "Live status of our services" }
                }
                div class="card p-0" {
                    @for (name, up, uptime, bars) in &rows {
                        div class="grid items-center gap-4 border-t border-white/5 px-5 py-4 first:border-t-0"
                            style="grid-template-columns:1fr 220px 64px 104px" {
                            div class="flex min-w-0 items-center gap-2.5" {
                                span class={"dot " (match up { Some(true)=>"bg-teal", Some(false)=>"bg-rose-500", None=>"bg-slate-600" })} {}
                                span class="truncate font-medium" { (name) }
                            }
                            div class="flex h-[26px] items-stretch gap-[2px]" {
                                @for u in bars {
                                    span class={"min-w-[2px] flex-1 rounded-sm " (if *u { "bg-teal" } else { "bg-rose-500" })} {}
                                }
                            }
                            span class="num text-right text-xs text-slate-400" { (format!("{uptime:.1}%")) }
                            span class={"text-right text-xs font-semibold " (if matches!(up, Some(false)) { "text-rose-300" } else { "text-teal" })} {
                                (if matches!(up, Some(false)) { "Down" } else { "Operational" })
                            }
                        }
                    }
                }
                p class="mt-4 text-center text-xs text-slate-500" { "powered by Last Monitor" }
            }
        },
    )
    .into_response()
}

// ---- manage (tabbed sub-sections) -------------------------------------------

async fn user_namespaces(state: &AppState, user: &CurrentUser) -> Vec<(Uuid, String)> {
    let q = if user.is_admin {
        sqlx::query_as::<_, (Uuid, String)>("SELECT id, name FROM namespaces ORDER BY name")
            .fetch_all(&state.config)
            .await
    } else {
        sqlx::query_as::<_, (Uuid, String)>(
            "SELECT n.id, n.name FROM namespaces n \
             JOIN memberships m ON m.namespace_id = n.id WHERE m.user_id = $1 ORDER BY n.name",
        )
        .bind(user.id)
        .fetch_all(&state.config)
        .await
    };
    q.unwrap_or_default()
}

const MANAGE_TABS: &[(&str, &str)] = &[
    ("namespaces", "Namespaces"),
    ("servers", "Servers"),
    ("monitors", "Monitors"),
    ("notifications", "Notifications"),
    ("members", "Members"),
    ("status", "Status pages"),
    ("users", "Users"),
    ("data", "Data"),
];

/// Tabs only admins may see.
fn admin_only_tab(slug: &str) -> bool {
    slug == "users" || slug == "data"
}

fn manage_layout(active: &str, user: &CurrentUser, content: Markup) -> Markup {
    layout(
        "Manage",
        Some(user),
        html! {
            div class="flex flex-col gap-6 md:flex-row" {
                aside class="md:w-48 shrink-0" {
                    nav class="flex flex-row flex-wrap gap-1 md:flex-col" {
                        @for (slug, label) in MANAGE_TABS {
                            @if !admin_only_tab(slug) || user.is_admin {
                                a href={"/manage/"(slug)}
                                  class=(if active == *slug {
                                      "rounded-md bg-teal/15 px-3 py-2 text-sm font-medium text-teal"
                                  } else {
                                      "rounded-md px-3 py-2 text-sm text-slate-400 hover:bg-line hover:text-slate-200"
                                  }) { (label) }
                            }
                        }
                    }
                }
                div class="min-w-0 flex-1" { (content) }
            }
            script { (PreEscaped(JPOST_JS)) }
        },
    )
}

fn ns_select(nss: &[(Uuid, String)]) -> Markup {
    html! {
        select class="input" name="_ns" required {
            @for (id, name) in nss { option value=(id) { (name) } }
        }
    }
}

fn section_card(title: &str, form: Markup, list: Markup) -> Markup {
    html! {
        h1 class="mb-4 text-xl font-semibold" { (title) }
        div class="card mb-4" { (form) }
        div class="card p-0 overflow-x-auto" { (list) }
    }
}

macro_rules! redirect_if_anon {
    ($user:expr) => {
        match $user {
            Some(u) => u,
            None => return Redirect::to("/login").into_response(),
        }
    };
}

pub async fn manage_redirect() -> Redirect {
    Redirect::to("/manage/namespaces")
}

pub async fn manage_namespaces(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Response {
    let user = redirect_if_anon!(user);
    let nss = user_namespaces(&state, &user).await;
    let content = section_card(
        "Namespaces",
        html! {
            form data-url="/api/namespaces" onsubmit="return jpost(event)" class="flex flex-wrap items-center gap-2" {
                input class="input max-w-[260px]" name="name" placeholder="name (lowercase, e.g. team-a)" required {}
                button class="btn" type="submit" { "Create namespace" }
            }
            p class="mt-2 text-xs text-slate-500" { "DNS-label style: lowercase letters, digits, hyphens." }
        },
        html! {
            table class="w-full" {
                thead { tr class="border-b border-line" { th class="th" { "Name" } th class="th" { "ID" } } }
                tbody {
                    @if nss.is_empty() { tr { td class="td text-slate-400" colspan="2" { "None yet." } } }
                    @for (id, name) in &nss {
                        tr { td class="td font-medium" { (name) } td class="td text-xs text-slate-500" { (id) } }
                    }
                }
            }
        },
    );
    manage_layout("namespaces", &user, content).into_response()
}

pub async fn manage_servers(State(state): State<AppState>, user: Option<CurrentUser>) -> Response {
    let user = redirect_if_anon!(user);
    let nss = user_namespaces(&state, &user).await;
    // Reusable tokens (with how many servers each enrolled).
    let tokens: Vec<(Uuid, String, String, i64, String)> = sqlx::query_as(
        "SELECT t.id, t.name, t.token, count(s.id), n.name \
         FROM enrollment_tokens t JOIN namespaces n ON n.id = t.namespace_id \
         LEFT JOIN servers s ON s.token_id = t.id \
         WHERE $1 OR t.namespace_id IN (SELECT namespace_id FROM memberships WHERE user_id = $2) \
         GROUP BY t.id, n.name ORDER BY n.name, t.name",
    )
    .bind(user.is_admin)
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .unwrap_or_default();
    // Auto-registered servers.
    let servers: Vec<(Uuid, String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT s.id, s.name, s.hostname, s.agent_version, n.name \
         FROM servers s JOIN namespaces n ON n.id = s.namespace_id \
         WHERE $1 OR s.namespace_id IN (SELECT namespace_id FROM memberships WHERE user_id = $2) \
         ORDER BY n.name, s.hostname",
    )
    .bind(user.is_admin)
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .unwrap_or_default();

    let content = html! {
        h1 class="mb-4 text-xl font-semibold" { "Servers & tokens" }
        // Tokens
        div class="card mb-4" {
            h2 class="mb-3 text-sm font-semibold text-slate-300" { "Enrollment tokens" }
            form data-url="/api/namespaces/__NS__/tokens" onsubmit="return jpost(event)" class="flex flex-wrap items-end gap-2" {
                (ns_select(&nss))
                input class="input max-w-[220px]" name="name" placeholder="token name (e.g. prod-nodes)" maxlength="32" required {}
                button class="btn" type="submit" { (icon("plus")) "Create token" }
            }
            p class="mt-2 text-xs text-slate-500" { "One token enrolls many servers — reuse it across a fleet / k8s DaemonSet." }
        }
        div class="card mb-6 overflow-x-auto p-0" {
            table class="w-full" {
                thead { tr class="border-b border-line" {
                    th class="th" { "Namespace" } th class="th" { "Token name" } th class="th" { "Token" } th class="th" { "Servers" } th class="th" { "" }
                } }
                tbody {
                    @if tokens.is_empty() { tr { td class="td text-slate-400" colspan="5" { "No tokens yet." } } }
                    @for (id, name, token, count, nsn) in &tokens {
                        tr {
                            td class="td text-slate-400" { (nsn) }
                            td class="td font-medium" { span class="block max-w-[220px] truncate" title=(name) { (name) } }
                            td class="td" {
                                code class="rounded bg-ink px-1.5 py-0.5 text-xs text-teal" title="hidden for security" { (mask_token(token)) }
                                button class="ml-1 text-slate-500 hover:text-slate-300" title="Copy token" onclick=(format!("copyTxt('{token}')")) { (icon("copy")) }
                            }
                            td class="td" { (count) }
                            td class="td text-right" {
                                button class="btn-ghost text-xs text-rose-400" onclick=(format!("delToken('{id}')")) { (icon("trash")) "Delete" }
                            }
                        }
                    }
                }
            }
        }
        // Servers (auto-registered)
        h2 class="mb-3 text-sm font-semibold text-slate-300" { "Registered servers" }
        div class="card overflow-x-auto p-0" {
            table class="w-full" {
                thead { tr class="border-b border-line" {
                    th class="th" { "Namespace" } th class="th" { "Name" } th class="th" { "Hostname" } th class="th" { "Agent" } th class="th" { "" }
                } }
                tbody {
                    @if servers.is_empty() { tr { td class="td text-slate-400" colspan="5" { "No servers have reported yet." } } }
                    @for (id, name, host, ver, nsn) in &servers {
                        tr {
                            td class="td text-slate-400" { (nsn) }
                            td class="td font-medium" { (name) }
                            td class="td text-slate-400" { (host) }
                            td class="td text-slate-400" { (ver.clone().unwrap_or_else(|| "—".into())) }
                            td class="td whitespace-nowrap text-right" {
                                button class="btn-ghost text-xs" onclick=(format!("renameServer('{id}','{name}')")) { "Rename" }
                                button class="btn-ghost text-xs text-rose-400" onclick=(format!("jdelete('/api/servers/{id}','Remove server row {host}? (re-registers if its agent is still running)')")) { "Remove" }
                            }
                        }
                    }
                }
            }
        }
    };
    manage_layout("servers", &user, content).into_response()
}

pub async fn manage_monitors(State(state): State<AppState>, user: Option<CurrentUser>) -> Response {
    let user = redirect_if_anon!(user);
    let nss = user_namespaces(&state, &user).await;
    let monitors: Vec<(Uuid, String, String, String, i32, bool, String)> = sqlx::query_as(
        "SELECT m.id, m.name, m.kind::text, m.target, m.interval_secs, m.enabled, n.name \
         FROM monitors m JOIN namespaces n ON n.id = m.namespace_id \
         WHERE $1 OR m.namespace_id IN (SELECT namespace_id FROM memberships WHERE user_id = $2) \
         ORDER BY n.name, m.name",
    )
    .bind(user.is_admin)
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .unwrap_or_default();

    let content = section_card(
        "Monitors",
        html! {
            form data-url="/api/namespaces/__NS__/monitors" onsubmit="return jpost(event)" class="grid gap-2 sm:grid-cols-2 lg:grid-cols-6" {
                (ns_select(&nss))
                input class="input" name="name" placeholder="name" required {}
                select class="input" name="kind" required {
                    option value="http" { "http" } option value="tcp" { "tcp" }
                    option value="ping" { "ping" } option value="keyword" { "keyword" }
                }
                input class="input" name="target" placeholder="url / host:port / host" required {}
                input class="input" name="interval_secs" type="number" placeholder="interval s" value="60" {}
                button class="btn" type="submit" { "Add monitor" }
            }
        },
        html! {
            table class="w-full" {
                thead { tr class="border-b border-line" {
                    th class="th" { "Namespace" } th class="th" { "Name" } th class="th" { "Kind" }
                    th class="th" { "Target" } th class="th" { "Interval" } th class="th" { "State" } th class="th" { "" }
                } }
                tbody {
                    @if monitors.is_empty() { tr { td class="td text-slate-400" colspan="7" { "None yet." } } }
                    @for (id, name, kind, target, iv, enabled, ns) in &monitors {
                        tr {
                            td class="td text-slate-400" { (ns) } td class="td font-medium" { (name) }
                            td class="td text-slate-400" { (kind) } td class="td text-slate-400" { (target) }
                            td class="td text-slate-400" { (iv) "s" }
                            td class="td" {
                                @if *enabled { span class="pill bg-teal/20 text-teal" { "enabled" } }
                                @else { span class="pill bg-slate-500/20 text-slate-400" { "paused" } }
                            }
                            td class="td whitespace-nowrap text-right" {
                                button class="btn-ghost text-xs" onclick=(format!("toggleMon('{id}',{})", !enabled)) {
                                    @if *enabled { "Pause" } @else { "Resume" }
                                }
                                button class="btn-ghost text-xs" onclick=(format!("editMon('{id}','{name}','{target}',{iv})")) { "Edit" }
                                button class="btn-ghost text-xs text-rose-400" onclick=(format!("jdelete('/api/monitors/{id}','Delete monitor {name}?')")) { "Delete" }
                            }
                        }
                    }
                }
            }
        },
    );
    manage_layout("monitors", &user, content).into_response()
}

pub async fn manage_notifications(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Response {
    let user = redirect_if_anon!(user);
    let nss = user_namespaces(&state, &user).await;
    let channels: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT c.id, c.name, c.kind, n.name FROM notification_channels c \
         JOIN namespaces n ON n.id = c.namespace_id \
         WHERE $1 OR c.namespace_id IN (SELECT namespace_id FROM memberships WHERE user_id = $2) \
         ORDER BY n.name, c.name",
    )
    .bind(user.is_admin)
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .unwrap_or_default();

    manage_layout(
        "notifications",
        &user,
        html! {
            h1 class="mb-4 text-xl font-semibold" { "Notifications" }
            div class="card mb-4" {
                h2 class="mb-3 text-sm font-semibold text-slate-300" { "New channel" }
                form data-url="/api/namespaces/__NS__/channels" onsubmit="return jpost(event)" class="grid gap-2 sm:grid-cols-2 lg:grid-cols-5" {
                    (ns_select(&nss))
                    input class="input" name="name" placeholder="name" required {}
                    select class="input" name="kind" required {
                        option value="webhook" { "webhook" } option value="telegram" { "telegram" } option value="email" { "email" }
                    }
                    input class="input lg:col-span-1" name="config" placeholder=r#"config JSON {"url":"…"}"# {}
                    button class="btn" type="submit" { "Add channel" }
                }
            }
            div class="card mb-4" {
                h2 class="mb-3 text-sm font-semibold text-slate-300" { "New alert rule" }
                form data-url="/api/namespaces/__NS__/alerts" onsubmit="return jpost(event)" class="grid gap-2 sm:grid-cols-2 lg:grid-cols-3" {
                    (ns_select(&nss))
                    input class="input" name="monitor_id" placeholder="monitor_id (or leave blank)" {}
                    input class="input" name="server_id" placeholder="server_id (or leave blank)" {}
                    input class="input" name="channel_id" placeholder="channel_id (see list)" required {}
                    input class="input" name="condition" placeholder=r#"condition JSON {"metric":"cpu_percent","op":">","value":90}"# {}
                    input class="input" name="cooldown_secs" type="number" placeholder="cooldown s" value="300" {}
                    button class="btn" type="submit" { "Add alert" }
                }
            }
            div class="card p-0 overflow-x-auto" {
                table class="w-full" {
                    thead { tr class="border-b border-line" {
                        th class="th" { "Namespace" } th class="th" { "Channel" } th class="th" { "Kind" } th class="th" { "Channel ID" } th class="th" { "" }
                    } }
                    tbody {
                        @if channels.is_empty() { tr { td class="td text-slate-400" colspan="5" { "No channels yet." } } }
                        @for (id, name, kind, ns) in &channels {
                            tr {
                                td class="td text-slate-400" { (ns) } td class="td font-medium" { (name) }
                                td class="td text-slate-400" { (kind) }
                                td class="td" { code class="rounded bg-ink px-1.5 py-0.5 text-xs text-slate-300" { (id) } }
                                td class="td text-right" {
                                    button class="btn-ghost text-xs text-rose-400" onclick=(format!("jdelete('/api/channels/{id}','Delete channel {name}?')")) { "Delete" }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
    .into_response()
}

pub async fn manage_members(State(state): State<AppState>, user: Option<CurrentUser>) -> Response {
    let user = redirect_if_anon!(user);
    let nss = user_namespaces(&state, &user).await;
    let members: Vec<(Uuid, String, Uuid, String, String)> = sqlx::query_as(
        "SELECT n.id, n.name, u.id, u.email, m.role::text FROM memberships m \
         JOIN users u ON u.id = m.user_id JOIN namespaces n ON n.id = m.namespace_id \
         WHERE $1 OR m.namespace_id IN (SELECT namespace_id FROM memberships WHERE user_id = $2) \
         ORDER BY n.name, u.email",
    )
    .bind(user.is_admin)
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .unwrap_or_default();

    let content = section_card(
        "Members",
        html! {
            form data-url="/api/namespaces/__NS__/members" onsubmit="return jpost(event)" class="flex flex-wrap items-end gap-2" {
                (ns_select(&nss))
                input class="input max-w-[220px]" name="email" type="email" placeholder="user email" required {}
                select class="input max-w-[140px]" name="role" required {
                    option value="viewer" { "viewer" } option value="editor" { "editor" } option value="owner" { "owner" }
                }
                button class="btn" type="submit" { "Add / update member" }
            }
        },
        html! {
            table class="w-full" {
                thead { tr class="border-b border-line" {
                    th class="th" { "Namespace" } th class="th" { "User" } th class="th" { "Role" } th class="th" { "" }
                } }
                tbody {
                    @if members.is_empty() { tr { td class="td text-slate-400" colspan="4" { "None yet." } } }
                    @for (ns_id, ns, uid, email, role) in &members {
                        tr { td class="td text-slate-400" { (ns) } td class="td" { (email) }
                             td class="td" { span class="pill bg-slate-500/20 text-slate-300" { (role) } }
                             td class="td text-right" {
                                button class="btn-ghost text-xs text-rose-400" onclick=(format!("jdelete('/api/namespaces/{ns_id}/members/{uid}','Remove {email} from {ns}?')")) { "Remove" }
                             } }
                    }
                }
            }
        },
    );
    manage_layout("members", &user, content).into_response()
}

pub async fn manage_status(State(state): State<AppState>, user: Option<CurrentUser>) -> Response {
    let user = redirect_if_anon!(user);
    let nss = user_namespaces(&state, &user).await;
    let pages: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT sp.id, sp.slug, sp.title, n.name FROM status_pages sp \
         JOIN namespaces n ON n.id = sp.namespace_id \
         WHERE $1 OR sp.namespace_id IN (SELECT namespace_id FROM memberships WHERE user_id = $2) \
         ORDER BY n.name, sp.slug",
    )
    .bind(user.is_admin)
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .unwrap_or_default();

    let content = section_card(
        "Status pages",
        html! {
            form data-url="/api/namespaces/__NS__/status-pages" onsubmit="return jpost(event)" class="flex flex-wrap items-end gap-2" {
                (ns_select(&nss))
                input class="input max-w-[200px]" name="slug" placeholder="slug" required {}
                input class="input max-w-[240px]" name="title" placeholder="title" required {}
                button class="btn" type="submit" { "Create page" }
            }
        },
        html! {
            table class="w-full" {
                thead { tr class="border-b border-line" {
                    th class="th" { "Namespace" } th class="th" { "Title" } th class="th" { "Public URL" } th class="th" { "" }
                } }
                tbody {
                    @if pages.is_empty() { tr { td class="td text-slate-400" colspan="4" { "None yet." } } }
                    @for (id, slug, title, ns) in &pages {
                        tr { td class="td text-slate-400" { (ns) } td class="td font-medium" { (title) }
                             td class="td" { a class="text-teal hover:underline" href={"/status/"(slug)} { "/status/" (slug) } }
                             td class="td text-right" {
                                button class="btn-ghost text-xs text-rose-400" onclick=(format!("jdelete('/api/status-pages/{id}','Delete status page {title}?')")) { "Delete" }
                             } }
                    }
                }
            }
        },
    );
    manage_layout("status", &user, content).into_response()
}

pub async fn manage_users(State(state): State<AppState>, user: Option<CurrentUser>) -> Response {
    let user = redirect_if_anon!(user);
    if !user.is_admin {
        return StatusCode::FORBIDDEN.into_response();
    }
    let users: Vec<(String, bool)> =
        sqlx::query_as("SELECT email, is_admin FROM users ORDER BY email")
            .fetch_all(&state.config)
            .await
            .unwrap_or_default();

    let content = section_card(
        "Users",
        html! {
            form data-url="/api/users" onsubmit="return jpost(event)" class="flex flex-wrap items-end gap-2" {
                input class="input max-w-[240px]" name="email" type="email" placeholder="email" required {}
                input class="input max-w-[180px]" name="password" type="password" placeholder="password" required {}
                button class="btn" type="submit" { "Create user" }
            }
        },
        html! {
            table class="w-full" {
                thead { tr class="border-b border-line" { th class="th" { "Email" } th class="th" { "Role" } } }
                tbody {
                    @for (email, is_admin) in &users {
                        tr { td class="td" { (email) }
                             td class="td" { @if *is_admin { span class="pill bg-teal/20 text-teal" { "admin" } } @else { span class="text-slate-400" { "user" } } } }
                    }
                }
            }
        },
    );
    manage_layout("users", &user, content).into_response()
}

pub async fn manage_data(State(state): State<AppState>, user: Option<CurrentUser>) -> Response {
    let user = redirect_if_anon!(user);
    if !user.is_admin {
        return StatusCode::FORBIDDEN.into_response();
    }
    let s = crate::data_admin::stats(&state.data).await;
    let content = html! {
        h1 class="mb-1 text-xl font-semibold" { "Data" }
        p class="mb-4 text-sm text-slate-400" { "Data database size: " span class="font-semibold text-slate-200" { (s.db_size) } }

        h2 class="mb-2 text-sm font-semibold text-slate-300" { "Storage by table" }
        div class="card mb-6 p-0 overflow-x-auto" {
            table class="w-full" {
                thead { tr class="border-b border-line" { th class="th" { "Table" } th class="th" { "Size" } th class="th" { "Rows" } } }
                tbody {
                    @for t in &s.tables {
                        tr { td class="td font-medium" { (t.name) } td class="td text-slate-300" { (t.size) }
                             td class="td text-slate-400 tabular-nums" { (t.rows) } }
                    }
                }
            }
        }

        h2 class="mb-2 text-sm font-semibold text-slate-300" { "Retention & downsampling" }
        p class="mb-3 text-xs text-slate-500" { "Raw samples are rolled up into 1-minute and 1-hour aggregates automatically. Set how long each tier is kept (days)." }
        div class="card p-0 overflow-x-auto" {
            table class="w-full" {
                thead { tr class="border-b border-line" { th class="th" { "Tier" } th class="th" { "Keep (days)" } th class="th" { "" } } }
                tbody {
                    @for r in &s.retention {
                        tr {
                            td class="td font-medium" { (r.label) }
                            td class="td" {
                                input id={"ret-"(r.table)} class="input max-w-[110px] py-1 text-sm" type="number" min="1"
                                    value=(r.days.map(|d| d.to_string()).unwrap_or_default()) placeholder="∞" {}
                            }
                            td class="td text-right" {
                                button class="btn-ghost text-xs" onclick=(format!("setRet('{}')", r.table)) { "Save" }
                            }
                        }
                    }
                }
            }
        }
    };
    manage_layout("data", &user, content).into_response()
}

const JPOST_JS: &str = r#"
async function jpost(e){
  e.preventDefault();
  const f=e.target; const o={}; let url=f.dataset.url;
  for(const el of f.elements){
    if(!el.name) continue;
    if(el.name==='_ns'){ url=url.replace('__NS__', el.value); continue; }
    if(el.value==='') continue;
    if(el.name==='config' || el.name==='condition'){ try{o[el.name]=JSON.parse(el.value)}catch{alert('Invalid JSON in '+el.name);return false} }
    else if(el.name==='interval_secs' || el.name==='cooldown_secs') o[el.name]=Number(el.value);
    else o[el.name]=el.value;
  }
  const r=await fetch(url,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(o)});
  if(r.ok) location.reload(); else alert('Error '+r.status+': '+(await r.text()));
  return false;
}
async function jdelete(url,msg){
  if(!confirm(msg||'Delete?')) return;
  const r=await fetch(url,{method:'DELETE'});
  if(r.ok) location.reload(); else alert('Error '+r.status+': '+(await r.text()));
}
async function jpatch(url,obj){
  const r=await fetch(url,{method:'PATCH',headers:{'content-type':'application/json'},body:JSON.stringify(obj)});
  if(r.ok) location.reload(); else alert('Error '+r.status+': '+(await r.text()));
}
function renameServer(id,cur){ const n=prompt('New server name', cur); if(n&&n!==cur) jpatch('/api/servers/'+id,{name:n}); }
function setRet(table){ const v=document.getElementById('ret-'+table).value;
  fetch('/api/admin/retention',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({table,days:Number(v)})})
    .then(async r=>{ if(r.ok)location.reload(); else alert('Error '+r.status+': '+(await r.text())); }); }
function toggleMon(id,en){ jpatch('/api/monitors/'+id,{enabled:en}); }
function editMon(id,name,target,iv){
  const n=prompt('Name', name); if(n===null) return;
  const t=prompt('Target', target); if(t===null) return;
  const i=prompt('Interval (s)', iv); if(i===null) return;
  jpatch('/api/monitors/'+id,{name:n,target:t,interval_secs:Number(i)});
}
"#;

// ---- helpers ----------------------------------------------------------------

fn status_pill(up: Option<bool>) -> Markup {
    match up {
        Some(true) => html! { span class="pill bg-teal/20 text-teal" { "● up" } },
        Some(false) => html! { span class="pill bg-rose-500/20 text-rose-300" { "● down" } },
        None => html! { span class="pill bg-slate-500/20 text-slate-400" { "○ —" } },
    }
}

fn pct(used: i64, total: i64) -> f64 {
    if total > 0 {
        used as f64 / total as f64 * 100.0
    } else {
        0.0
    }
}

/// A colored usage bar + percentage. Green < 60, amber < 85, rose otherwise.
fn gauge(value: f64) -> Markup {
    let v = value.clamp(0.0, 100.0);
    let color = if v >= 85.0 {
        "bg-rose-500"
    } else if v >= 60.0 {
        "bg-amber-500"
    } else {
        "bg-teal"
    };
    html! {
        div class="flex items-center gap-2" {
            div class="gauge-track" {
                div class={"gauge-fill " (color)} style=(format!("width:{v:.0}%")) {}
            }
            span class="w-9 tabular-nums text-xs text-slate-300" { (format!("{v:.0}%")) }
        }
    }
}

/// Status dot color from how recently the server reported.
fn online_dot(last_seen: Option<chrono::DateTime<chrono::Utc>>) -> &'static str {
    match last_seen {
        Some(t) if (chrono::Utc::now() - t).num_seconds() < 60 => "bg-teal",
        Some(_) => "bg-rose-500",
        None => "bg-slate-600",
    }
}

fn fmt_bytes(v: f64) -> String {
    let units = ["B", "K", "M", "G"];
    let mut v = v;
    let mut i = 0;
    while v >= 1024.0 && i < 3 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{v:.0}{}", units[i])
    } else {
        format!("{v:.1}{}", units[i])
    }
}

/// Down/up arrows with per-second network rate from the two newest samples.
fn net_rate(
    samples: &[(
        chrono::DateTime<chrono::Utc>,
        f64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    )],
) -> Markup {
    let (rx, tx) = match (samples.first(), samples.get(1)) {
        (Some(new), Some(old)) => {
            let dt = (new.0 - old.0).num_seconds().max(1) as f64;
            (
                (new.6 - old.6).max(0) as f64 / dt,
                (new.7 - old.7).max(0) as f64 / dt,
            )
        }
        _ => (0.0, 0.0),
    };
    html! {
        span class="text-sky-400" { "↓ " (fmt_bytes(rx)) "/s" }
        " "
        span class="text-rose-400" { "↑ " (fmt_bytes(tx)) "/s" }
    }
}

fn ise<E: std::fmt::Display>(e: E) -> StatusCode {
    tracing::error!(error = %e, "ui DB error");
    StatusCode::INTERNAL_SERVER_ERROR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pct_handles_zero_total() {
        assert_eq!(pct(0, 0), 0.0);
        assert_eq!(pct(1, 2), 50.0);
        assert_eq!(pct(8, 8), 100.0);
    }

    #[test]
    fn fmt_bytes_units() {
        assert_eq!(fmt_bytes(512.0), "512B");
        assert_eq!(fmt_bytes(1024.0), "1.0K");
        assert_eq!(fmt_bytes(1024.0 * 1024.0), "1.0M");
        assert_eq!(fmt_bytes(1024.0 * 1024.0 * 1024.0), "1.0G");
    }

    #[test]
    fn online_dot_thresholds() {
        let now = chrono::Utc::now();
        assert_eq!(online_dot(Some(now)), "bg-teal");
        assert_eq!(
            online_dot(Some(now - chrono::Duration::minutes(5))),
            "bg-rose-500"
        );
        assert_eq!(online_dot(None), "bg-slate-600");
    }
}
