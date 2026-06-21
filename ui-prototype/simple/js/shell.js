// Shared app shell (sidebar + topbar) — NGUỒN DUY NHẤT cho mọi trang.
// Dùng: <div id="page" data-title="..." data-active="systems" data-root="."> NỘI DUNG </div>
//       <script src="js/shell.js"></script>   (hoặc ../js/shell.js từ pages/)
//   data-root: "." nếu trang ở simple/ (index.html), ".." nếu ở simple/pages/.
// Namespace filter dùng chung qua window.NS; trang nào cần lọc thì gán window.onNamespaceChange.
// Sau này phần này thành layout component trong Vue.
(function () {
  const page = document.getElementById('page');
  if (!page) return;
  const title = page.dataset.title || '';
  const active = page.dataset.active || '';
  const root = page.dataset.root || '..';
  const p = (rel) => `${root}/${rel}`;

  // ---- shared namespace filter state ----
  const ALL_NS = ['production', 'staging', 'edge'];
  const NS_COLOR = { production: 'bg-teal', staging: 'bg-amber-400', edge: 'bg-sky-400' };
  if (!window.NS) window.NS = { all: ALL_NS, colors: NS_COLOR, selected: new Set(ALL_NS) };

  const link = (href, key, label, icon) => `
    <a href="${href}" class="flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm transition ${
      active === key ? 'bg-teal/10 font-medium text-teal' : 'text-slate-400 hover:bg-panel2 hover:text-slate-100'
    }">${icon}${label}</a>`;

  const I = {
    servers: '<svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/></svg>',
    monitors: '<svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 12h-4l-3 9L9 3l-3 9H2"/></svg>',
    status: '<svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><path d="m9 12 2 2 4-4"/></svg>',
    ns: '<svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 7h18M3 12h18M3 17h18"/></svg>',
    srv: '<svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 6h16M4 12h16M4 18h10"/></svg>',
    notif: '<svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/></svg>',
    members: '<svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/></svg>',
    data: '<svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14a9 3 0 0 0 18 0V5"/></svg>',
  };

  const frame = document.createElement('div');
  frame.innerHTML = `
  <div class="flex min-h-screen">
    <div id="backdrop" class="fixed inset-0 z-30 hidden bg-black/60 md:hidden" onclick="toggleSidebar(false)"></div>
    <aside id="sidebar" class="fixed inset-y-0 left-0 z-40 flex w-60 shrink-0 -translate-x-full flex-col border-r border-line bg-panel transition-transform md:static md:translate-x-0">
      <div class="flex items-center gap-2.5 px-5 py-4">
        <span class="inline-block h-6 w-6 rounded-md bg-teal shadow-[0_0_18px_-4px_#34E1C4]"></span>
        <span class="text-base font-semibold tracking-tight text-slate-100">last-monitor</span>
      </div>
      <div id="nsSwitcher" class="px-3 pb-2"></div>
      <nav class="flex-1 space-y-1 overflow-y-auto px-3 py-2">
        ${link(p('index.html'), 'systems', 'Systems', I.servers)}
        <!-- Monitors & Status page: tạm gác, làm sau -->
        <div class="px-3 pb-1 pt-4 text-[11px] uppercase tracking-wider text-slate-600">Manage</div>
        ${link('#', 'mns', 'Namespaces', I.ns)}
        ${link('#', 'msys', 'Systems', I.srv)}
        ${link('#', 'notif', 'Notifications', I.notif)}
        ${link('#', 'members', 'Members', I.members)}
        ${link('#', 'data', 'Data & retention', I.data)}
      </nav>
      <div class="border-t border-line p-3">
        <div class="flex items-center gap-2.5 rounded-lg px-2 py-1.5">
          <span class="grid h-8 w-8 place-items-center rounded-full bg-panel2 text-xs text-teal">AD</span>
          <div class="min-w-0 flex-1"><div class="truncate text-sm text-slate-200">admin@example.com</div><div class="text-[11px] text-slate-600">Admin</div></div>
          <a href="${p('login.html')}" title="Logout" class="text-slate-500 hover:text-teal"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9"/></svg></a>
        </div>
      </div>
    </aside>

    <div class="flex min-w-0 flex-1 flex-col">
      <header class="flex items-center justify-between border-b border-line bg-panel/60 px-4 py-3 backdrop-blur sm:px-6">
        <div class="flex items-center gap-3">
          <button onclick="toggleSidebar(true)" class="rounded-lg border border-line bg-panel2 p-1.5 text-slate-400 hover:text-teal md:hidden">
            <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M3 12h18M3 18h18"/></svg>
          </button>
          <h1 class="text-lg font-semibold text-slate-100">${title}</h1>
        </div>
        <div class="flex items-center gap-3">
          <button class="flex items-center gap-2 rounded-lg border border-line bg-panel2 px-2.5 py-1.5 text-sm text-slate-500 hover:border-teal/50 sm:px-3">
            <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>
            <span class="hidden sm:inline">Search</span>
            <kbd class="hidden rounded border border-line bg-ink px-1.5 text-[11px] text-slate-500 sm:inline">⌘K</kbd>
          </button>
          <button id="themeBtn" onclick="toggleTheme()" title="Toggle theme" class="rounded-lg border border-line bg-panel2 p-1.5 text-slate-400 hover:text-teal"></button>
        </div>
      </header>
      <main id="main-slot" class="flex-1 p-4 sm:p-6"></main>
    </div>
  </div>`;

  // Move shell into <body> before #page, then relocate #page's content into the slot.
  document.body.insertBefore(frame.firstElementChild, page);
  const slot = document.getElementById('main-slot');
  while (page.firstChild) slot.appendChild(page.firstChild);
  page.remove();

  // ---- namespace switcher (multi-select) ----
  function nsSwitcherHTML() {
    const sel = window.NS.selected, all = sel.size === ALL_NS.length;
    const label = all ? 'All namespaces' : sel.size === 0 ? 'No namespace' : sel.size === 1 ? [...sel][0] : `${sel.size} namespaces`;
    const chevron = '<svg class="h-4 w-4 shrink-0 text-slate-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m6 9 6 6 6-6"/></svg>';
    const box = (on) => `<span class="grid h-4 w-4 shrink-0 place-items-center rounded border ${on ? 'border-teal bg-teal' : 'border-line'}">${on ? '<svg class="h-3 w-3 text-teal-fg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3"><path d="M20 6 9 17l-5-5"/></svg>' : ''}</span>`;
    const opts = ALL_NS.map(n => `
      <button type="button" onclick="nsToggle(event,'${n}')" class="flex w-full items-center gap-2.5 px-3 py-2 text-left text-sm hover:bg-panel ${sel.has(n) ? 'text-slate-100' : 'text-slate-400'}">
        ${box(sel.has(n))}<span class="h-1.5 w-1.5 rounded-full ${NS_COLOR[n] || 'bg-slate-500'}"></span>${n}
      </button>`).join('');
    return `<div class="relative">
      <button type="button" onclick="nsToggleMenu(event)" class="flex w-full items-center justify-between gap-2 rounded-lg border border-line bg-panel2 px-3 py-2 text-sm text-slate-300 hover:border-teal/50">
        <span class="flex min-w-0 items-center gap-2"><span class="h-2 w-2 shrink-0 rounded-full bg-teal"></span><span class="truncate">${label}</span></span>${chevron}
      </button>
      <div id="nsSwitchMenu" class="absolute left-0 right-0 z-30 mt-1 hidden overflow-hidden rounded-lg border border-line bg-panel2 py-1 shadow-xl">
        <button type="button" onclick="nsToggle(event,'__all__')" class="flex w-full items-center gap-2.5 border-b border-line px-3 py-2 text-left text-sm ${all ? 'text-teal' : 'text-slate-400'} hover:bg-panel">${box(all)}All namespaces</button>
        ${opts}
      </div>
    </div>`;
  }
  function renderNsSwitcher(open) {
    document.getElementById('nsSwitcher').innerHTML = nsSwitcherHTML();
    if (open) document.getElementById('nsSwitchMenu').classList.remove('hidden');
  }
  window.nsToggleMenu = function (e) { e.stopPropagation(); document.getElementById('nsSwitchMenu').classList.toggle('hidden'); };
  window.nsToggle = function (e, n) {
    e.stopPropagation();
    const sel = window.NS.selected;
    if (n === '__all__') { sel.size === ALL_NS.length ? sel.clear() : ALL_NS.forEach(x => sel.add(x)); }
    else { sel.has(n) ? sel.delete(n) : sel.add(n); }
    renderNsSwitcher(true);
    if (window.onNamespaceChange) window.onNamespaceChange();
  };
  renderNsSwitcher();

  window.toggleSidebar = function (open) {
    document.getElementById('sidebar').classList.toggle('-translate-x-full', !open);
    document.getElementById('backdrop').classList.toggle('hidden', !open);
  };

  // ---- light/dark theme ----
  // Prototype dùng màu tối hardcode (bg-ink/panel/text-slate…). Thay vì sửa markup,
  // ta override các class đó khi <html> có class .light. Lưu localStorage để đồng bộ mọi trang.
  const MOON = '<svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/></svg>';
  const SUN = '<svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg>';
  if (!document.getElementById('lm-theme-style')) {
    const st = document.createElement('style');
    st.id = 'lm-theme-style';
    st.textContent = `
      /* hàng bảng: hover/select rõ ràng ở cả 2 mode (left accent khi chọn) */
      .lm-row:hover{background:rgba(255,255,255,.055)}
      .lm-row.sel{background:rgba(52,225,196,.14);box-shadow:inset 3px 0 0 #34E1C4}
      html.light .lm-row:hover{background:#e0f2fe}
      html.light .lm-row.sel{background:#cffafe;box-shadow:inset 3px 0 0 #0d9488}

      html.light .bg-ink{background:#f1f5f9}
      html.light body{background:#f1f5f9}
      html.light .bg-panel{background:#ffffff}
      html.light .bg-panel\\/60{background:rgba(255,255,255,.7)}
      html.light .bg-panel2{background:#eef2f7}
      html.light .bg-panel2\\/40{background:rgba(226,232,240,.5)}
      html.light .bg-line{background:#e2e8f0}
      html.light .border-line{border-color:#e2e8f0}
      html.light .text-slate-100{color:#0f172a}
      html.light .text-slate-200{color:#1e293b}
      html.light .text-slate-300{color:#334155}
      html.light .text-slate-400{color:#475569}
      html.light .text-slate-500{color:#64748b}
      html.light .text-slate-600{color:#94a3b8}
      html.light .text-slate-700{color:#cbd5e1}
      html.light .text-teal{color:#0f766e}
      /* hover variants là class riêng (hover:bg-panel2…) nên phải override riêng — dùng xanh da trời nhạt */
      html.light .hover\\:bg-panel2:hover{background:#e0f2fe}
      html.light .hover\\:bg-panel:hover{background:#e0f2fe}
      html.light .hover\\:bg-panel2\\/40:hover{background:rgba(224,242,254,.7)}
      html.light .hover\\:text-slate-100:hover{color:#0f172a}
      html.light .hover\\:text-slate-300:hover{color:#334155}
    `;
    document.head.appendChild(st);
  }
  window.applyTheme = function (light) {
    document.documentElement.classList.toggle('light', light);
    const btn = document.getElementById('themeBtn');
    if (btn) btn.innerHTML = light ? SUN : MOON;
  };
  window.toggleTheme = function () {
    const light = !document.documentElement.classList.contains('light');
    try { localStorage.setItem('lm-theme', light ? 'light' : 'dark'); } catch (e) {}
    window.applyTheme(light);
  };
  window.applyTheme((function () { try { return localStorage.getItem('lm-theme') === 'light'; } catch (e) { return false; } })());

  // close switcher menu on outside click
  document.addEventListener('click', (e) => {
    const m = document.getElementById('nsSwitchMenu');
    if (m && !m.classList.contains('hidden') && !e.target.closest('#nsSwitcher')) m.classList.add('hidden');
  });
})();
