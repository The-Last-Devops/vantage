# Last Monitor — API

The hub exposes a JSON API (the web UI consumes it) plus an embedded **MCP server**.
This is a hand-maintained reference; treat the running hub as the source of truth.

## Base & format

- Base URL: your hub origin, e.g. `https://monitor.example.com`.
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
create a dedicated *service-account* user with membership in only the namespaces it needs,
then issue the PAT as that user.

```bash
curl -H "Authorization: Bearer $LM_TOKEN" https://monitor.example.com/api/systems
```

## RBAC

Permissions are namespace-scoped: `viewer` (read) < `editor` (add/edit systems, services,
alerts, channels) < `owner` (also manage members). A system **admin** is owner everywhere;
a **read-only admin** can view every namespace. Reads return only the namespaces you can see;
writes require `editor`+ in the target's namespace.

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
| GET | `/api/systems` | hosts + latest sample (in your namespaces) |
| GET | `/api/systems/{id}/metrics` | time series; `range` + `bucket` params |
| GET | `/api/systems/{id}/containers` · `/temps` · `/gpu` | per-host detail |
| GET | `/api/fleet` | fleet-wide overview |
| PATCH | `/api/systems/{id}` | rename (editor+) |
| DELETE | `/api/systems/{id}` | remove (editor+) |
| GET/POST | `/api/thresholds` | per-namespace alert thresholds |

### Services (monitors)
| Method | Path | Notes |
|---|---|---|
| GET | `/api/monitors` | all service checks you can see, with up/down |
| POST | `/api/namespaces/{id}/monitors` | create (editor+); probed immediately |
| PATCH/DELETE | `/api/monitors/{id}` | edit / delete (editor+) |
| GET | `/api/monitors/{id}/debug` · `/events` | last request/response · status history |

### Alerts
| Method | Path | Notes |
|---|---|---|
| GET | `/api/namespaces/{id}/alerts` | rules whose target is in this namespace |
| POST | `/api/namespaces/{id}/alerts` | create rule (editor+); `channel_ids` may be any channel |
| PATCH/DELETE | `/api/alerts/{id}` | edit / delete (editor+) |
| POST | `/api/alerts/{id}/test` | send a test through the rule's channels |
| GET | `/api/namespaces/{id}/alert-events` | fire/recover history |
| GET | `/api/events` | recent service status changes |

### Notify channels (shared resource)
| Method | Path | Notes |
|---|---|---|
| GET | `/api/channels` | every channel + `namespace`, `can_edit` (secrets masked unless you can edit) |
| GET | `/api/channel-types` | provider manifest (form schema) |
| POST | `/api/namespaces/{id}/channels` | create in a namespace (editor+) |
| PATCH/DELETE | `/api/channels/{id}` | edit / delete (editor of the channel's namespace) |
| POST | `/api/channels/{id}/test` | send a test notification |
| GET | `/api/channels/{id}/alerts` | rules that use this channel |

### Namespaces, members, users (admin)
| Method | Path | Notes |
|---|---|---|
| GET/POST | `/api/namespaces` · DELETE `/api/namespaces/{id}` | manage namespaces |
| GET/POST/DELETE | `/api/namespaces/{id}/members` | membership + role |
| GET/POST | `/api/users` · PATCH/DELETE `/api/users/{id}` | accounts (admin) |
| GET | `/api/users/{id}/memberships` | a user's per-namespace roles |
| GET | `/api/audit` | action log (admin) |

### Admin: backup & retention
| Method | Path | Notes |
|---|---|---|
| GET | `/api/admin/data` · POST `/api/admin/retention` | retention tiers |
| GET/POST | `/api/admin/backup*` | download/restore + S3 (secrets redacted on read) |

### Public (no session)
| Method | Path | Notes |
|---|---|---|
| POST | `/pub/ingest` | agent metrics push (`x-agent-token`) |
| GET | `/pub/push/{token}` | passive push-monitor heartbeat |
| GET | `/pub/install.sh` · `/pub/agent.yaml` | agent install assets |
| GET | `/healthz` | liveness |

## MCP server

`POST /mcp` speaks **JSON-RPC 2.0** (MCP). Authenticate with a PAT
(`Authorization: Bearer <pat>`); tools run with that user's RBAC.

Methods: `initialize`, `tools/list`, `tools/call`, `ping`.

| Tool | Access | Args |
|---|---|---|
| `list_systems` | read | — |
| `list_services` | read | — |
| `alerts_firing` | read | — |
| `recent_events` | read | `limit?` |
| `run_service_check` | editor of target ns | `monitor_id` |
| `toggle_alert_rule` | editor of target ns | `alert_id`, `enabled` |

```bash
# list available tools
curl -s -X POST https://monitor.example.com/mcp \
  -H "Authorization: Bearer $LM_TOKEN" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'

# call a tool
curl -s -X POST https://monitor.example.com/mcp \
  -H "Authorization: Bearer $LM_TOKEN" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"alerts_firing","arguments":{}}}'
```

To connect an MCP client (e.g. Claude), point it at `<hub>/mcp` as a *streamable HTTP* MCP
server and set the `Authorization: Bearer <pat>` header.
