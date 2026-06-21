# Kế hoạch triển khai Frontend thật (Vue SPA) — bản đầy đủ

Mục tiêu: port prototype `ui-prototype/simple` → app Vue thật, ăn JSON API của hub,
hiển thị **Metrics/Systems** với dữ liệu thật (đã có `scripts/sim-agents.sh` để test).
Monitors & Status page **tạm hoãn**.

> Các quyết định mở đã được chốt sẵn bằng **default khuyến nghị** (mục 2 & 3) để không
> kẹt khi bắt tay làm. Nếu muốn đổi, sửa ở đó trước khi chạy milestone 1.

---

## 1. Stack (đã chốt)
- **Vite + Vue 3 + vue-router + Pinia** — đã scaffold ở `frontend/`.
- **Tailwind qua PostCSS** (không CDN) — palette teal/navy đồng bộ prototype.
- **Charts: uPlot** (theo CLAUDE.md) — wrapper Vue mỏng.
- **Auth**: session cookie, `fetch(credentials:'include')`, same-origin.

## 2. [CHỐT] Hub phục vụ SPA = **embed `dist/` vào binary** (rust-embed)
Lý do: giữ triết lý **single-binary** của dự án.
- Thêm crate `rust-embed`; build script `frontend` chạy `vite build` → `frontend/dist`.
- Hub có handler fallback: route không khớp `/api/*` → trả file tĩnh trong `dist`,
  không thấy file → trả `index.html` (SPA history fallback).
- Dev: chạy `vite` (:5173) proxy `/api`→:8080; không cần rebuild hub khi sửa UI.
- Prod/CI: `vite build` trước khi `cargo build` hub (thêm bước vào Dockerfile.hub).

## 3. [CHỐT] Chuyển đổi = **strangler** (song song rồi cắt)
- Giai đoạn đầu: Vue phục vụ ở **`/app/*`**, SSR cũ (`ui.rs`) giữ nguyên ở `/`.
  → không phá cái đang chạy, làm tới đâu xem tới đó.
- Khi milestone 2–5 xong & ổn: **cắt `/` sang Vue**, gỡ dần route + code `ui.rs`.
- Lợi: an toàn, rollback dễ; hại: tồn tại 2 UI tạm thời (chấp nhận được).

---

## 4. API: đã có vs cần thêm
**Đã đủ cho phần lớn (đọc):**
- `POST /api/auth/login` · `POST /api/auth/logout` · `GET /api/me`
- `GET /api/servers` — đã có `kind` (`node`/`docker`/`k8s`) + `cluster` → group client-side
- `GET /api/servers/:id/metrics` (history) · `/containers` · `/temps` · `/gpu`
- `GET /api/namespaces` · CRUD namespaces/servers/tokens/members
- `DELETE /api/servers/:id` (bulk-delete) · `POST /api/namespaces/:id/tokens` (Add system)

**Cần thêm (đánh dấu phase):**
- [P-realtime] **SSE live 1s**: `GET /api/servers/:id/stream` (+ stream toàn fleet cho dashboard).
- [P-realtime] **Ghi rollup 1m** thay vì ghi raw mỗi report (theo memory `metrics-storage-tiers`).
- [M4] `server_metrics` nhận tham số **range + bucket** (1m/5m/15m/1h) để chart đổi theo range.
- [M4] (tuỳ) endpoint **container history** cho stacked-area docker (đã có `/containers` snapshot).

---

## 5. Milestones (thứ tự, mỗi cái chạy được mới sang tiếp)

### M1 — Nền tảng (foundation)
- `frontend/`: hoàn tất `pnpm install` (approve esbuild build), vite config proxy `/api`.
- Tailwind config + `src/style.css` tokens (ink/panel/panel2/line/teal) — port từ prototype.
- `src/lib/api.js` (fetch wrapper, credentials), `src/stores/auth.js` (me/login/logout).
- `src/router/index.js` + guard (chưa login → /app/login).
- Hub: thêm rust-embed + fallback route phục vụ `dist` ở `/app` (mục 2).
- **Done khi**: vào `/app` thấy shell rỗng + redirect login; `vite build` embed chạy qua hub.

### M2 — Login
- `pages/Login.vue` (port giao diện prototype), gọi `auth.login`.
- Guard: đã login → vào Systems; sai pass → báo lỗi.
- **Done khi**: login bằng `admin@local/admin123` vào được Systems.

### M3 — Systems dashboard
- `components/AppShell.vue` (sidebar + topbar, theme toggle, namespace switcher multi-select).
- `pages/Systems.vue`: hero (online/avg CPU/mem từ `/api/servers`), 3 section
  **Nodes / Docker / Kubernetes** (group theo `kind`; k8s gom theo `cluster`).
- Bảng: sort cột, filter, checkbox + **bulk delete** (`DELETE /api/servers/:id`).
- K8s cluster = dòng tổng hợp (aggregate client-side các node cùng `cluster`), expand ra node.
- Docker = dòng host, expand ra containers (`/api/servers/:id/containers`).
- **Done khi**: chạy `sim-agents.sh` → thấy đúng 3 section, số liệu khớp, sort/filter/select/xoá chạy.

### M4 — System detail (theo `kind`)
- `pages/SystemDetail.vue` route `/app/system/:id` (+ query type cho leaf).
- **node/host**: charts uPlot từ `/api/servers/:id/metrics` (CPU/Mem/Disk/IO/Net + Temp/GPU nếu có), range selector → resolution.
- **docker**: host snapshot (chỉ số hiện tại) + nút "Host metrics" + bảng **Containers** (click → container detail).
- **k8s cluster**: overview gọn + bảng **Nodes** (click node → host charts).
- Breadcrumb phân cấp; empty-state cho Temp/GPU khi không có sensor.
- **Done khi**: click từ Systems vào đúng layout từng loại, chart vẽ từ data thật.

### M5 — Theme (đàng hoàng)
- Token màu qua **CSS variables** (`:root` dark, `.light` override) — không hack như prototype.
- Toggle ở topbar, lưu `localStorage`, áp toàn app (kể cả login).
- Hover/select hàng rõ ràng cả 2 mode (đã định nghĩa ở prototype, chuyển sang biến).
- **Done khi**: toggle mượt, mọi trang đổi theme nhất quán.

### M6 — Add system
- Modal Node/Docker/K8s + tabs lệnh cài (Binary/Docker/Compose; Helm/Manifest).
- Tạo token ngầm qua `POST /api/namespaces/:id/tokens`, nhúng vào lệnh, không lộ token UI.
- Namespace dropdown tùy biến.
- **Done khi**: tạo system mới → token sinh ngầm → lệnh cài đúng theo loại.

### M7 — Realtime (phase backend + frontend)
- Backend: ghi **rollup 1m** (gom report trong RAM → ghi 1 dòng/phút); `IngestAck.next_interval_secs` để agent đẩy ~1s khi có viewer.
- Backend: **SSE** endpoint fan-out report live cho viewer đang mở.
- Frontend: overlay điểm "Live 1s" lên chart + cập nhật gauge/sparkline realtime.
- Retention/nén đã cấu hình sẵn (memory `metrics-storage-tiers`).
- **Done khi**: mở detail → thấy điểm live nhảy 1s; đóng tab → agent giảm tần suất.

### M8 — Cutover + dọn
- Chuyển `/` sang Vue, gỡ route SSR trong `main.rs` + code `ui.rs` không dùng.
- Cập nhật CLAUDE.md (frontend = Vue SPA embed, không còn HTMX/SSR cho các trang đã port).
- **Done khi**: 1 UI duy nhất, binary vẫn single-file.

---

## 6. Test mỗi bước
- `bash scripts/sim-agents.sh` (fleet node/docker/k8s, data thật) → render & thao tác.
- `bash scripts/sim-reset.sh` khi cần dọn data test.
- `bash scripts/check-systems.sh` verify API.
- Đổi quy mô: `NODES=80 DOCKER=15 K8S_CLUSTERS=4 bash scripts/sim-agents.sh`.

## 7. Rủi ro / lưu ý
- **Cookie same-origin**: dev qua Vite proxy; prod cùng hub → OK. Nếu sau này tách domain (Cloudflare Pages) → đổi cookie `SameSite=None;Secure` + CORS, hoặc Pages proxy `/api`.
- **uPlot** thiếu types đẹp → wrapper mỏng, load như script.
- **Khối lượng lớn** → tuyệt đối làm tuần tự theo milestone; không nhảy cóc.
- **Strangler**: giữ SSR tới M8 mới gỡ, tránh mất tính năng giữa chừng.

## 8. Thứ tự thực thi gợi ý (1 mạch)
M1 → M2 → M3 → M4 → M5 → M6 → (M7 realtime, lớn, có thể tách đợt) → M8 cutover.
MVP xem được = hết **M4** (Login + Systems + Detail với data thật).
