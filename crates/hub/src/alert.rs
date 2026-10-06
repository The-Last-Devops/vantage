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
    /// Short per-target label for the workspace-wide roll-up, e.g. "k8s14 (CPU 94%)".
    brief: String,
    /// Targets currently breaching (names). Empty when ok. The notification headline
    /// names them, instead of the rule's scope ("All hosts"), which read as "every host
    /// is down" when one host had a CPU spike.
    affected: Vec<String>,
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
            let (target, kind_label, workspace, endpoint) = target_info(state, &rule).await;
            let target = headline_target(target, &rule, &eval);
            let n = crate::notify::Notification {
                firing: eval.firing,
                repeat: was_firing && eval.firing, // a re-notify while still firing
                threshold: is_threshold(&rule),
                target,
                kind_label: kind_label.to_string(),
                workspace,
                condition: condition_text(&rule),
                endpoint,
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

/// Build the notification a rule WOULD send, for the "Test rule" button.
///
/// Deliberately goes through the same `target_info` + `condition_text` the live engine
/// uses, so a test is not a lookalike — it is the real payload for that rule, with the
/// only difference in `detail`, which says plainly that nothing is wrong. `firing`
/// picks the DOWN or the UP shape; the UI sends both so people can see each one.
pub(crate) async fn test_notification(
    state: &AppState,
    alert_id: Uuid,
    firing: bool,
) -> anyhow::Result<crate::notify::Notification> {
    type Row = (
        Option<Uuid>,
        Option<Uuid>,
        Option<String>,
        Option<Uuid>,
        Json<Value>,
    );
    let (monitor_id, system_id, scope_kind, scope_ns, condition): Row = sqlx::query_as(
        "SELECT monitor_id, system_id, scope_kind, scope_workspace_id, condition \
         FROM alerts WHERE id = $1",
    )
    .bind(alert_id)
    .fetch_one(&state.config)
    .await?;
    let rule = Rule {
        id: alert_id,
        monitor_id,
        system_id,
        scope_kind,
        scope_ns,
        condition: condition.0,
        renotify_secs: None,
        channels: Vec::new(),
    };
    let (target, kind_label, workspace, endpoint) = target_info(state, &rule).await;
    Ok(crate::notify::Notification {
        firing,
        repeat: false,
        threshold: is_threshold(&rule),
        target,
        kind_label: kind_label.to_string(),
        workspace,
        condition: condition_text(&rule),
        endpoint,
        detail: if firing {
            "Test alert — this is what a real DOWN notification for this rule looks like. Nothing is wrong.".into()
        } else {
            "Test alert — this is what the matching recovery looks like. Nothing is wrong.".into()
        },
        at: chrono::Utc::now().format("%Y-%m-%d %H:%M UTC").to_string(),
    })
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
    let mut briefs: Vec<String> = Vec::new();
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
                briefs.push(if e.brief.is_empty() {
                    name.clone()
                } else {
                    e.brief
                });
            }
        }
    }
    if !any_data {
        return Ok(None);
    }
    let firing = !down.is_empty();
    let message = if firing {
        format!(
            "{} of {} {label}: {}",
            down.len(),
            targets.len(),
            briefs.join(", ")
        )
    } else {
        format!("all {} {label} ok", targets.len())
    };
    Ok(Some(Eval {
        firing,
        message,
        brief: String::new(),
        affected: down,
    }))
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
        brief: String::new(),
        affected: Vec::new(),
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
            brief: String::new(),
            affected: if firing { vec![name] } else { Vec::new() },
        }));
    }

    // Metric threshold: {"metric":"cpu_percent","op":">","value":90}
    let metric = cond.get("metric").and_then(Value::as_str);
    let op = cond.get("op").and_then(Value::as_str);
    let threshold = cond.get("value").and_then(Value::as_f64);
    if let (Some(metric), Some(op), Some(threshold)) = (metric, op, threshold) {
        let for_secs = cond.get("for_secs").and_then(Value::as_i64).unwrap_or(0);
        // Core count lives in the config DB (the host row); the samples in the data DB.
        // The two are never joined — fetched separately and combined here.
        let cores: Option<(Option<i32>,)> =
            sqlx::query_as("SELECT cpu_cores FROM systems WHERE id = $1")
                .bind(system_id)
                .fetch_optional(&state.config)
                .await?;
        let cores = cores.and_then(|(c,)| c).map(|c| c as i64);
        // Newest first. With a sustain window, every sample inside it; otherwise just
        // the latest one. The window query is bounded by the rule's own duration, so a
        // 15-minute rule on the 5-second tier reads at most 180 rows.
        type Row = (
            chrono::DateTime<chrono::Utc>,
            f64,
            f64,
            f64,
            f64,
            i64,
            i64,
            i64,
            i64,
        );
        let rows: Vec<Row> = if for_secs > 0 {
            sqlx::query_as(
                "SELECT time, cpu_percent, load1, COALESCE(load5, 0), COALESCE(load15, 0), \
                        mem_used, mem_total, disk_used, disk_total \
                 FROM system_metrics_5s WHERE system_id = $1 AND time > now() - ($2 * interval '1 second') \
                 ORDER BY time DESC",
            )
            .bind(system_id)
            .bind(for_secs as f64)
            .fetch_all(&state.data)
            .await?
        } else {
            sqlx::query_as(
                "SELECT time, cpu_percent, load1, COALESCE(load5, 0), COALESCE(load15, 0), \
                        mem_used, mem_total, disk_used, disk_total \
                 FROM system_metrics_5s WHERE system_id = $1 ORDER BY time DESC LIMIT 1",
            )
            .bind(system_id)
            .fetch_all(&state.data)
            .await?
        };
        if rows.is_empty() {
            return Ok(None);
        }
        let mut values = Vec::with_capacity(rows.len());
        for (_, cpu, load1, load5, load15, mem_used, mem_total, disk_used, disk_total) in &rows {
            let sample = Sample {
                cpu: *cpu,
                load1: *load1,
                load5: *load5,
                load15: *load15,
                mem_used: *mem_used,
                mem_total: *mem_total,
                disk_used: *disk_used,
                disk_total: *disk_total,
                cores,
            };
            // Unknown metric: unreachable for a rule created through the API, which
            // rejects those, but a hand-edited row must not fire on a value it never read.
            let Some(v) = sample.value(metric) else {
                return Ok(None);
            };
            values.push(v);
        }
        let current = values[0];
        let oldest_age = (chrono::Utc::now() - rows[rows.len() - 1].0).num_seconds();
        let firing = sustained(&values, op, threshold, for_secs, oldest_age);
        let label = metric_label(metric);
        let shown = if metric.ends_with("_percent") {
            format!("{label} {current:.0}%")
        } else {
            format!("{label} {current:.2}")
        };
        return Ok(Some(Eval {
            firing,
            message: format!(
                "server '{name}' {metric}={current:.1} {op} {threshold}{} -> {}",
                if for_secs > 0 {
                    format!(" for {}", for_text(for_secs))
                } else {
                    String::new()
                },
                if firing { "BREACH" } else { "ok" }
            ),
            brief: format!("{name} ({shown})"),
            affected: if firing { vec![name] } else { Vec::new() },
        }));
    }

    Ok(None)
}

/// The latest host sample a threshold rule is evaluated against.
pub struct Sample {
    pub cpu: f64,
    pub load1: f64,
    pub load5: f64,
    pub load15: f64,
    pub mem_used: i64,
    pub mem_total: i64,
    pub disk_used: i64,
    pub disk_total: i64,
    /// Logical cores, from the host row; None until the agent has reported them.
    pub cores: Option<i64>,
}

impl Sample {
    fn pct(used: i64, total: i64) -> f64 {
        if total > 0 {
            used as f64 / total as f64 * 100.0
        } else {
            0.0
        }
    }

    /// Value for a metric name, or `None` when the engine cannot evaluate it.
    ///
    /// Pure, so the arms can be tested — and so `HOST_METRICS` can be checked against it
    /// directly. Those two drifting apart is exactly how a disk rule became creatable and
    /// unfireable at the same time: the allowlist is what the API promises, this is what
    /// the engine delivers, and nothing was comparing them.
    pub fn value(&self, metric: &str) -> Option<f64> {
        Some(match metric {
            "cpu_percent" => self.cpu,
            "load1" => self.load1,
            "load5" => self.load5,
            "load15" => self.load15,
            // Load is only comparable across hosts relative to core count: load 8 is a
            // saturated 4-core box and an idle 32-core one. None (not 0) when the host
            // has not reported its cores — a rule must not fire on a made-up number.
            "load_per_core" => match self.cores {
                Some(c) if c > 0 => self.load1 / c as f64,
                _ => return None,
            },
            "mem_percent" => Self::pct(self.mem_used, self.mem_total),
            // A filesystem that fills takes the machine down with it and, unlike CPU or
            // load, never recovers on its own. The agent has always collected these
            // numbers; until 3.5.0 nothing read them.
            "disk_percent" => Self::pct(self.disk_used, self.disk_total),
            _ => return None,
        })
    }
}

/// Host metrics a threshold rule may be written against. The API validates against this
/// list, so a typo is a 400 instead of a rule that renders perfectly and never fires —
/// which is how a disk alert could previously be "configured" and do nothing at all.
pub const HOST_METRICS: &[&str] = &[
    "cpu_percent",
    "mem_percent",
    "disk_percent",
    "load1",
    "load5",
    "load15",
    "load_per_core",
];

/// Sustain windows a rule may ask for (seconds). 0 = fire on the latest sample.
/// Bounded so a rule cannot ask the engine to scan hours of the 5-second tier each tick.
pub const FOR_SECS_MAX: i64 = 3600;

/// Whether a breach counts as firing. `values` is newest first, every sample inside the
/// rule's window (or just the latest one when `for_secs` is 0).
///
/// With a window, EVERY sample must breach — one dip resets the clock, which is the
/// Prometheus `for:` semantic and what stops a single CPU spike from paging. The window
/// must also be reasonably covered: a host that came online 20 seconds ago has three
/// samples, all breaching, and a 5-minute rule must not fire on those. Three quarters of
/// the window is the bar, which forgives an agent on a slow push interval.
fn sustained(values: &[f64], op: &str, threshold: f64, for_secs: i64, oldest_age: i64) -> bool {
    if values.is_empty() {
        return false;
    }
    if for_secs <= 0 {
        return compare(values[0], op, threshold);
    }
    if oldest_age * 4 < for_secs * 3 {
        return false;
    }
    values.iter().all(|v| compare(*v, op, threshold))
}

/// "5 min" / "90 s" for the human condition line.
fn for_text(secs: i64) -> String {
    if secs % 60 == 0 {
        format!("{} min", secs / 60)
    } else {
        format!("{secs} s")
    }
}

/// Human metric name, matching the editor's dropdown so a notification reads the same
/// as the rule it came from.
fn metric_label(metric: &str) -> &'static str {
    match metric {
        "cpu_percent" => "CPU %",
        "mem_percent" => "Memory %",
        "disk_percent" => "Disk %",
        "load1" => "Load 1m",
        "load5" => "Load 5m",
        "load15" => "Load 15m",
        "load_per_core" => "Load / core",
        _ => "metric",
    }
}

/// A rule that compares a metric (as opposed to down/up or offline). These get the
/// ALERT / RECOVERED vocabulary in notifications instead of DOWN / UP.
fn is_threshold(rule: &Rule) -> bool {
    rule.monitor_id.is_none()
        && rule.scope_kind.as_deref() != Some("all_services")
        && rule.condition.get("metric").is_some()
}

/// What the headline names. A single-target rule names its target. A workspace-wide
/// rule used to say "All hosts", which with "DOWN" next to it read as an outage of every
/// host; it now names the breaching host, or counts them. On recovery there is nothing
/// breaching to name, so the scope label stays (paired with RECOVERED, it is unambiguous).
fn headline_target(scope_name: String, rule: &Rule, eval: &Eval) -> String {
    if rule.scope_kind.is_none() || !eval.firing {
        return scope_name;
    }
    match eval.affected.len() {
        0 => scope_name,
        1 => eval.affected[0].clone(),
        n => format!(
            "{n} {}",
            if rule.scope_kind.as_deref() == Some("all_hosts") {
                "hosts"
            } else {
                "services"
            }
        ),
    }
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
/// (display name, kind label, workspace, probed endpoint). The endpoint is what the
/// on-call dev actually needs — the URL/host being checked — and is empty for host
/// rules and all-services rules, which have no single target.
async fn target_info(state: &AppState, rule: &Rule) -> (String, &'static str, String, String) {
    if let Some(mid) = rule.monitor_id {
        let r: Option<(String, String, String)> = sqlx::query_as(
            "SELECT m.name, n.name, COALESCE(m.target, '') FROM monitors m \
             JOIN workspaces n ON n.id = m.workspace_id WHERE m.id = $1",
        )
        .bind(mid)
        .fetch_optional(&state.config)
        .await
        .ok()
        .flatten();
        let (t, ws, endpoint) =
            r.unwrap_or_else(|| ("service".into(), String::new(), String::new()));
        // A target can be a connection string with a password in it.
        return (t, "Service", ws, crate::notify::redact_endpoint(&endpoint));
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
        return (t, "Host", ws, String::new());
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
        Some("all_hosts") => ("All hosts".into(), "Host", ws, String::new()),
        _ => ("All services".into(), "Service", ws, String::new()),
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
        (Some(m), Some(op), Some(v)) => {
            let mut t = format!("{} {op} {v}", metric_label(m));
            if let Some(f) = c.get("for_secs").and_then(Value::as_i64).filter(|f| *f > 0) {
                t.push_str(&format!(" for {}", for_text(f)));
            }
            t
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod metric_tests {
    use super::*;

    fn sample() -> Sample {
        Sample {
            cpu: 12.5,
            load1: 3.0,
            load5: 2.0,
            load15: 1.0,
            mem_used: 3,
            mem_total: 4,
            disk_used: 9,
            disk_total: 10,
            cores: Some(4),
        }
    }

    #[test]
    fn load_per_core_divides_by_reported_cores() {
        assert_eq!(sample().value("load_per_core"), Some(0.75));
        assert_eq!(sample().value("load5"), Some(2.0));
        assert_eq!(sample().value("load15"), Some(1.0));
    }

    /// No core count (agent too old, or first report pending) must mean "cannot
    /// evaluate", never "divide by something and fire".
    #[test]
    fn load_per_core_without_cores_is_not_evaluated() {
        let s = Sample {
            cores: None,
            ..sample()
        };
        assert_eq!(s.value("load_per_core"), None);
        let z = Sample {
            cores: Some(0),
            ..sample()
        };
        assert_eq!(z.value("load_per_core"), None);
    }

    /// The sustain window: every sample must breach, and the window must be mostly
    /// covered. These two rules are what turn a spike into silence and a real plateau
    /// into one alert.
    #[test]
    fn sustained_needs_every_sample_over_the_whole_window() {
        // no window: latest sample decides
        assert!(sustained(&[95.0], ">", 90.0, 0, 0));
        assert!(!sustained(&[80.0, 95.0], ">", 90.0, 0, 0));
        // window fully covered, all breaching
        assert!(sustained(&[95.0, 92.0, 91.0], ">", 90.0, 300, 300));
        // one dip inside the window resets it
        assert!(!sustained(&[95.0, 85.0, 91.0], ">", 90.0, 300, 300));
        // all breaching but the host only has 60s of samples for a 300s rule
        assert!(!sustained(&[95.0, 92.0], ">", 90.0, 300, 60));
        // three quarters covered is enough (slow push interval)
        assert!(sustained(&[95.0, 92.0], ">", 90.0, 300, 225));
        assert!(!sustained(&[], ">", 90.0, 300, 300));
    }

    #[test]
    fn condition_text_names_the_window() {
        let rule = |c: Value| Rule {
            id: Uuid::nil(),
            monitor_id: None,
            system_id: None,
            scope_kind: Some("all_hosts".into()),
            scope_ns: None,
            condition: c,
            renotify_secs: None,
            channels: Vec::new(),
        };
        assert_eq!(
            condition_text(&rule(
                serde_json::json!({"metric":"cpu_percent","op":">","value":90,"for_secs":300})
            )),
            "CPU % > 90 for 5 min"
        );
        assert_eq!(
            condition_text(&rule(
                serde_json::json!({"metric":"load_per_core","op":">","value":1.5})
            )),
            "Load / core > 1.5"
        );
        assert!(is_threshold(&rule(
            serde_json::json!({"metric":"cpu_percent"})
        )));
        assert!(!is_threshold(&rule(
            serde_json::json!({"offline_secs":120})
        )));
    }

    /// The headline is what people read in Discord. "All hosts — DOWN" for one hot host
    /// was read as a fleet outage; it must name the host.
    #[test]
    fn headline_names_the_breaching_host_not_the_scope() {
        let rule = Rule {
            id: Uuid::nil(),
            monitor_id: None,
            system_id: None,
            scope_kind: Some("all_hosts".into()),
            scope_ns: None,
            condition: serde_json::json!({"metric":"cpu_percent","op":">","value":90}),
            renotify_secs: None,
            channels: Vec::new(),
        };
        let ev = |affected: Vec<&str>| Eval {
            firing: !affected.is_empty(),
            message: String::new(),
            brief: String::new(),
            affected: affected.into_iter().map(String::from).collect(),
        };
        assert_eq!(
            headline_target("All hosts".into(), &rule, &ev(vec!["k8s14"])),
            "k8s14"
        );
        assert_eq!(
            headline_target("All hosts".into(), &rule, &ev(vec!["a", "b", "c"])),
            "3 hosts"
        );
        assert_eq!(
            headline_target("All hosts".into(), &rule, &ev(vec![])),
            "All hosts"
        );
    }

    /// The allowlist the API validates against must be exactly what the engine can
    /// evaluate. A name in one and not the other is a rule that either cannot be created
    /// or cannot fire — and the second kind is silent.
    #[test]
    fn every_allowed_metric_is_evaluable() {
        for m in HOST_METRICS {
            assert!(
                sample().value(m).is_some(),
                "{m} is allowed but not evaluated"
            );
        }
    }

    #[test]
    fn unknown_metric_is_not_evaluated() {
        assert!(sample().value("disk_usage").is_none());
        assert!(sample().value("").is_none());
    }

    #[test]
    fn percentages_are_computed_from_used_over_total() {
        assert_eq!(sample().value("disk_percent"), Some(90.0));
        assert_eq!(sample().value("mem_percent"), Some(75.0));
        assert_eq!(sample().value("cpu_percent"), Some(12.5));
    }

    /// A host that reports a zero total (no disk reported yet) must read 0, not NaN —
    /// NaN compares false against every threshold, so the rule would never fire and
    /// never say why.
    #[test]
    fn zero_total_is_zero_not_nan() {
        let s = Sample {
            disk_total: 0,
            mem_total: 0,
            ..sample()
        };
        assert_eq!(s.value("disk_percent"), Some(0.0));
        assert_eq!(s.value("mem_percent"), Some(0.0));
    }
}
