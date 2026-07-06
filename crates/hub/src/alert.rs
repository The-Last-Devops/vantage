//! Alert engine.
//!
//! A background loop evaluates every enabled alert rule, tracks a per-rule
//! firing/ok state in `alert_state`, and notifies on transitions (plus recovery,
//! plus re-notify after the rule's cooldown while still firing).
//!
//! Conditions live in `alert_rules.condition` (JSONB) so new condition shapes
//! need no schema change. Notification channels dispatch by `kind` + `config`
//! (JSONB), so adding a channel type is one match arm.

use std::time::Duration;

use serde_json::Value;
use sqlx::types::Json;
use uuid::Uuid;

use crate::AppState;

const TICK: Duration = Duration::from_secs(10);

struct ChannelDef {
    kind: String,
    config: Value,
    name: String,
}

struct Rule {
    id: Uuid,
    monitor_id: Option<Uuid>,
    system_id: Option<Uuid>,
    /// Workspace-wide scope: "all_services" | "all_hosts" (+ scope_ns), else specific target.
    scope_kind: Option<String>,
    scope_ns: Option<Uuid>,
    condition: Value,
    /// Re-notify cadence while still firing; None = notify once, never repeat.
    renotify_secs: Option<i32>,
    channels: Vec<ChannelDef>,
}

struct Eval {
    firing: bool,
    message: String,
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("alert client");
        loop {
            if let Err(e) = tick(&state, &client).await {
                tracing::error!(error = %e, "alert tick");
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

async fn tick(state: &AppState, client: &reqwest::Client) -> anyhow::Result<()> {
    let rules = load_rules(state).await?;
    for rule in rules {
        let eval = match evaluate(state, &rule).await {
            Ok(Some(e)) => e,
            Ok(None) => continue, // not enough data yet
            Err(e) => {
                tracing::error!(error = %e, rule = %rule.id, "evaluate");
                continue;
            }
        };

        // Prior state.
        let prior: Option<(bool, Option<chrono::DateTime<chrono::Utc>>)> =
            sqlx::query_as("SELECT firing, last_notified FROM alert_state WHERE alert_id = $1")
                .bind(rule.id)
                .fetch_optional(&state.config)
                .await?;
        let (was_firing, last_notified) = prior.unwrap_or((false, None));

        let now = chrono::Utc::now();
        // Re-notify only when the rule opts in (renotify_secs set) and the interval
        // has elapsed since the last notification. None = fire once, never repeat.
        let renotify_due = match (rule.renotify_secs, last_notified) {
            (Some(secs), Some(t)) => (now - t).num_seconds() >= secs as i64,
            (Some(_), None) => true,
            (None, _) => false,
        };

        let should_notify = match (was_firing, eval.firing) {
            (false, true) => true,
            (true, true) => renotify_due,
            (true, false) => true,
            _ => false,
        };

        if should_notify {
            let (target, kind_label, workspace) = target_info(state, &rule).await;
            let n = crate::notify::Notification {
                firing: eval.firing,
                repeat: was_firing && eval.firing, // a re-notify while still firing
                target,
                kind_label: kind_label.to_string(),
                workspace,
                condition: condition_text(&rule),
                detail: eval.message.clone(),
                at: now.format("%Y-%m-%d %H:%M UTC").to_string(),
            };
            notify(client, &rule, &n).await;
        }

        // Record fired/recovered transitions for the history feed (not re-notifies).
        if eval.firing != was_firing {
            let _ = sqlx::query(
                "INSERT INTO alert_events (alert_id, firing, message) VALUES ($1, $2, $3)",
            )
            .bind(rule.id)
            .bind(eval.firing)
            .bind(&eval.message)
            .execute(&state.config)
            .await;
        }

        // Persist state on EVERY evaluation that produced a verdict — so a brand-new
        // rule that's healthy from the start gets an `ok` row immediately instead of
        // showing "Pending" forever (it only ever had a row written on a transition).
        // last_changed advances only on an actual firing flip; last_notified only when
        // we notified.
        sqlx::query(
            "INSERT INTO alert_state (alert_id, firing, last_changed, last_notified) \
             VALUES ($1, $2, now(), CASE WHEN $3 THEN now() ELSE NULL END) \
             ON CONFLICT (alert_id) DO UPDATE SET \
               firing = EXCLUDED.firing, \
               last_changed = CASE WHEN alert_state.firing <> EXCLUDED.firing \
                                   THEN now() ELSE alert_state.last_changed END, \
               last_notified = CASE WHEN $3 THEN now() ELSE alert_state.last_notified END",
        )
        .bind(rule.id)
        .bind(eval.firing)
        .bind(should_notify)
        .execute(&state.config)
        .await?;
    }
    Ok(())
}

async fn load_rules(state: &AppState) -> anyhow::Result<Vec<Rule>> {
    // One row per (rule × channel); a rule with no channels still appears (LEFT
    // JOIN) so it can record fire/recover events even though it can't notify.
    type Row = (
        Uuid,
        Option<Uuid>,
        Option<Uuid>,
        Option<String>,
        Option<Uuid>,
        Json<Value>,
        Option<i32>,
        Option<String>,
        Option<Json<Value>>,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT r.id, r.monitor_id, r.system_id, r.scope_kind, r.scope_workspace_id, \
                r.condition, r.renotify_secs, c.kind, c.config, c.name \
         FROM alerts r \
         LEFT JOIN alert_channels ac ON ac.alert_id = r.id \
         LEFT JOIN channels c ON c.id = ac.channel_id \
         WHERE r.enabled = true \
         ORDER BY r.id",
    )
    .fetch_all(&state.config)
    .await?;

    // Collapse the rows into one Rule per id, accumulating its channels.
    let mut rules: Vec<Rule> = Vec::new();
    for (
        id,
        monitor_id,
        system_id,
        scope_kind,
        scope_ns,
        condition,
        renotify_secs,
        kind,
        config,
        name,
    ) in rows
    {
        if rules.last().map(|r| r.id) != Some(id) {
            rules.push(Rule {
                id,
                monitor_id,
                system_id,
                scope_kind,
                scope_ns,
                condition: condition.0,
                renotify_secs,
                channels: Vec::new(),
            });
        }
        if let (Some(kind), Some(config), Some(name)) = (kind, config, name) {
            rules.last_mut().unwrap().channels.push(ChannelDef {
                kind,
                config: config.0,
                name,
            });
        }
    }
    Ok(rules)
}

/// Returns None when there's no data yet to judge (avoids false alerts on cold start).
async fn evaluate(state: &AppState, rule: &Rule) -> anyhow::Result<Option<Eval>> {
    if let Some(mid) = rule.monitor_id {
        return evaluate_monitor(state, mid).await;
    }
    if let Some(sid) = rule.system_id {
        return evaluate_server(state, sid, &rule.condition).await;
    }
    if let (Some(kind), Some(ws)) = (rule.scope_kind.as_deref(), rule.scope_ns) {
        return evaluate_scope(state, kind, ws, &rule.condition).await;
    }
    Ok(None)
}

/// A workspace-wide rule: evaluate every matching target and aggregate. Fires when
/// ANY target is failing; the message names them. None until at least one target has data.
async fn evaluate_scope(
    state: &AppState,
    kind: &str,
    ws: Uuid,
    cond: &Value,
) -> anyhow::Result<Option<Eval>> {
    let (targets, label): (Vec<(Uuid, String)>, &str) = match kind {
        "all_services" => (
            sqlx::query_as(
                "SELECT id, name FROM monitors WHERE workspace_id = $1 AND enabled = true",
            )
            .bind(ws)
            .fetch_all(&state.config)
            .await?,
            "services",
        ),
        "all_hosts" => (
            sqlx::query_as("SELECT id, name FROM systems WHERE workspace_id = $1")
                .bind(ws)
                .fetch_all(&state.config)
                .await?,
            "hosts",
        ),
        _ => return Ok(None),
    };
    if targets.is_empty() {
        return Ok(None);
    }
    let mut any_data = false;
    let mut down: Vec<String> = Vec::new();
    for (id, name) in &targets {
        let e = if kind == "all_services" {
            evaluate_monitor(state, *id).await?
        } else {
            evaluate_server(state, *id, cond).await?
        };
        if let Some(e) = e {
            any_data = true;
            if e.firing {
                down.push(name.clone());
            }
        }
    }
    if !any_data {
        return Ok(None);
    }
    let firing = !down.is_empty();
    let message = if firing {
        format!(
            "{} of {} {label} affected: {}",
            down.len(),
            targets.len(),
            down.join(", ")
        )
    } else {
        format!("all {} {label} ok", targets.len())
    };
    Ok(Some(Eval { firing, message }))
}

async fn evaluate_monitor(state: &AppState, monitor_id: Uuid) -> anyhow::Result<Option<Eval>> {
    let latest: Option<(bool, Option<String>)> = sqlx::query_as(
        "SELECT up, message FROM heartbeats WHERE monitor_id = $1 ORDER BY time DESC LIMIT 1",
    )
    .bind(monitor_id)
    .fetch_optional(&state.data)
    .await?;
    let name = monitor_name(state, monitor_id).await.unwrap_or_default();
    Ok(latest.map(|(up, msg)| Eval {
        firing: !up,
        message: if up {
            format!("monitor '{name}' is up")
        } else {
            format!("monitor '{name}' is DOWN ({})", msg.unwrap_or_default())
        },
    }))
}

async fn evaluate_server(
    state: &AppState,
    system_id: Uuid,
    cond: &Value,
) -> anyhow::Result<Option<Eval>> {
    let name = server_name(state, system_id).await.unwrap_or_default();

    // Offline check: no fresh sample within N seconds.
    if let Some(secs) = cond.get("offline_secs").and_then(Value::as_i64) {
        let last_seen: Option<(Option<chrono::DateTime<chrono::Utc>>,)> =
            sqlx::query_as("SELECT last_seen FROM systems WHERE id = $1")
                .bind(system_id)
                .fetch_optional(&state.config)
                .await?;
        let last = last_seen.and_then(|(t,)| t);
        let firing = match last {
            Some(t) => (chrono::Utc::now() - t).num_seconds() > secs,
            None => true,
        };
        return Ok(Some(Eval {
            firing,
            message: if firing {
                format!("server '{name}' is OFFLINE (no data > {secs}s)")
            } else {
                format!("server '{name}' is online")
            },
        }));
    }

    // Metric threshold: {"metric":"cpu_percent","op":">","value":90}
    let metric = cond.get("metric").and_then(Value::as_str);
    let op = cond.get("op").and_then(Value::as_str);
    let threshold = cond.get("value").and_then(Value::as_f64);
    if let (Some(metric), Some(op), Some(threshold)) = (metric, op, threshold) {
        let row: Option<(f64, f64, i64, i64)> = sqlx::query_as(
            "SELECT cpu_percent, load1, mem_used, mem_total FROM system_metrics \
             WHERE system_id = $1 ORDER BY time DESC LIMIT 1",
        )
        .bind(system_id)
        .fetch_optional(&state.data)
        .await?;
        let Some((cpu, load1, mem_used, mem_total)) = row else {
            return Ok(None);
        };
        let current = match metric {
            "cpu_percent" => cpu,
            "load1" => load1,
            "mem_percent" => {
                if mem_total > 0 {
                    mem_used as f64 / mem_total as f64 * 100.0
                } else {
                    0.0
                }
            }
            _ => return Ok(None),
        };
        let firing = compare(current, op, threshold);
        return Ok(Some(Eval {
            firing,
            message: format!(
                "server '{name}' {metric}={current:.1} {op} {threshold} -> {}",
                if firing { "BREACH" } else { "ok" }
            ),
        }));
    }

    Ok(None)
}

fn compare(a: f64, op: &str, b: f64) -> bool {
    match op {
        ">" => a > b,
        ">=" => a >= b,
        "<" => a < b,
        "<=" => a <= b,
        "==" => (a - b).abs() < f64::EPSILON,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_ops() {
        assert!(compare(91.0, ">", 90.0));
        assert!(!compare(90.0, ">", 90.0));
        assert!(compare(90.0, ">=", 90.0));
        assert!(compare(10.0, "<", 20.0));
        assert!(compare(20.0, "<=", 20.0));
        assert!(compare(5.0, "==", 5.0));
        assert!(!compare(5.0, "??", 5.0)); // unknown operator never fires
    }
}

async fn monitor_name(state: &AppState, id: Uuid) -> Option<String> {
    sqlx::query_as::<_, (String,)>("SELECT name FROM monitors WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.config)
        .await
        .ok()
        .flatten()
        .map(|(n,)| n)
}

async fn server_name(state: &AppState, id: Uuid) -> Option<String> {
    sqlx::query_as::<_, (String,)>("SELECT name FROM systems WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.config)
        .await
        .ok()
        .flatten()
        .map(|(n,)| n)
}

// ---- notification dispatch --------------------------------------------------

/// Fan a notification out to every channel wired to the rule. One channel
/// failing must not stop the others, so each is dispatched independently.
async fn notify(client: &reqwest::Client, rule: &Rule, n: &crate::notify::Notification) {
    for ch in &rule.channels {
        match crate::notify::dispatch(client, &ch.kind, &ch.config, n).await {
            Ok(()) => tracing::info!(rule = %rule.id, channel = %ch.name, "notified"),
            Err(e) => {
                tracing::warn!(error = %e, rule = %rule.id, channel = %ch.name, "notify failed")
            }
        }
    }
}

/// Human description of a rule's target: (name, "Service"|"Host", workspace).
async fn target_info(state: &AppState, rule: &Rule) -> (String, &'static str, String) {
    if let Some(mid) = rule.monitor_id {
        let r: Option<(String, String)> = sqlx::query_as(
            "SELECT m.name, n.name FROM monitors m JOIN workspaces n ON n.id = m.workspace_id WHERE m.id = $1",
        )
        .bind(mid)
        .fetch_optional(&state.config)
        .await
        .ok()
        .flatten();
        let (t, ws) = r.unwrap_or_else(|| ("service".into(), String::new()));
        return (t, "Service", ws);
    }
    if let Some(sid) = rule.system_id {
        let r: Option<(String, String)> = sqlx::query_as(
            "SELECT s.name, n.name FROM systems s JOIN workspaces n ON n.id = s.workspace_id WHERE s.id = $1",
        )
        .bind(sid)
        .fetch_optional(&state.config)
        .await
        .ok()
        .flatten();
        let (t, ws) = r.unwrap_or_else(|| ("host".into(), String::new()));
        return (t, "Host", ws);
    }
    let ws = match rule.scope_ns {
        Some(id) => sqlx::query_as::<_, (String,)>("SELECT name FROM workspaces WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.config)
            .await
            .ok()
            .flatten()
            .map(|(n,)| n)
            .unwrap_or_default(),
        None => String::new(),
    };
    match rule.scope_kind.as_deref() {
        Some("all_hosts") => ("All hosts".into(), "Host", ws),
        _ => ("All services".into(), "Service", ws),
    }
}

/// Human condition, e.g. "is DOWN" / "CPU % > 90" / "offline > 120s".
fn condition_text(rule: &Rule) -> String {
    let service_like =
        rule.monitor_id.is_some() || rule.scope_kind.as_deref() == Some("all_services");
    if service_like {
        return "is DOWN".into();
    }
    let c = &rule.condition;
    if let Some(secs) = c.get("offline_secs").and_then(Value::as_i64) {
        return format!("offline > {secs}s");
    }
    match (
        c.get("metric").and_then(Value::as_str),
        c.get("op").and_then(Value::as_str),
        c.get("value"),
    ) {
        (Some(m), Some(op), Some(v)) => format!("{m} {op} {v}"),
        _ => String::new(),
    }
}
