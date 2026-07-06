//! Per-channel send logic. [`dispatch`] routes a [`Notification`] to the right
//! transport by `kind`; each arm reads the channel's config fields and performs the
//! actual HTTP/SMTP call. Shared by the alert engine and the "send test" endpoint.

use serde_json::{json, Value};

use super::Notification;

fn s<'a>(cfg: &'a Value, k: &str) -> Option<&'a str> {
    cfg.get(k).and_then(Value::as_str).filter(|v| !v.is_empty())
}
fn sreq<'a>(cfg: &'a Value, kind: &str, k: &'a str) -> anyhow::Result<&'a str> {
    s(cfg, k).ok_or_else(|| anyhow::anyhow!("{kind} missing {k}"))
}
fn flag(cfg: &Value, k: &str) -> bool {
    cfg.get(k).and_then(Value::as_bool).unwrap_or(false)
}

async fn post_json(client: &reqwest::Client, url: &str, body: Value) -> anyhow::Result<()> {
    crate::net_guard::check_target(url)?;
    client
        .post(url)
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

/// Send a [`Notification`] through a channel. Shared by the alert engine and the
/// "send test" endpoint. Plain transports use `n.text()`; rich ones format natively.
pub async fn dispatch(
    client: &reqwest::Client,
    kind: &str,
    cfg: &Value,
    n: &Notification,
) -> anyhow::Result<()> {
    let text = n.text();
    let body = text.as_str();
    match kind {
        "webhook" => {
            let url = sreq(cfg, kind, "url")?;
            crate::net_guard::check_target(url)?;
            let method = s(cfg, "method").unwrap_or("POST").to_uppercase();
            let payload: Value = match s(cfg, "body") {
                Some(tpl) => {
                    let rendered = tpl
                        .replace("{{message}}", body)
                        .replace("{{title}}", &n.title())
                        .replace("{{status}}", n.status_word());
                    serde_json::from_str(&rendered).unwrap_or_else(|_| json!({ "text": body }))
                }
                // Default: a structured payload so downstream automation can branch on it.
                None => json!({
                    "event": if n.firing { "alert" } else { "recovery" },
                    "status": n.status_word(),
                    "firing": n.firing,
                    "title": n.title(),
                    "target": n.target,
                    "type": n.kind_label,
                    "workspace": n.workspace,
                    "condition": n.condition,
                    "detail": n.detail,
                    "at": n.at,
                    "text": body,
                }),
            };
            let m = reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::POST);
            let mut req = client.request(m, url).json(&payload);
            if let Some(hdrs) = s(cfg, "headers") {
                for line in hdrs.lines() {
                    if let Some((k, v)) = line.split_once(':') {
                        req = req.header(k.trim(), v.trim());
                    }
                }
            }
            req.send().await?.error_for_status()?;
        }
        "slack" | "mattermost" => {
            let url = sreq(cfg, kind, "url")?;
            post_json(
                client,
                url,
                json!({ "attachments": [{
                    "color": n.color_hex(),
                    "title": n.title(),
                    "fields": n.slack_fields(),
                    "footer": "Vantage",
                }] }),
            )
            .await?;
        }
        "teams" | "gchat" => {
            let url = sreq(cfg, kind, "url")?;
            post_json(client, url, json!({ "text": body })).await?;
        }
        "discord" => {
            let url = sreq(cfg, kind, "url")?;
            let mut payload = json!({ "embeds": [{
                "title": n.title(),
                "color": n.color_int(),
                "fields": n.discord_fields(),
                "footer": { "text": "Vantage" },
            }] });
            if let Some(u) = s(cfg, "username") {
                payload["username"] = json!(u);
            }
            // Posting into a thread is a query param on the webhook URL, not a body field.
            let url = match s(cfg, "thread_id") {
                Some(t) => {
                    let sep = if url.contains('?') { '&' } else { '?' };
                    format!("{url}{sep}thread_id={t}")
                }
                None => url.to_string(),
            };
            post_json(client, &url, payload).await?;
        }
        "telegram" => {
            let token = sreq(cfg, kind, "bot_token")?;
            let chat_id = sreq(cfg, kind, "chat_id")?;
            let mut payload = json!({ "chat_id": chat_id, "text": n.html(), "parse_mode": "HTML" });
            if let Some(t) = s(cfg, "thread_id") {
                payload["message_thread_id"] = json!(t);
            }
            if flag(cfg, "silent") {
                payload["disable_notification"] = json!(true);
            }
            let url = format!("https://api.telegram.org/bot{token}/sendMessage");
            post_json(client, &url, payload).await?;
        }
        "matrix" => {
            let hs = sreq(cfg, kind, "homeserver")?.trim_end_matches('/');
            crate::net_guard::check_target(hs)?;
            let room = sreq(cfg, kind, "room_id")?;
            let token = sreq(cfg, kind, "token")?;
            let txn = uuid::Uuid::new_v4();
            let url = format!(
                "{hs}/_matrix/client/v3/rooms/{room}/send/m.room.message/{txn}?access_token={token}"
            );
            client
                .put(url)
                .json(&json!({
                    "msgtype": "m.text",
                    "body": body,
                    "format": "org.matrix.custom.html",
                    "formatted_body": n.html(),
                }))
                .send()
                .await?
                .error_for_status()?;
        }
        "ntfy" => {
            let server = s(cfg, "server")
                .unwrap_or("https://ntfy.sh")
                .trim_end_matches('/');
            crate::net_guard::check_target(server)?;
            let topic = sreq(cfg, kind, "topic")?;
            let url = format!("{server}/{topic}");
            let mut req = client.post(url).body(body.to_string());
            if let Some(prio) = s(cfg, "priority") {
                req = req.header("X-Priority", prio);
            }
            if let Some(tok) = s(cfg, "token") {
                req = req.bearer_auth(tok);
            }
            req.send().await?.error_for_status()?;
        }
        "pushover" => {
            let token = sreq(cfg, kind, "token")?;
            let user = sreq(cfg, kind, "user")?;
            let mut form = vec![("token", token), ("user", user), ("message", body)];
            if let Some(p) = s(cfg, "priority") {
                form.push(("priority", p));
            }
            client
                .post("https://api.pushover.net/1/messages.json")
                .form(&form)
                .send()
                .await?
                .error_for_status()?;
        }
        "gotify" => {
            let server = sreq(cfg, kind, "server")?.trim_end_matches('/');
            let token = sreq(cfg, kind, "token")?;
            let url = format!("{server}/message?token={token}");
            let prio: i64 = s(cfg, "priority").and_then(|p| p.parse().ok()).unwrap_or(5);
            post_json(client, &url, json!({ "message": body, "priority": prio })).await?;
        }
        "bark" => {
            let server = s(cfg, "server")
                .unwrap_or("https://api.day.app")
                .trim_end_matches('/');
            let key = sreq(cfg, kind, "device_key")?;
            post_json(
                client,
                &format!("{server}/push"),
                json!({ "device_key": key, "title": "Vantage", "body": body }),
            )
            .await?;
        }
        "pagerduty" => {
            let routing_key = sreq(cfg, kind, "routing_key")?;
            let severity = s(cfg, "severity").unwrap_or("error");
            post_json(
                client,
                "https://events.pagerduty.com/v2/enqueue",
                json!({
                    "routing_key": routing_key,
                    "event_action": "trigger",
                    "payload": { "summary": body, "source": "vantage", "severity": severity },
                }),
            )
            .await?;
        }
        "opsgenie" => {
            let api_key = sreq(cfg, kind, "api_key")?;
            let base = match s(cfg, "region") {
                Some("eu") => "https://api.eu.opsgenie.com",
                _ => "https://api.opsgenie.com",
            };
            client
                .post(format!("{base}/v2/alerts"))
                .header("Authorization", format!("GenieKey {api_key}"))
                .json(&json!({ "message": body }))
                .send()
                .await?
                .error_for_status()?;
        }
        "twilio" => {
            let sid = sreq(cfg, kind, "account_sid")?;
            let token = sreq(cfg, kind, "auth_token")?;
            let from = sreq(cfg, kind, "from")?;
            let to = sreq(cfg, kind, "to")?;
            let url = format!("https://api.twilio.com/2010-04-01/Accounts/{sid}/Messages.json");
            client
                .post(url)
                .basic_auth(sid, Some(token))
                .form(&[("From", from), ("To", to), ("Body", body)])
                .send()
                .await?
                .error_for_status()?;
        }
        "apprise" => {
            let server = sreq(cfg, kind, "server")?.trim_end_matches('/');
            let urls = sreq(cfg, kind, "urls")?
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(",");
            post_json(
                client,
                &format!("{server}/notify"),
                json!({ "urls": urls, "body": body, "title": "Vantage" }),
            )
            .await?;
        }
        "email" => send_email(cfg, n).await?,
        other => anyhow::bail!("unsupported channel kind: {other}"),
    }
    Ok(())
}

async fn send_email(cfg: &Value, n: &Notification) -> anyhow::Result<()> {
    use lettre::message::header::ContentType;
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

    let host = sreq(cfg, "email", "host")?;
    let port: u16 = s(cfg, "port").and_then(|p| p.parse().ok()).unwrap_or(587);
    // SSRF guard: the SMTP host is editor-supplied — block loopback/link-local/internal
    // targets just like the HTTP transports, so a channel can't port-scan the network.
    crate::net_guard::check_host(host, port)?;
    let from = sreq(cfg, "email", "from")?;
    let to = sreq(cfg, "email", "to")?;

    let email = Message::builder()
        .from(from.parse()?)
        .to(to.parse()?)
        .subject(n.title())
        .header(ContentType::TEXT_HTML)
        .body(n.email_html())?;

    let mut builder = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)?.port(port);
    if let (Some(u), Some(pw)) = (s(cfg, "username"), s(cfg, "password")) {
        builder = builder.credentials(Credentials::new(u.to_string(), pw.to_string()));
    }
    builder.build().send(email).await?;
    Ok(())
}
