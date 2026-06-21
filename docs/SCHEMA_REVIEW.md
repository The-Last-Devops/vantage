# Schema naming review (pre-release standardization)

Goal: standardize table/field names in one pass before moving on. Resetting the DB is fine.

## Proposed conventions
- Tables: **plural, snake_case** (`api_keys`, `system_metrics`).
- Foreign keys: `<singular>_id` (`namespace_id`, `system_id`, `key_id`).
- Time-series: prefix by object (`system_metrics`, `container_metrics`).
- Drop leaky technical names ("token", "stats") → use domain terms ("key", "metrics").

## CONFIG DB

| Current | Proposed | Notes |
|---|---|---|
| `users` | `users` | keep |
| `sessions` | `sessions` | keep |
| `namespaces` | `namespaces` | keep (slug, name) |
| `memberships` | `memberships` | keep |
| `enrollment_tokens` | **`api_keys`** | column `token` → **`key`**; reuse as a general API key |
| `servers` | **`systems`** | UI already calls them "Systems"; covers node/docker/k8s |
| → `servers.token_id` | **`systems.key_id`** | FK to `api_keys` |
| → `servers.agent_token` | (drop) | replaced by key_id |
| `monitors` | `monitors` | keep (feature deferred) |
| `notification_channels` | **`channels`** | shorter |
| `alert_rules` | **`alerts`** | shorter |
| `status_pages` | `status_pages` | keep (deferred) |
| `alert_state` | `alert_state` | keep |

**`systems` (proposed columns):** `id, namespace_id, key_id, name, hostname, kind, cluster, enabled, last_seen, kernel, cpu_model, cpu_cores, agent_version, created_at`

## DATA DB

| Current | Proposed | Notes |
|---|---|---|
| `metrics` | **`system_metrics`** | clarifies it's a system metric; column `server_id` → **`system_id`** |
| `container_stats` | **`container_metrics`** | consistent "metrics" |
| `heartbeats` | `heartbeats` | keep (result of a monitor check) |
| `metrics_1m` / `metrics_1h` | **`system_metrics_1m` / `_1h`** | rollups follow the base table |

**`system_metrics` columns (kept, names already fine):** `time, system_id, cpu_percent, mem_used, mem_total, swap_used, swap_total, disk_used, disk_total, net_rx, net_tx, load1, uptime, disk_read, disk_write, temps, gpus`

## Ripple (follow-on changes)
- Wire type `MetricsReport` keeps its fields (already fine) + `kind`/`cluster`.
- Header `x-agent-token` → **`x-api-key`**; const `AGENT_TOKEN_HEADER` → `API_KEY_HEADER`.
- API routes: `/api/namespaces/:id/tokens` → `/keys`; `/api/tokens/:id` → `/api/keys/:id`.
- Code: ingest.rs, web.rs, api.rs, db.rs, agent, scripts (sim) — change `server_id→system_id`, `enrollment_tokens→api_keys`, `token→key`.
- Migrations: **consolidate** for cleanliness (merge server_meta/tokens/kind into init), reset the DB.
