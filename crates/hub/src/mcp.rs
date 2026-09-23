//! Embedded MCP (Model Context Protocol) server over HTTP, so AI assistants can
//! read and operate the monitor. One endpoint, `POST /mcp`, speaks JSON-RPC 2.0
//! (initialize / tools/list / tools/call). Auth is a PAT via `Authorization:
//! Bearer …` (the `CurrentUser` extractor), so every tool runs with that user's
//! RBAC — reads are scoped to their workspaces, writes require editor there.
//!
//! **Two layers, on purpose.** Curated tools (`list_services`, `create_service`,
//! …) carry a real input schema, so an assistant picks them correctly without
//! reading docs. Underneath, `api_request` dispatches ANY method+path into the
//! hub's own router in-process — same handlers, same `require_role`, same audit
//! middleware, no network hop. That is what makes the MCP surface complete:
//! every endpoint added to `main.rs` is reachable through it the day it lands,
//! and `list_endpoints` (asserted against `main.rs` by a test below) tells the
//! assistant what exists. Curated tools are themselves thin wrappers over that
//! dispatch — one code path, so a tool can never drift from its endpoint.
//!
//! There is no deny-list here: a PAT acts as its user, so what MCP may touch is
//! decided by that user's RBAC. Scope an assistant by issuing its PAT to a
//! service-account user with membership only where it belongs.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::rbac::{self, Role};
use crate::AppState;

/// Cap on a dispatched response we read back into a tool result. Big enough for
/// a fleet listing, small enough that one bad call can't blow up the assistant's
/// context (a metrics series over a long range is the realistic offender).
const MAX_BODY: usize = 512 * 1024;

const PROTOCOL: &str = "2024-11-05";

/// POST /mcp — a single JSON-RPC request (or notification). Authenticated by PAT.
pub async fn handle(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> Response {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");

    // Notifications (no id, e.g. notifications/initialized) get no JSON-RPC reply.
    if id.is_none() || id == Some(Value::Null) {
        return StatusCode::ACCEPTED.into_response();
    }

    let result: Result<Value, (i64, String)> = match method {
        "initialize" => {
            let pv = req
                .get("params")
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL)
                .to_string();
            Ok(json!({
                "protocolVersion": pv,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "vantage", "version": env!("CARGO_PKG_VERSION") },
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_defs() })),
        "tools/call" => call_tool(&state, &user, &headers, req.get("params")).await,
        other => Err((-32601, format!("method not found: {other}"))),
    };

    let body = match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    };
    Json(body).into_response()
}

fn tool_defs() -> Value {
    let empty = json!({ "type": "object", "properties": {} });
    let mut defs = vec![
        // ---- the escape hatch: the whole API, always in sync -----------------
        json!({ "name": "api_request", "description":
            "Call ANY hub API endpoint directly (same handlers, RBAC and audit trail as the web UI). \
             Use `list_endpoints` first to see what exists, or the curated tools when one fits. \
             Returns {status, body}. WebSocket routes (console, tunnel) cannot be reached this way.",
            "inputSchema": { "type": "object", "required": ["method", "path"], "properties": {
                "method": { "type": "string", "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"] },
                "path": { "type": "string", "description": "e.g. /api/systems or /api/monitors/<uuid>?range=24h" },
                "body": { "type": "object", "description": "JSON request body (POST/PUT/PATCH)" } } } }),
        json!({ "name": "list_endpoints", "description":
            "Every HTTP endpoint this hub serves, with the methods it accepts — the map for `api_request`.",
            "inputSchema": empty }),

        // ---- reads ------------------------------------------------------------
        json!({ "name": "list_systems", "description": "List monitored hosts with their workspace and online state.", "inputSchema": empty }),
        json!({ "name": "list_services", "description": "List service checks (monitors) with up/down state and workspace.", "inputSchema": empty }),
        json!({ "name": "alerts_firing", "description": "Alert rules that are currently firing.", "inputSchema": empty }),
        json!({ "name": "recent_events", "description": "Recent alert fire/recover events.", "inputSchema": {
            "type": "object", "properties": { "limit": { "type": "integer", "description": "max events (default 20)" } } } }),
        json!({ "name": "list_workspaces", "description": "Workspaces the caller can see, with their role in each.", "inputSchema": empty }),
        json!({ "name": "fleet", "description": "Fleet overview: every host with its latest sample (CPU, memory, disk, load).", "inputSchema": empty }),
        json!({ "name": "system_metrics", "description": "Time series of a host's metrics. Use this to diagnose a host, not list_systems.", "inputSchema": {
            "type": "object", "required": ["system_id"], "properties": {
                "system_id": { "type": "string", "description": "host UUID" },
                "range": { "type": "string", "description": "window, e.g. 1h / 24h / 7d (default 1h)" } } } }),
        json!({ "name": "system_containers", "description": "Containers reported by a host, with their CPU/memory.", "inputSchema": {
            "type": "object", "required": ["system_id"], "properties": { "system_id": { "type": "string" } } } }),
        json!({ "name": "kube_summaries", "description": "Kubernetes clusters reporting in, with node and workload counts.", "inputSchema": empty }),
        json!({ "name": "get_service", "description": "One service check in full: config, interval, current state.", "inputSchema": {
            "type": "object", "required": ["monitor_id"], "properties": { "monitor_id": { "type": "string" } } } }),
        json!({ "name": "service_heartbeats", "description": "A service check's recent probe results — the history behind its up/down state.", "inputSchema": {
            "type": "object", "required": ["monitor_id"], "properties": {
                "monitor_id": { "type": "string" },
                "limit": { "type": "integer", "description": "max beats (default 50)" } } } }),
        json!({ "name": "list_alert_rules", "description": "Alert rules in a workspace, with their condition and channels.", "inputSchema": {
            "type": "object", "required": ["workspace_id"], "properties": { "workspace_id": { "type": "string" } } } }),
        json!({ "name": "get_alert_rule", "description": "One alert rule in full.", "inputSchema": {
            "type": "object", "required": ["alert_id"], "properties": { "alert_id": { "type": "string" } } } }),
        json!({ "name": "list_channels", "description": "Notification channels the caller can see.", "inputSchema": empty }),
        json!({ "name": "audit_log", "description": "Recent audited (mutating) API calls — who changed what.", "inputSchema": {
            "type": "object", "properties": { "limit": { "type": "integer", "description": "max entries (default 50)" } } } }),

        // ---- writes (each requires editor+ in the target workspace) ------------
        json!({ "name": "run_service_check", "description": "Probe a service immediately and return its result. Requires editor access to its workspace.", "inputSchema": {
            "type": "object", "required": ["monitor_id"], "properties": { "monitor_id": { "type": "string", "description": "the monitor's UUID (from list_services)" } } } }),
        json!({ "name": "toggle_alert_rule", "description": "Enable or disable an alert rule. Requires editor access to its workspace.", "inputSchema": {
            "type": "object", "required": ["alert_id", "enabled"], "properties": {
                "alert_id": { "type": "string" }, "enabled": { "type": "boolean" } } } }),
        json!({ "name": "create_service", "description": "Add a service check to a workspace.", "inputSchema": {
            "type": "object", "required": ["workspace_id", "name", "kind", "target"], "properties": {
                "workspace_id": { "type": "string" },
                "name": { "type": "string" },
                "kind": { "type": "string", "description": "http | tcp | ping | keyword | … (see an existing check for the set this build accepts)" },
                "target": { "type": "string", "description": "URL for http/keyword, host:port for tcp, host for ping" },
                "interval_secs": { "type": "integer" },
                "config": { "type": "object", "description": "kind-specific settings (keyword, expected status, timeout…)" } } } }),
        json!({ "name": "update_service", "description": "Edit a service check (any subset of fields).", "inputSchema": {
            "type": "object", "required": ["monitor_id"], "properties": {
                "monitor_id": { "type": "string" },
                "name": { "type": "string" }, "target": { "type": "string" },
                "interval_secs": { "type": "integer" }, "enabled": { "type": "boolean" },
                "config": { "type": "object" } } } }),
        json!({ "name": "delete_service", "description": "Delete a service check and its history.", "inputSchema": {
            "type": "object", "required": ["monitor_id"], "properties": { "monitor_id": { "type": "string" } } } }),
        json!({ "name": "create_alert_rule", "description": "Create an alert rule on a service, a host, or workspace-wide.", "inputSchema": {
            "type": "object", "required": ["workspace_id", "channel_ids"], "properties": {
                "workspace_id": { "type": "string" },
                "monitor_id": { "type": "string", "description": "target one service check" },
                "system_id": { "type": "string", "description": "target one host" },
                "scope_kind": { "type": "string", "enum": ["all_services", "all_hosts"], "description": "workspace-wide instead of a single target" },
                "channel_ids": { "type": "array", "items": { "type": "string" }, "description": "channels to notify (from list_channels)" },
                "condition": { "type": "object", "description": "threshold condition; omit for plain down/up" },
                "cooldown_secs": { "type": "integer" },
                "renotify_secs": { "type": "integer", "description": "re-notify cadence while firing; omit for off" } } } }),
        json!({ "name": "update_alert_rule", "description": "Edit an alert rule (any subset: enabled, condition, channels, cooldown, target).", "inputSchema": {
            "type": "object", "required": ["alert_id"], "properties": {
                "alert_id": { "type": "string" },
                "enabled": { "type": "boolean" }, "cooldown_secs": { "type": "integer" },
                "renotify_secs": { "type": "integer" },
                "channel_ids": { "type": "array", "items": { "type": "string" } },
                "condition": { "type": "object" },
                "monitor_id": { "type": "string" }, "system_id": { "type": "string" },
                "scope_kind": { "type": "string" }, "scope_workspace_id": { "type": "string" } } } }),
        json!({ "name": "delete_alert_rule", "description": "Delete an alert rule.", "inputSchema": {
            "type": "object", "required": ["alert_id"], "properties": { "alert_id": { "type": "string" } } } }),
        json!({ "name": "test_alert_rule", "description": "Send the rule's own notification (DOWN then UP) to its channels, without changing state.", "inputSchema": {
            "type": "object", "required": ["alert_id"], "properties": { "alert_id": { "type": "string" } } } }),
        json!({ "name": "create_channel", "description": "Add a notification channel to a workspace. Call `api_request` on GET /api/channel-types first for the kinds and their config fields.", "inputSchema": {
            "type": "object", "required": ["workspace_id", "name", "kind"], "properties": {
                "workspace_id": { "type": "string" }, "name": { "type": "string" },
                "kind": { "type": "string" }, "config": { "type": "object" } } } }),
        json!({ "name": "test_channel", "description": "Send a test notification through a saved channel.", "inputSchema": {
            "type": "object", "required": ["channel_id"], "properties": { "channel_id": { "type": "string" } } } }),
        json!({ "name": "delete_channel", "description": "Delete a notification channel.", "inputSchema": {
            "type": "object", "required": ["channel_id"], "properties": { "channel_id": { "type": "string" } } } }),
    ];
    defs.sort_by_key(|d| d["name"].as_str().unwrap_or("").to_string());
    Value::Array(defs)
}

/// Every route the hub serves. Hand-written so tool output stays readable — a
/// test below asserts it against the `.route(` literals in `main.rs`, so adding
/// an endpoint without listing it here fails the build.
const ENDPOINTS: &[(&str, &str)] = &[
    ("GET", "/healthz"),
    ("GET", "/readyz"),
    ("GET", "/exposure-check"),
    ("POST", "/pub/ingest"),
    ("POST", "/pub/kube"),
    ("GET|POST", "/pub/push/{token}"),
    ("GET", "/pub/agent.yaml"),
    ("GET", "/pub/install.sh"),
    ("GET", "/pub/tunnel"),
    ("GET|POST", "/api/setup"),
    ("POST", "/api/auth/login"),
    ("POST", "/api/auth/logout"),
    ("GET", "/api/me"),
    ("POST", "/api/me/password"),
    ("GET", "/api/me/2fa"),
    ("POST", "/api/me/2fa/start"),
    ("POST", "/api/me/2fa/enable"),
    ("POST", "/api/me/2fa/disable"),
    ("GET", "/api/me/passkeys"),
    ("DELETE", "/api/me/passkeys/{id}"),
    ("POST", "/api/me/passkeys/register/start"),
    ("POST", "/api/me/passkeys/register/finish"),
    ("GET|POST", "/api/pats"),
    ("DELETE", "/api/pats/{id}"),
    ("GET|POST", "/api/users"),
    ("PATCH|DELETE", "/api/users/{id}"),
    ("GET", "/api/users/{id}/memberships"),
    ("GET", "/api/about"),
    ("GET", "/api/audit"),
    ("PUT", "/api/admin/audit/retention"),
    ("POST", "/api/admin/exposure-check"),
    ("GET", "/api/admin/data"),
    ("GET|POST", "/api/admin/ingest-intervals"),
    ("GET", "/api/admin/logs"),
    ("POST", "/api/admin/retention"),
    ("POST", "/api/admin/data-cap"),
    ("POST", "/api/admin/data-cap/enforce"),
    ("POST", "/api/admin/config-retention"),
    ("GET", "/api/admin/backup"),
    ("POST", "/api/admin/restore"),
    ("GET|PUT", "/api/admin/backup/s3"),
    ("POST", "/api/admin/backup/s3/test"),
    ("POST", "/api/admin/backup/s3/upload"),
    ("GET", "/api/admin/backup/s3/list"),
    ("POST", "/api/admin/backup/s3/restore"),
    ("GET|PUT", "/api/admin/backup/schedule"),
    ("GET|POST", "/api/workspaces"),
    ("DELETE", "/api/workspaces/{id}"),
    ("GET", "/api/thresholds"),
    ("PUT", "/api/workspaces/{id}/thresholds"),
    ("GET|POST", "/api/workspaces/{id}/members"),
    ("DELETE", "/api/workspaces/{id}/members/{user_id}"),
    ("PUT", "/api/workspaces/{id}/members/{user_id}/exec"),
    ("GET", "/api/workspaces/{id}/member-candidates"),
    ("GET|POST", "/api/workspaces/{id}/keys"),
    ("DELETE", "/api/keys/{id}"),
    ("GET", "/api/keys/{id}/systems"),
    ("POST", "/api/workspaces/{id}/monitors"),
    ("GET|POST", "/api/workspaces/{id}/channels"),
    ("POST", "/api/workspaces/{id}/channels/test"),
    ("GET|POST", "/api/workspaces/{id}/alerts"),
    ("GET", "/api/workspaces/{id}/alert-events"),
    ("POST", "/api/workspaces/{id}/status-pages"),
    ("DELETE", "/api/status-pages/{id}"),
    ("GET", "/api/channel-types"),
    ("GET", "/api/channels"),
    ("PATCH|DELETE", "/api/channels/{id}"),
    ("POST", "/api/channels/{id}/test"),
    ("GET", "/api/channels/{id}/alerts"),
    ("GET|PATCH|DELETE", "/api/alerts/{id}"),
    ("POST", "/api/alerts/{id}/test"),
    ("GET", "/api/systems"),
    ("PATCH|DELETE", "/api/systems/{id}"),
    ("GET", "/api/systems/{id}/alerts"),
    ("GET", "/api/systems/{id}/metrics"),
    ("GET", "/api/systems/{id}/containers"),
    ("GET", "/api/systems/{id}/temps"),
    ("GET", "/api/systems/{id}/gpu"),
    ("GET|PUT", "/api/systems/{id}/shell"),
    ("POST", "/api/systems/{id}/console/ticket"),
    ("GET", "/api/systems/{id}/console"),
    ("GET", "/api/systems/{id}/kube/summary"),
    ("GET", "/api/systems/{id}/kube/aggregate"),
    ("GET", "/api/systems/{id}/kube/containers"),
    ("GET", "/api/systems/{id}/kube/series"),
    ("GET", "/api/systems/{id}/kube/series-by"),
    ("GET", "/api/kube/summaries"),
    ("GET", "/api/fleet"),
    ("GET", "/api/monitors"),
    ("GET|PATCH|DELETE", "/api/monitors/{id}"),
    ("GET", "/api/monitors/{id}/debug"),
    ("GET", "/api/monitors/{id}/heartbeats"),
    ("GET", "/api/monitors/{id}/events"),
    ("GET", "/api/monitors/{id}/alerts"),
    ("GET", "/api/events"),
    ("GET|POST", "/api/ssh-keys"),
    ("DELETE", "/api/ssh-keys/{id}"),
    ("POST", "/mcp"),
];

async fn call_tool(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    params: Option<&Value>,
) -> Result<Value, (i64, String)> {
    let p = params.ok_or((-32602, "missing params".to_string()))?;
    let name = p.get("name").and_then(Value::as_str).unwrap_or("");
    let args = p.get("arguments").cloned().unwrap_or_else(|| json!({}));

    let out = match name {
        // Hand-shaped summaries: these answer "what is going on" in one screen,
        // which the raw endpoints spread over several calls.
        "list_systems" => t_list_systems(state, user).await,
        "list_services" => t_list_services(state, user).await,
        "alerts_firing" => t_alerts_firing(state, user).await,
        "recent_events" => t_recent_events(state, user, &args).await,
        "run_service_check" => t_run_check(state, user, &args).await,
        "toggle_alert_rule" => t_toggle_rule(state, user, &args).await,
        "list_endpoints" => Ok(json!(ENDPOINTS
            .iter()
            .map(|(m, p)| json!({ "methods": m, "path": p }))
            .collect::<Vec<_>>())),
        "api_request" => t_api_request(state, headers, &args).await,
        // Everything else is a named front door onto one endpoint.
        other => match curated(other, &args) {
            Some(Ok((method, path, body))) => {
                dispatch(state, headers, &method, &path, body.as_ref()).await
            }
            Some(Err(e)) => Err(e),
            None => return Err((-32602, format!("unknown tool: {other}"))),
        },
    };
    // Tool errors are reported in-band (isError) per MCP, not as JSON-RPC errors.
    Ok(match out {
        Ok(v) => json!({ "content": [{ "type": "text", "text": pretty(&v) }] }),
        Err(e) => {
            json!({ "content": [{ "type": "text", "text": format!("Error: {e}") }], "isError": true })
        }
    })
}

// ---- in-process dispatch into the hub's own router --------------------------

/// Re-enter the router with the caller's own credentials. Forwarding the
/// `Authorization` / `Cookie` headers (rather than trusting the already-resolved
/// `CurrentUser`) is what keeps this honest: the handler re-authenticates and
/// re-authorizes exactly as it would for an outside request, and the audit
/// middleware records the call under the right user.
async fn dispatch(
    state: &AppState,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    let router = state
        .router
        .get()
        .ok_or("the hub is still starting up; try again in a moment")?
        .clone();
    let method: Method = method
        .parse()
        .map_err(|_| format!("bad HTTP method: {method}"))?;

    let mut req = Request::builder().method(method).uri(path);
    for h in ["authorization", "cookie"] {
        if let Some(v) = headers.get(h) {
            req = req.header(h, v);
        }
    }
    let req = req
        .header("content-type", "application/json")
        .header("x-vantage-mcp", "1")
        .body(match body {
            Some(b) => Body::from(serde_json::to_vec(b).map_err(|e| e.to_string())?),
            None => Body::empty(),
        })
        .map_err(|e| format!("bad request: {e}"))?;

    let resp = router.oneshot(req).await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), MAX_BODY)
        .await
        .map_err(|_| format!("response larger than {} KB — narrow the range or add a limit", MAX_BODY / 1024))?;
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    if status.is_success() {
        Ok(json!({ "status": status.as_u16(), "body": body }))
    } else {
        Err(format!(
            "{} {}{}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("error"),
            match &body {
                Value::Null => String::new(),
                v => format!(" — {v}"),
            }
        ))
    }
}

async fn t_api_request(
    state: &AppState,
    headers: &HeaderMap,
    args: &Value,
) -> Result<Value, String> {
    let method = args
        .get("method")
        .and_then(Value::as_str)
        .ok_or("method is required")?
        .to_uppercase();
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or("path is required")?;
    check_path(path)?;
    dispatch(state, headers, &method, path, args.get("body")).await
}

/// What `api_request` may address. Anything outside `/api` and `/pub` falls
/// through to the SPA fallback, which answers `index.html` with a 200 — an
/// assistant would read that as a successful call returning HTML.
fn check_path(path: &str) -> Result<(), String> {
    if !path.starts_with("/api/") && !path.starts_with("/pub/") {
        return Err("path must start with /api/ or /pub/ — call list_endpoints".to_string());
    }
    if path.starts_with("/mcp") {
        return Err("refusing to call /mcp from inside itself".to_string());
    }
    if path.contains("/console") && !path.ends_with("/ticket") {
        return Err("that route is a WebSocket; it cannot be used over MCP".to_string());
    }
    Ok(())
}

// ---- curated tools: one named front door per endpoint -----------------------

fn uuid_arg(args: &Value, key: &str) -> Result<Uuid, String> {
    args.get(key)
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| format!("{key} must be a UUID"))
}

/// Copy the named arguments through to a request body, dropping absent ones so
/// a PATCH stays a partial update instead of nulling fields the caller left out.
fn body_of(args: &Value, keys: &[&str]) -> Value {
    let mut o = serde_json::Map::new();
    for k in keys {
        if let Some(v) = args.get(*k) {
            if !v.is_null() {
                o.insert((*k).to_string(), v.clone());
            }
        }
    }
    Value::Object(o)
}

fn limit_arg(args: &Value, default: i64) -> i64 {
    args.get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(default)
        .clamp(1, 500)
}

type Call = (String, String, Option<Value>);

/// Map a curated tool call onto `(method, path, body)`. `None` = not a curated
/// tool name. Keeping every wrapper in one table is what stops a tool from
/// quietly pointing at a path that no longer exists.
fn curated(name: &str, args: &Value) -> Option<Result<Call, String>> {
    fn get(path: String) -> Result<Call, String> {
        Ok(("GET".into(), path, None))
    }
    let out: Result<Call, String> = match name {
        "list_workspaces" => get("/api/workspaces".into()),
        "fleet" => get("/api/fleet".into()),
        "kube_summaries" => get("/api/kube/summaries".into()),
        "list_channels" => get("/api/channels".into()),
        "audit_log" => get(format!("/api/audit?limit={}", limit_arg(args, 50))),
        "system_metrics" => uuid_arg(args, "system_id").and_then(|id| {
            let range = args.get("range").and_then(Value::as_str).unwrap_or("1h");
            if !range
                .chars()
                .all(|c| c.is_ascii_alphanumeric())
            {
                return Err("range must be something like 1h, 24h or 7d".to_string());
            }
            get(format!("/api/systems/{id}/metrics?range={range}"))
        }),
        "system_containers" => uuid_arg(args, "system_id")
            .and_then(|id| get(format!("/api/systems/{id}/containers"))),
        "get_service" => {
            uuid_arg(args, "monitor_id").and_then(|id| get(format!("/api/monitors/{id}")))
        }
        "service_heartbeats" => uuid_arg(args, "monitor_id").and_then(|id| {
            get(format!(
                "/api/monitors/{id}/heartbeats?limit={}",
                limit_arg(args, 50)
            ))
        }),
        "list_alert_rules" => uuid_arg(args, "workspace_id")
            .and_then(|ws| get(format!("/api/workspaces/{ws}/alerts"))),
        "get_alert_rule" => {
            uuid_arg(args, "alert_id").and_then(|id| get(format!("/api/alerts/{id}")))
        }

        "create_service" => uuid_arg(args, "workspace_id").map(|ws| {
            (
                "POST".into(),
                format!("/api/workspaces/{ws}/monitors"),
                Some(body_of(
                    args,
                    &["name", "kind", "target", "interval_secs", "config"],
                )),
            )
        }),
        "update_service" => uuid_arg(args, "monitor_id").map(|id| {
            (
                "PATCH".into(),
                format!("/api/monitors/{id}"),
                Some(body_of(
                    args,
                    &["name", "target", "interval_secs", "enabled", "config"],
                )),
            )
        }),
        "delete_service" => uuid_arg(args, "monitor_id")
            .map(|id| ("DELETE".into(), format!("/api/monitors/{id}"), None)),
        "create_alert_rule" => uuid_arg(args, "workspace_id").map(|ws| {
            (
                "POST".into(),
                format!("/api/workspaces/{ws}/alerts"),
                Some(body_of(
                    args,
                    &[
                        "monitor_id",
                        "system_id",
                        "scope_kind",
                        "channel_ids",
                        "condition",
                        "cooldown_secs",
                        "renotify_secs",
                    ],
                )),
            )
        }),
        "update_alert_rule" => uuid_arg(args, "alert_id").map(|id| {
            (
                "PATCH".into(),
                format!("/api/alerts/{id}"),
                Some(body_of(
                    args,
                    &[
                        "enabled",
                        "cooldown_secs",
                        "renotify_secs",
                        "channel_ids",
                        "condition",
                        "monitor_id",
                        "system_id",
                        "scope_kind",
                        "scope_workspace_id",
                    ],
                )),
            )
        }),
        "delete_alert_rule" => uuid_arg(args, "alert_id")
            .map(|id| ("DELETE".into(), format!("/api/alerts/{id}"), None)),
        "test_alert_rule" => uuid_arg(args, "alert_id").map(|id| {
            (
                "POST".into(),
                format!("/api/alerts/{id}/test"),
                Some(json!({})),
            )
        }),
        "create_channel" => uuid_arg(args, "workspace_id").map(|ws| {
            (
                "POST".into(),
                format!("/api/workspaces/{ws}/channels"),
                Some(body_of(args, &["name", "kind", "config"])),
            )
        }),
        "test_channel" => uuid_arg(args, "channel_id").map(|id| {
            (
                "POST".into(),
                format!("/api/channels/{id}/test"),
                Some(json!({})),
            )
        }),
        "delete_channel" => uuid_arg(args, "channel_id")
            .map(|id| ("DELETE".into(), format!("/api/channels/{id}"), None)),
        _ => return None,
    };
    Some(out)
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

// ---- read tools (scoped to the caller's workspaces) -------------------------

async fn t_list_systems(state: &AppState, user: &CurrentUser) -> Result<Value, String> {
    let rows: Vec<(String, String, String, Option<chrono::DateTime<Utc>>)> = sqlx::query_as(
        "SELECT s.name, s.kind, n.name, s.last_seen FROM systems s \
         JOIN workspaces n ON n.id = s.workspace_id \
         WHERE $1 OR s.workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $2) \
         ORDER BY n.name, s.name",
    )
    .bind(user.can_read_all())
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .map_err(|e| e.to_string())?;
    let now = Utc::now();
    Ok(json!(rows
        .iter()
        .map(|(name, kind, ws, last)| json!({
            "name": name, "kind": kind, "workspace": ws,
            "online": last.map(|t| (now - t).num_seconds() < 120).unwrap_or(false),
            "last_seen": last.map(|t| t.to_rfc3339()),
        }))
        .collect::<Vec<_>>()))
}

async fn t_list_services(state: &AppState, user: &CurrentUser) -> Result<Value, String> {
    let mons: Vec<(Uuid, String, String, String, String, bool)> = sqlx::query_as(
        "SELECT m.id, m.name, m.kind::text, m.target, n.name, m.enabled FROM monitors m \
         JOIN workspaces n ON n.id = m.workspace_id \
         WHERE $1 OR m.workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $2) \
         ORDER BY n.name, m.name",
    )
    .bind(user.can_read_all())
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .map_err(|e| e.to_string())?;
    let ids: Vec<Uuid> = mons.iter().map(|m| m.0).collect();
    let beats: Vec<(Uuid, bool, Option<String>)> = sqlx::query_as(
        // LATERAL LIMIT 1 per monitor, same as the monitors list — `DISTINCT ON` over
        // `= ANY($1)` sorted every heartbeat row of every monitor to keep the newest.
        "SELECT s.mid, h.up, h.message FROM unnest($1::uuid[]) AS s(mid) \
         JOIN LATERAL (SELECT up, message FROM heartbeats WHERE monitor_id = s.mid \
                       ORDER BY time DESC LIMIT 1) h ON true",
    )
    .bind(&ids)
    .fetch_all(&state.data)
    .await
    .map_err(|e| e.to_string())?;
    let latest: std::collections::HashMap<Uuid, (bool, Option<String>)> = beats
        .into_iter()
        .map(|(id, up, msg)| (id, (up, msg)))
        .collect();
    Ok(json!(mons
        .iter()
        .map(|(id, name, kind, target, ws, enabled)| {
            let st = latest.get(id);
            json!({
                "id": id, "name": name, "kind": kind, "target": target, "workspace": ws,
                "enabled": enabled,
                "status": st.map(|(up, _)| if *up { "up" } else { "down" }).unwrap_or("pending"),
                "message": st.and_then(|(_, m)| m.clone()),
            })
        })
        .collect::<Vec<_>>()))
}

async fn t_alerts_firing(state: &AppState, user: &CurrentUser) -> Result<Value, String> {
    let rows: Vec<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<chrono::DateTime<Utc>>,
    )> = sqlx::query_as(
        "SELECT m.name, s.name, n.name, st.last_changed \
             FROM alert_state st JOIN alerts r ON r.id = st.alert_id \
             LEFT JOIN monitors m ON m.id = r.monitor_id \
             LEFT JOIN systems s ON s.id = r.system_id \
             LEFT JOIN workspaces n ON n.id = COALESCE(m.workspace_id, s.workspace_id) \
             WHERE st.firing = true AND r.enabled = true \
             AND ($1 OR COALESCE(m.workspace_id, s.workspace_id) IN \
                  (SELECT workspace_id FROM memberships WHERE user_id = $2)) \
             ORDER BY st.last_changed",
    )
    .bind(user.can_read_all())
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .map_err(|e| e.to_string())?;
    Ok(json!(rows
        .iter()
        .map(|(m, s, ws, since)| json!({
            "target": m.clone().or_else(|| s.clone()),
            "kind": if m.is_some() { "service" } else { "host" },
            "workspace": ws,
            "firing_since": since.map(|t| t.to_rfc3339()),
        }))
        .collect::<Vec<_>>()))
}

async fn t_recent_events(
    state: &AppState,
    user: &CurrentUser,
    args: &Value,
) -> Result<Value, String> {
    let limit = args
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(20)
        .clamp(1, 200);
    let rows: Vec<(
        chrono::DateTime<Utc>,
        bool,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT e.at, e.firing, e.message, m.name, s.name, n.name \
             FROM alert_events e JOIN alerts r ON r.id = e.alert_id \
             LEFT JOIN monitors m ON m.id = r.monitor_id \
             LEFT JOIN systems s ON s.id = r.system_id \
             LEFT JOIN workspaces n ON n.id = COALESCE(m.workspace_id, s.workspace_id) \
             WHERE $1 OR COALESCE(m.workspace_id, s.workspace_id) IN \
                   (SELECT workspace_id FROM memberships WHERE user_id = $2) \
             ORDER BY e.at DESC LIMIT $3",
    )
    .bind(user.can_read_all())
    .bind(user.id)
    .bind(limit)
    .fetch_all(&state.config)
    .await
    .map_err(|e| e.to_string())?;
    Ok(json!(rows
        .iter()
        .map(|(at, firing, msg, m, s, ws)| json!({
            "at": at.to_rfc3339(),
            "state": if *firing { "down" } else { "recovered" },
            "target": m.clone().or_else(|| s.clone()),
            "workspace": ws,
            "message": msg,
        }))
        .collect::<Vec<_>>()))
}

// ---- write tools (require editor in the target's workspace) -----------------

async fn require_editor(state: &AppState, user: &CurrentUser, ws: Uuid) -> Result<(), String> {
    rbac::require_role(state, user, ws, Role::Editor)
        .await
        .map(|_| ())
        .map_err(|_| "forbidden: editor access to this workspace is required".to_string())
}

async fn t_run_check(state: &AppState, user: &CurrentUser, args: &Value) -> Result<Value, String> {
    let id = args
        .get("monitor_id")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or("monitor_id must be a UUID")?;
    let ws: Option<(Uuid,)> = sqlx::query_as("SELECT workspace_id FROM monitors WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.config)
        .await
        .map_err(|e| e.to_string())?;
    let (ws,) = ws.ok_or("monitor not found")?;
    require_editor(state, user, ws).await?;
    crate::probe::check_once(state, id).await;
    let beat: Option<(bool, Option<String>)> = sqlx::query_as(
        "SELECT up, message FROM heartbeats WHERE monitor_id = $1 ORDER BY time DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&state.data)
    .await
    .map_err(|e| e.to_string())?;
    Ok(match beat {
        Some((up, msg)) => json!({ "status": if up { "up" } else { "down" }, "message": msg }),
        None => {
            json!({ "status": "no result", "message": "push monitors are not actively probed" })
        }
    })
}

async fn t_toggle_rule(
    state: &AppState,
    user: &CurrentUser,
    args: &Value,
) -> Result<Value, String> {
    let id = args
        .get("alert_id")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or("alert_id must be a UUID")?;
    let enabled = args
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or("enabled must be true/false")?;
    let ws: Option<(Option<Uuid>,)> = sqlx::query_as(
        "SELECT COALESCE(m.workspace_id, s.workspace_id) FROM alerts r \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN systems s ON s.id = r.system_id WHERE r.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.config)
    .await
    .map_err(|e| e.to_string())?;
    let ws = ws.and_then(|(n,)| n).ok_or("alert rule not found")?;
    require_editor(state, user, ws).await?;
    sqlx::query("UPDATE alerts SET enabled = $2 WHERE id = $1")
        .bind(id)
        .bind(enabled)
        .execute(&state.config)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "alert_id": id, "enabled": enabled }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The MCP surface is only "complete" as long as `ENDPOINTS` still lists
    /// every route. `api_request` can reach a new endpoint the day it lands, but
    /// an assistant only finds it through `list_endpoints` — so adding a route
    /// to `main.rs` without listing it here fails right there.
    #[test]
    fn endpoint_table_covers_every_route() {
        let main = include_str!("main.rs");
        let mut missing = Vec::new();
        for line in main.lines() {
            let Some(rest) = line.trim().strip_prefix(".route(\"") else {
                continue;
            };
            let Some(path) = rest.split('"').next() else {
                continue;
            };
            if !ENDPOINTS.iter().any(|(_, p)| *p == path) {
                missing.push(path.to_string());
            }
        }
        assert!(
            missing.is_empty(),
            "routes in main.rs missing from mcp::ENDPOINTS: {missing:?}"
        );
    }

    /// Every curated tool must be dispatchable — a name in `tool_defs` with no
    /// arm in `curated()` (or vice versa) is a tool that advertises itself and
    /// then answers "unknown tool".
    #[test]
    fn every_advertised_tool_is_reachable() {
        // Tools handled directly in `call_tool` rather than through `curated()`.
        let direct = [
            "list_systems",
            "list_services",
            "alerts_firing",
            "recent_events",
            "run_service_check",
            "toggle_alert_rule",
            "list_endpoints",
            "api_request",
        ];
        let defs = tool_defs();
        let names: Vec<&str> = defs
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["name"].as_str().unwrap())
            .collect();
        assert!(names.len() > 25, "expected a broad tool surface");
        for n in &names {
            if direct.contains(n) {
                continue;
            }
            assert!(
                curated(n, &json!({})).is_some(),
                "tool `{n}` is advertised but has no arm in curated()"
            );
        }
        // …and nothing is dispatchable without being advertised.
        for n in ["create_service", "delete_channel", "system_metrics"] {
            assert!(names.contains(&n), "`{n}` dropped out of tool_defs");
        }
    }

    /// `api_request` must not become a way to read the SPA shell or re-enter
    /// itself — both would look like a successful call and answer nonsense.
    #[test]
    fn api_request_rejects_paths_outside_the_api() {
        for (path, want) in [
            ("/dashboard", "must start with"),
            ("/mcp", "must start with"),
            ("/api/systems/x/console", "WebSocket"),
        ] {
            let err = check_path(path).unwrap_err();
            assert!(err.contains(want), "path {path} gave: {err}");
        }
        for path in ["/api/systems", "/api/systems/x/console/ticket", "/pub/ingest"] {
            assert!(check_path(path).is_ok(), "{path} should be allowed");
        }
    }
}
