//! Embedded MCP (Model Context Protocol) server over HTTP, so AI assistants can
//! read and operate the monitor. One endpoint, `POST /mcp`, speaks JSON-RPC 2.0
//! (initialize / tools/list / tools/call). Auth is a PAT via `Authorization:
//! Bearer …` (the `CurrentUser` extractor), so every tool runs with that user's
//! RBAC — reads are scoped to their workspaces, writes require editor there.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::rbac::{self, Role};
use crate::AppState;

const PROTOCOL: &str = "2024-11-05";

/// POST /mcp — a single JSON-RPC request (or notification). Authenticated by PAT.
pub async fn handle(
    State(state): State<AppState>,
    user: CurrentUser,
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
        "tools/call" => call_tool(&state, &user, req.get("params")).await,
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
    json!([
        { "name": "list_systems", "description": "List monitored hosts with their workspace and online state.", "inputSchema": empty },
        { "name": "list_services", "description": "List service checks (monitors) with up/down state and workspace.", "inputSchema": empty },
        { "name": "alerts_firing", "description": "Alert rules that are currently firing.", "inputSchema": empty },
        { "name": "recent_events", "description": "Recent alert fire/recover events.", "inputSchema": {
            "type": "object", "properties": { "limit": { "type": "integer", "description": "max events (default 20)" } } } },
        { "name": "run_service_check", "description": "Probe a service immediately and return its result. Requires editor access to its workspace.", "inputSchema": {
            "type": "object", "required": ["monitor_id"], "properties": { "monitor_id": { "type": "string", "description": "the monitor's UUID (from list_services)" } } } },
        { "name": "toggle_alert_rule", "description": "Enable or disable an alert rule. Requires editor access to its workspace.", "inputSchema": {
            "type": "object", "required": ["alert_id", "enabled"], "properties": {
                "alert_id": { "type": "string" }, "enabled": { "type": "boolean" } } } },
    ])
}

async fn call_tool(
    state: &AppState,
    user: &CurrentUser,
    params: Option<&Value>,
) -> Result<Value, (i64, String)> {
    let p = params.ok_or((-32602, "missing params".to_string()))?;
    let name = p.get("name").and_then(Value::as_str).unwrap_or("");
    let args = p.get("arguments").cloned().unwrap_or_else(|| json!({}));

    let out = match name {
        "list_systems" => t_list_systems(state, user).await,
        "list_services" => t_list_services(state, user).await,
        "alerts_firing" => t_alerts_firing(state, user).await,
        "recent_events" => t_recent_events(state, user, &args).await,
        "run_service_check" => t_run_check(state, user, &args).await,
        "toggle_alert_rule" => t_toggle_rule(state, user, &args).await,
        other => return Err((-32602, format!("unknown tool: {other}"))),
    };
    // Tool errors are reported in-band (isError) per MCP, not as JSON-RPC errors.
    Ok(match out {
        Ok(v) => json!({ "content": [{ "type": "text", "text": pretty(&v) }] }),
        Err(e) => {
            json!({ "content": [{ "type": "text", "text": format!("Error: {e}") }], "isError": true })
        }
    })
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
        "SELECT DISTINCT ON (monitor_id) monitor_id, up, message FROM heartbeats \
         WHERE monitor_id = ANY($1) ORDER BY monitor_id, time DESC",
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
