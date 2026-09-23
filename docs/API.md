# Vantage — API

The hub exposes a JSON API (the web UI consumes it) plus an embedded **MCP server**.
This is a hand-maintained reference; treat the running hub as the source of truth.

## Base & format

- Base URL: your hub origin, e.g. `https://vantage.example.com`.
- Requests/responses are JSON (`Content-Type: application/json`).
- Errors use HTTP status codes: `400` invalid input · `401` unauthenticated ·
  `403` insufficient role · `404` not found · `409` conflict · `5xx` server error.
- IDs are UUIDs. Timestamps are RFC 3339 / ISO 8601 (UTC).

## Authentication

Three independent paths — never mix them:

| Caller | How |
|---|---|
| **Programmatic** (scripts, third parties, MCP) | `Authorization: Bearer <pat>` |
| Browser (the SPA) | `session` httpOnly cookie from `POST /api/auth/login` |
| Agents (metrics push only) | `x-agent-token: <enrollment-key>` on `/pub/ingest` |

**Personal access tokens (PAT)** are the way to call the API from code. Create one in the
UI under **Settings › API tokens** (or `POST /api/pats`); the secret is shown once. A PAT
**acts as the user who created it** and inherits that user's RBAC. To limit a token's reach,
create a dedicated *service-account* user with membership in only the workspaces it needs,
then issue the PAT as that user.

```bash
curl -H "Authorization: Bearer $LM_TOKEN" https://vantage.example.com/api/systems
```

## RBAC

Permissions are workspace-scoped: `viewer` (read) < `editor` (add/edit systems, services,
alerts, channels) < `owner` (also manage members). A system **admin** is owner everywhere;
a **read-only admin** can view every workspace. Reads return only the workspaces you can see;
writes require `editor`+ in the target's workspace.

## Endpoints

### Auth & account
| Method | Path | Notes |
|---|---|---|
| POST | `/api/auth/login` | `{email,password}` → sets session cookie |
| POST | `/api/auth/logout` | clears the session |
| GET | `/api/me` | the current user |

### API tokens (PAT)
| Method | Path | Notes |
|---|---|---|
| GET | `/api/pats` | your tokens (metadata only) |
| POST | `/api/pats` | `{name, expires_in_days?}` → returns the token **once** |
| DELETE | `/api/pats/{id}` | revoke |

### Infrastructure (hosts)
| Method | Path | Notes |
|---|---|---|
| GET | `/api/systems` | hosts + latest sample (in your workspaces) |
| GET | `/api/systems/{id}/metrics` | time series; `range` + `bucket` params |
| GET | `/api/systems/{id}/containers` · `/temps` · `/gpu` | per-host detail |
| GET | `/api/fleet` | fleet-wide overview |
| PATCH | `/api/systems/{id}` | rename (editor+) |
| DELETE | `/api/systems/{id}` | remove (editor+) |
| GET | `/api/thresholds` | alert thresholds for every workspace you can see |
| PUT | `/api/workspaces/{id}/thresholds` | set that workspace's thresholds (editor+) |
| GET/PUT | `/api/systems/{id}/shell` | per-host shell/exec config (see docs/exec-design.md) |
| POST | `/api/systems/{id}/console/ticket` | mint a short-lived step-up ticket for the console |
| GET | `/api/systems/{id}/console` | console **WebSocket** (not reachable over MCP) |
| GET | `/api/systems/{id}/alerts` | rules targeting this host |
| GET/POST | `/api/ssh-keys` · DELETE `/api/ssh-keys/{id}` | your account's SSH key library |
| GET | `/api/kube/summaries` | Kubernetes clusters reporting in |
| GET | `/api/systems/{id}/kube/summary` · `/aggregate` · `/containers` · `/series` · `/series-by` | per-cluster views |

### Services (monitors)
| Method | Path | Notes |
|---|---|---|
| GET | `/api/monitors` | all service checks you can see, with up/down |
| POST | `/api/workspaces/{id}/monitors` | create (editor+); probed immediately |
| PATCH/DELETE | `/api/monitors/{id}` | edit / delete (editor+) |
| GET | `/api/monitors/{id}` | one check in full (config, state, uptime) |
| GET | `/api/monitors/{id}/debug` · `/events` · `/heartbeats` | last request/response · status history · raw probe results |
| GET | `/api/monitors/{id}/alerts` | rules targeting this check |

### Alerts
| Method | Path | Notes |
|---|---|---|
| GET | `/api/workspaces/{id}/alerts` | rules whose target is in this workspace |
| POST | `/api/workspaces/{id}/alerts` | create rule (editor+); `channel_ids` may be any channel |
| GET/PATCH/DELETE | `/api/alerts/{id}` | read / edit / delete (editor+) |
| POST | `/api/alerts/{id}/test` | send a test through the rule's channels |
| GET | `/api/workspaces/{id}/alert-events` | fire/recover history |
| GET | `/api/events` | recent service status changes |

### Notify channels (shared resource)
| Method | Path | Notes |
|---|---|---|
| GET | `/api/channels` | every channel + `workspace`, `can_edit` (secrets masked unless you can edit) |
| GET | `/api/channel-types` | provider manifest (form schema) |
| POST | `/api/workspaces/{id}/channels` | create in a workspace (editor+) |
| POST | `/api/workspaces/{id}/channels/test` | test an unsaved `{kind, config}` before creating/saving (editor+) |
| PATCH/DELETE | `/api/channels/{id}` | edit / delete (editor of the channel's workspace) |
| POST | `/api/channels/{id}/test` | send a test notification |
| GET | `/api/channels/{id}/alerts` | rules that use this channel (`target`, `kind`, `firing`) |

### Workspaces, members, users (admin)
| Method | Path | Notes |
|---|---|---|
| GET/POST | `/api/workspaces` · DELETE `/api/workspaces/{id}` | manage workspaces |
| GET/POST/DELETE | `/api/workspaces/{id}/members` | membership + role |
| GET/POST | `/api/users` · PATCH/DELETE `/api/users/{id}` | accounts (admin) |
| GET | `/api/users/{id}/memberships` | a user's per-workspace roles |
| DELETE | `/api/workspaces/{id}/members/{user_id}` | remove a member (owner+) |
| PUT | `/api/workspaces/{id}/members/{user_id}/exec` | grant/revoke shell access for a member |
| GET | `/api/workspaces/{id}/member-candidates` | users who could be added |
| GET/POST | `/api/workspaces/{id}/keys` · DELETE `/api/keys/{id}` | agent enrollment keys |
| GET | `/api/keys/{id}/systems` | hosts enrolled with a key |
| POST | `/api/workspaces/{id}/status-pages` · DELETE `/api/status-pages/{id}` | public status pages |
| GET/POST | `/api/me/2fa*` · `/api/me/passkeys*` · POST `/api/me/password` | your own account security |
| GET | `/api/about` | version + release notes |
| GET | `/api/audit` | action log (admin); filters `?q=&method=&status=ok\|client\|server&limit=&offset=`, returns `{rows, total, retention_days}` |
| PUT | `/api/admin/audit/retention` | `{days}` — keep window for the audit log; `null`/0 = forever (admin) |

### Admin: backup & retention
| Method | Path | Notes |
|---|---|---|
| GET | `/api/admin/data` · POST `/api/admin/retention` | retention tiers |
| POST | `/api/admin/data-cap` · `/api/admin/data-cap/enforce` | data-DB size cap + manual eviction |
| POST | `/api/admin/config-retention` | config-DB log retention |
| GET/POST | `/api/admin/ingest-intervals` | agent push cadence |
| GET | `/api/admin/logs` | the hub's in-memory log ring |
| POST | `/api/admin/exposure-check` | public-exposure self-check |
| GET/POST | `/api/admin/backup*` | download/restore + S3 (secrets redacted on read) |

The full, always-current list is the `list_endpoints` MCP tool — it is asserted
against the router in `crates/hub/src/mcp.rs`, so it cannot drift the way this
hand-written table can.

### Public (no session)
| Method | Path | Notes |
|---|---|---|
| POST | `/pub/ingest` | agent metrics push (`x-agent-token`) |
| GET | `/pub/push/{token}` | passive push-monitor heartbeat |
| GET | `/pub/install.sh` · `/pub/agent.yaml` | agent install assets |
| GET | `/healthz` | liveness |

## MCP server

`POST /mcp` speaks **JSON-RPC 2.0** (MCP). Authenticate with a PAT
(`Authorization: Bearer <pat>`); every tool runs with that user's RBAC — reads
scoped to their workspaces, writes requiring `editor` there. There is no
separate permission layer for MCP: to limit what an assistant can do, issue its
PAT to a service-account user with membership only where it belongs.

Methods: `initialize`, `tools/list`, `tools/call`, `ping`.

### The whole API, in one tool

`api_request` dispatches any `{method, path, body}` into the hub's **own router,
in-process** — the same handlers, the same `require_role`, the same audit trail,
with no network hop. So every endpoint above is reachable from an MCP client the
day it is added, and `list_endpoints` returns the route map to go with it.

Only `/api/**` and `/pub/**` are addressable (anything else would hit the SPA
fallback and answer `index.html` with a 200), and WebSocket routes — the host
console, the agent tunnel — cannot be driven this way.

| Tool | Access | Args |
|---|---|---|
| `api_request` | as the PAT's user | `method`, `path`, `body?` |
| `list_endpoints` | read | — |

### What the objects are

The hub hands an assistant a short orientation in the `initialize` response
(MCP's `instructions`), because a flat list of 30 tools does not say what the
things are or what order they go in:

    workspace  — the tenant boundary; everything belongs to one
    system     — a monitored host, pushing metrics from its agent
    service    — an outbound check the hub runs (called a `monitor` in URLs)
    channel    — where a notification goes; shared across workspaces
    alert rule — a condition on a system or service + the channels to notify;
                 its channels must exist first

Two shapes of rule. On a **service**, omit `condition` — it fires when the check
goes down. On a **host**, pass a threshold:

```json
{"metric": "cpu_percent", "op": ">", "value": 90}
```

`metric` is `cpu_percent` | `mem_percent` | `load1`; `op` is `>` `>=` `<` `<=`.

Service `kind` is one of `http` `tcp` `ping` `keyword` `postgres` `redis` `dns`
`rabbitmq` `mysql` `mongodb` `tls` `push`. All but `push` need a `target`
(`push` is passive — the hub generates a URL and waits to be pinged).

Channel `kind` is one of 17 providers, each with **different** `config` fields —
call `channel_types` for them rather than guessing, since a wrong config yields a
channel that silently never delivers. `test_channel` proves one works.

### Curated tools

Named front doors onto single endpoints, with a real input schema so an
assistant picks them without reading this page. Each is a thin wrapper over the
same dispatch, so a tool cannot drift from its endpoint.

| Tool | Access | Args |
|---|---|---|
| `list_systems` · `list_services` · `alerts_firing` | read | — |
| `list_workspaces` · `list_channels` · `fleet` · `kube_summaries` | read | — |
| `channel_types` | read | — |
| `recent_events` · `audit_log` | read | `limit?` |
| `system_metrics` | read | `system_id`, `range?` |
| `system_containers` | read | `system_id` |
| `get_service` | read | `monitor_id` |
| `service_heartbeats` | read | `monitor_id`, `limit?` |
| `list_alert_rules` | read | `workspace_id` |
| `get_alert_rule` | read | `alert_id` |
| `run_service_check` | editor of target ws | `monitor_id` |
| `create_service` | editor | `workspace_id`, `name`, `kind`, `target`, `interval_secs?`, `config?` |
| `update_service` | editor | `monitor_id` + any of `name`, `target`, `interval_secs`, `enabled`, `config` |
| `delete_service` | editor | `monitor_id` |
| `create_alert_rule` | editor | `workspace_id`, `channel_ids` + a target (`monitor_id` / `system_id` / `scope_kind`) |
| `update_alert_rule` | editor | `alert_id` + any rule field |
| `toggle_alert_rule` | editor | `alert_id`, `enabled` |
| `delete_alert_rule` · `test_alert_rule` | editor | `alert_id` |
| `create_channel` | editor | `workspace_id`, `name`, `kind`, `config?` |
| `test_channel` · `delete_channel` | editor | `channel_id` |

```bash
# list available tools
curl -s -X POST https://vantage.example.com/mcp \
  -H "Authorization: Bearer $LM_TOKEN" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'

# call a curated tool
curl -s -X POST https://vantage.example.com/mcp \
  -H "Authorization: Bearer $LM_TOKEN" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"alerts_firing","arguments":{}}}'

# reach an endpoint that has no curated tool
curl -s -X POST https://vantage.example.com/mcp \
  -H "Authorization: Bearer $LM_TOKEN" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"api_request",
       "arguments":{"method":"GET","path":"/api/systems/<uuid>/kube/summary"}}}'
```

To connect an MCP client (e.g. Claude), point it at `<hub>/mcp` as a *streamable HTTP* MCP
server and set the `Authorization: Bearer <pat>` header.

`bash scripts/check-mcp.sh` smoke-tests all of this against a running hub.
