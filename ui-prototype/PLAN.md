# Kế hoạch UI — last-monitor

Tài liệu chốt **pages / tính năng / hướng giao diện** trước khi dựng prototype.
Bám theo app hiện tại (routes trong `crates/hub`) + parity với Beszel & Uptime-Kuma.

---

## 1. Hướng giao diện (design direction)

**Identity:** nền navy-đen sâu, accent teal/aqua — giữ đúng palette đã có.

| Token | Value | Dùng cho |
|-------|-------|----------|
| `ink` | `#0B0E14` | nền trang |
| `panel` | `#10151F` | header, card nền |
| `panel2` | `#141A26` | row, input |
| `line` | `#1E2632` | border |
| `teal` | `#34E1C4` | accent, link, status up |
| đỏ/amber | tailwind `red-400`/`amber-400` | down / cảnh báo |

- **Font:** mono cho số liệu/nhãn kỹ thuật, sans cho nội dung dài.
- **Layout:** **sidebar trái cố định toàn app** (logo · nav chính · nhóm Manage · user) + topbar mỏng (namespace switcher · search Ctrl-K · theme · user menu). Nội dung bên phải.
- **Mật độ:** dày thông tin như Beszel (bảng compact, bar gauge, sparkline), không thưa.
- **Theme:** dark trước, light sau (token-driven để đổi dễ).
- **Charts:** uPlot (đúng kiến trúc gốc), area/line, legend hover.
- **Tương tác:** realtime auto-refresh, filter/sort tại client.

---

## 2. Pages (màn hình)

### Nhóm A — App chính (sau đăng nhập)

| Page | Route | Mục đích | Thành phần chính | Ưu tiên |
|------|-------|----------|------------------|---------|
| **Login** | `/login` | Đăng nhập | form email/pass, logo, lỗi | P0 |
| **Systems** (dashboard) | `/` | Tổng quan toàn fleet | hero fleet-health, bảng system (status · CPU/Mem/Disk bar · Net · agent ver · sparkline · 🔔 · ⋯), filter/sort/columns, **Add system** (Node / Docker host / K8s cluster) | P0 |
| **System detail** | `/system/:id` | Chi tiết 1 system (node/docker/cluster) | header metadata (uptime, kernel, CPU model/cores, host), range selector, charts: CPU/Mem/Disk/Disk-I/O/Net/Temp/GPU, **container stats** stacked-area | P0 |
| ~~**Monitors**~~ | `/monitors` | Service checks | **TẠM HOÃN — làm sau, ưu tiên metrics** | — |
| ~~**Monitor detail**~~ | `/monitors/:id` | — | **TẠM HOÃN** | — |

### Nhóm B — Quản trị (Manage)

| Page | Route | Mục đích | Ưu tiên |
|------|-------|----------|---------|
| **Namespaces** | `/manage/namespaces` | Tạo/xem namespace | P1 |
| **Systems cfg** | `/manage/systems` | Enroll token, sửa/xóa system (node/docker/cluster) | P1 |
| ~~**Monitors cfg**~~ | `/manage/monitors` | **TẠM HOÃN** | — |
| **Notifications** | `/manage/notifications` | Channels (Telegram/Discord/webhook…) | P2 |
| **Alert rules** | (trong namespace) | Rule + channel | P2 |
| **Members** | `/manage/members` | RBAC: owner/editor/viewer | P2 |
| ~~**Status pages**~~ | `/manage/status` | **TẠM HOÃN** | — |
| **Users** | `/manage/users` | Admin tạo user | P2 |
| **Data** | `/manage/data` | Thống kê + retention TimescaleDB | P3 |

### Nhóm C — Public (không cần đăng nhập)

| Page | Route | Mục đích | Ưu tiên |
|------|-------|----------|---------|
| ~~**Status page**~~ | `/status/:slug` | **TẠM HOÃN — làm sau** | — |

---

## 3. Checklist tính năng (parity Beszel)

**Đã có (port sang giao diện mới):** status dot, CPU/Mem/Disk gauge, net rate, range selector, charts CPU/Mem/Disk/Net, namespace switcher, manage CRUD.

**Cần thêm (theo Beszel):**
- [ ] Container/Docker stats (stacked-area per container) — *điểm nhấn lớn*
- [ ] Disk I/O (read/write)
- [ ] Temperature (multi-sensor)
- [ ] GPU (power, usage, VRAM)
- [ ] Cột Agent version
- [ ] Sort / filter / show-hide columns trên bảng
- [ ] Light theme
- [ ] i18n (đa ngôn ngữ)
- [ ] Command palette (Ctrl-K)
- [ ] Header metadata host (uptime/kernel/CPU model)
- [ ] Realtime updates (SSE)

---

## 4. Thứ tự dựng prototype (tĩnh trong `ui-prototype/simple/`)

1. [x] **Shell + Login** — sidebar trái + topbar + login (dark teal).
2. [x] **Systems dashboard** — hero (động) + 3 section (Servers/Docker/K8s), cột Namespace, sort theo cột, checkbox chọn + thanh bulk-action (Delete), filter toàn cục, nút Add theo loại.
3. [x] **Server detail** — metadata header + range + chart grid + container stacked-area.
4. [~] **Monitors** — đã dựng nhưng **TẠM GỠ khỏi nav** (làm sau).
5. [~] **Public status page** — đã dựng nhưng **TẠM GỠ khỏi nav** (làm sau).
   > Quyết định: tập trung **Metrics/Systems** cho xịn trước; Monitors & Status page làm sau.
6. [ ] **Manage** (sidebar + vài form mẫu) — chưa làm.

> Responsive: sidebar off-canvas + hamburger trên mobile, hero/summary grid co theo breakpoint, bảng `overflow-x-auto`.
> Shell dùng chung: `simple/js/shell.js` (sau này thành layout component Vue).

> Mỗi bước: dựng tĩnh → chốt look → sau này port sang Vue/SSR.
