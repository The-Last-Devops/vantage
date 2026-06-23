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

struct Rule {
    id: Uuid,
    monitor_id: Option<Uuid>,
    system_id: Option<Uuid>,
    condition: Value,
    cooldown_secs: i32,
    channel_kind: String,
    channel_config: Value,
    channel_name: String,
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
        let cooldown_passed = last_notified
            .map(|t| (now - t).num_seconds() >= rule.cooldown_secs as i64)
            .unwrap_or(true);

        let (should_notify, subject) = match (was_firing, eval.firing) {
            (false, true) => (true, "🔴 ALERT"),
            (true, true) if cooldown_passed => (true, "🔴 ALERT (still firing)"),
            (true, false) => (true, "✅ RECOVERED"),
            _ => (false, ""),
        };

        if should_notify {
            let body = format!("{subject}: {}", eval.message);
            match notify(client, &rule, &body).await {
                Ok(()) => {
                    tracing::info!(rule = %rule.id, channel = %rule.channel_name, "{subject}")
                }
                Err(e) => tracing::warn!(error = %e, rule = %rule.id, "notify failed"),
            }
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

        // Persist state. last_notified advances only when we actually notified.
        if eval.firing != was_firing || should_notify {
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
    }
    Ok(())
}

async fn load_rules(state: &AppState) -> anyhow::Result<Vec<Rule>> {
    let rows: Vec<(
        Uuid,
        Option<Uuid>,
        Option<Uuid>,
        Json<Value>,
        i32,
        String,
        Json<Value>,
        String,
    )> = sqlx::query_as(
        "SELECT r.id, r.monitor_id, r.system_id, r.condition, r.cooldown_secs, \
                c.kind, c.config, c.name \
         FROM alerts r JOIN channels c ON c.id = r.channel_id \
         WHERE r.enabled = true",
    )
    .fetch_all(&state.config)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                monitor_id,
                system_id,
                condition,
                cooldown_secs,
                channel_kind,
                channel_config,
                channel_name,
            )| {
                Rule {
                    id,
                    monitor_id,
                    system_id,
                    condition: condition.0,
                    cooldown_secs,
                    channel_kind,
                    channel_config: channel_config.0,
                    channel_name,
                }
            },
        )
        .collect())
}

/// Returns None when there's no data yet to judge (avoids false alerts on cold start).
async fn evaluate(state: &AppState, rule: &Rule) -> anyhow::Result<Option<Eval>> {
    if let Some(mid) = rule.monitor_id {
        return evaluate_monitor(state, mid).await;
    }
    if let Some(sid) = rule.system_id {
        return evaluate_server(state, sid, &rule.condition).await;
    }
    Ok(None)
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

async fn notify(client: &reqwest::Client, rule: &Rule, body: &str) -> anyhow::Result<()> {
    dispatch(client, &rule.channel_kind, &rule.channel_config, body).await
}

/// Send `body` through a notification channel. Shared by the alert engine and the
/// "send test" endpoint. Adding a channel type is one match arm.
pub async fn dispatch(
    client: &reqwest::Client,
    kind: &str,
    config: &Value,
    body: &str,
) -> anyhow::Result<()> {
    let str_field = |k: &str| config.get(k).and_then(Value::as_str).map(str::to_owned);
    match kind {
        "webhook" => {
            let url = str_field("url").ok_or_else(|| anyhow::anyhow!("webhook missing url"))?;
            client
                .post(url)
                .json(&serde_json::json!({ "text": body }))
                .send()
                .await?
                .error_for_status()?;
        }
        // Slack incoming webhook expects {"text"}; Discord expects {"content"}.
        "slack" => {
            let url = str_field("url").ok_or_else(|| anyhow::anyhow!("slack missing url"))?;
            client
                .post(url)
                .json(&serde_json::json!({ "text": body }))
                .send()
                .await?
                .error_for_status()?;
        }
        "discord" => {
            let url = str_field("url").ok_or_else(|| anyhow::anyhow!("discord missing url"))?;
            client
                .post(url)
                .json(&serde_json::json!({ "content": body }))
                .send()
                .await?
                .error_for_status()?;
        }
        "telegram" => {
            let token = str_field("bot_token")
                .ok_or_else(|| anyhow::anyhow!("telegram missing bot_token"))?;
            let chat_id =
                str_field("chat_id").ok_or_else(|| anyhow::anyhow!("telegram missing chat_id"))?;
            let url = format!("https://api.telegram.org/bot{token}/sendMessage");
            client
                .post(url)
                .json(&serde_json::json!({ "chat_id": chat_id, "text": body }))
                .send()
                .await?
                .error_for_status()?;
        }
        other => anyhow::bail!("unsupported channel kind: {other}"),
    }
    Ok(())
}
