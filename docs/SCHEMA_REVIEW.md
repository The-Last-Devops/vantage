# Schema naming review (pre-release standardization)

Mục tiêu: chuẩn hoá tên bảng/trường một lượt trước khi đi tiếp. Reset DB thoải mái.

## Quy ước đề xuất
- Bảng: **số nhiều, snake_case** (`api_keys`, `system_metrics`).
- Khoá ngoài: `<singular>_id` (`namespace_id`, `system_id`, `key_id`).
- Time-series: tiền tố theo đối tượng (`system_metrics`, `container_metrics`).
- Bỏ tên kỹ thuật rò rỉ ("token", "stats") → dùng domain ("key", "metrics").

## CONFIG DB

| Hiện tại | Đề xuất | Ghi chú |
|---|---|---|
| `users` | `users` | giữ |
| `sessions` | `sessions` | giữ |
| `namespaces` | `namespaces` | giữ (slug, name) |
| `memberships` | `memberships` | giữ |
| `enrollment_tokens` | **`api_keys`** | cột `token` → **`key`**; tái dùng làm API key chung |
| `servers` | **`systems`** | UI đã gọi "Systems"; gồm node/docker/k8s |
| → `servers.token_id` | **`systems.key_id`** | FK tới `api_keys` |
| → `servers.agent_token` | (bỏ) | đã thay bằng key_id |
| `monitors` | `monitors` | giữ (tạm hoãn feature) |
| `notification_channels` | **`channels`** | gọn |
| `alert_rules` | **`alerts`** | gọn |
| `status_pages` | `status_pages` | giữ (tạm hoãn) |
| `alert_state` | `alert_state` | giữ |

**`systems` (đề xuất cột):** `id, namespace_id, key_id, name, hostname, kind, cluster, enabled, last_seen, kernel, cpu_model, cpu_cores, agent_version, created_at`

## DATA DB

| Hiện tại | Đề xuất | Ghi chú |
|---|---|---|
| `metrics` | **`system_metrics`** | rõ là metric của system; cột `server_id` → **`system_id`** |
| `container_stats` | **`container_metrics`** | nhất quán "metrics" |
| `heartbeats` | `heartbeats` | giữ (kết quả monitor check) |
| `metrics_1m` / `metrics_1h` | **`system_metrics_1m` / `_1h`** | rollup theo bảng gốc |

**Cột `system_metrics` (giữ, tên đã ổn):** `time, system_id, cpu_percent, mem_used, mem_total, swap_used, swap_total, disk_used, disk_total, net_rx, net_tx, load1, uptime, disk_read, disk_write, temps, gpus`

## Ripple (đổi theo)
- Wire type `MetricsReport` giữ nguyên field (đã ổn) + `kind`/`cluster`.
- Header `x-agent-token` → **`x-api-key`**; const `AGENT_TOKEN_HEADER` → `API_KEY_HEADER`.
- API routes: `/api/namespaces/:id/tokens` → `/keys`; `/api/tokens/:id` → `/api/keys/:id`.
- Code: ingest.rs, web.rs, api.rs, db.rs, agent, scripts (sim) — đổi `server_id→system_id`, `enrollment_tokens→api_keys`, `token→key`.
- Migrations: **consolidate** lại cho sạch (gộp server_meta/tokens/kind vào init), reset DB.
