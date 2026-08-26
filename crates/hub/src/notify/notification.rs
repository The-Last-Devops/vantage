//! The structured alert payload and its per-transport renderings (plain text,
//! Telegram HTML, Slack/Discord field lists, email HTML). Built by the alert
//! engine; [`Notification::test`] powers the "send test" button.

use serde_json::{json, Value};

/// A structured alert payload. Plain transports get a tidy multi-line [`Notification::text`];
/// rich ones (Discord embed, Slack attachment, Telegram HTML, webhook JSON, email
/// HTML) format it natively.
pub struct Notification {
    /// true = a problem fired; false = recovered.
    pub firing: bool,
    /// true = a re-notification while still firing (not the first alert).
    pub repeat: bool,
    pub target: String,     // "api.shop" or "All services"
    pub kind_label: String, // "Service" / "Host"
    pub workspace: String,
    pub condition: String, // "is DOWN" / "CPU % > 90" / "offline > 120s"
    /// What was actually probed — the monitor's target ("https://api.shop/health",
    /// "db.internal:5432"). Empty for host rules, push monitors and all-services
    /// rules. **Always pass it through [`redact_endpoint`]** — a target can be a
    /// connection string with a password in it.
    pub endpoint: String,
    pub detail: String, // the probe / evaluation message
    pub at: String,     // formatted UTC timestamp
}

impl Notification {
    /// A friendly payload for the "send test" button.
    pub fn test() -> Self {
        Notification {
            firing: false,
            repeat: false,
            target: "Test notification".into(),
            kind_label: String::new(),
            workspace: String::new(),
            condition: String::new(),
            endpoint: String::new(),
            detail: "Your channel is wired up correctly — real alerts will arrive here.".into(),
            at: now_utc(),
        }
    }
    /// The word in the headline and in `{{status}}` / the webhook `status` field.
    /// Deliberately UP / DOWN / STILL DOWN — the same vocabulary the UI uses for a
    /// host or service, so a phone notification reads at a glance. (A threshold rule
    /// like "CPU % > 90" also renders DOWN; the `Condition` field carries the nuance.)
    pub(crate) fn status_word(&self) -> &'static str {
        if !self.firing {
            "UP"
        } else if self.repeat {
            "STILL DOWN"
        } else {
            "DOWN"
        }
    }
    fn emoji(&self) -> &'static str {
        if self.firing {
            "🔴"
        } else {
            "✅"
        }
    }
    /// Headline, e.g. "🔴 api.shop — ALERT".
    pub fn title(&self) -> String {
        format!("{} {} — {}", self.emoji(), self.target, self.status_word())
    }
    /// (label, value) rows the rich providers render; empty values are skipped.
    fn fields(&self) -> Vec<(&'static str, &str)> {
        let mut v: Vec<(&'static str, &str)> = Vec::new();
        if !self.kind_label.is_empty() {
            v.push(("Type", &self.kind_label));
        }
        if !self.workspace.is_empty() {
            v.push(("Workspace", &self.workspace));
        }
        if !self.endpoint.is_empty() {
            // "URL" for something a dev can click, "Target" for host:port and the like.
            let label =
                if self.endpoint.starts_with("http://") || self.endpoint.starts_with("https://") {
                    "URL"
                } else {
                    "Target"
                };
            v.push((label, &self.endpoint));
        }
        if !self.condition.is_empty() {
            v.push(("Condition", &self.condition));
        }
        if !self.detail.is_empty() {
            v.push(("Detail", &self.detail));
        }
        v.push(("When", &self.at));
        v
    }
    /// Tidy multi-line plain text — the default for transports without rich markup.
    pub fn text(&self) -> String {
        let mut s = self.title();
        for (k, val) in self.fields() {
            s.push_str(&format!("\n{k}: {val}"));
        }
        s.push_str("\n— Vantage");
        s
    }
    /// Telegram HTML body.
    pub(crate) fn html(&self) -> String {
        let esc = |t: &str| {
            t.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        };
        let mut s = format!("<b>{}</b>", esc(&self.title()));
        for (k, val) in self.fields() {
            s.push_str(&format!("\n<b>{k}:</b> {}", esc(val)));
        }
        s.push_str("\n<i>Vantage</i>");
        s
    }
    pub(crate) fn color_int(&self) -> u32 {
        if self.firing {
            0xE0_1E_5A
        } else {
            0x2E_B6_7D
        }
    }
    pub(crate) fn color_hex(&self) -> &'static str {
        if self.firing {
            "#E01E5A"
        } else {
            "#2EB67D"
        }
    }
    pub(crate) fn slack_fields(&self) -> Vec<Value> {
        self.fields()
            .iter()
            .map(|(k, v)| json!({ "title": k, "value": v, "short": true }))
            .collect()
    }
    pub(crate) fn discord_fields(&self) -> Vec<Value> {
        self.fields()
            .iter()
            .map(|(k, v)| json!({ "name": k, "value": v, "inline": true }))
            .collect()
    }
    pub(crate) fn email_html(&self) -> String {
        let esc = |t: &str| {
            t.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        };
        let rows: String = self
            .fields()
            .iter()
            .map(|(k, v)| {
                format!(
                    "<tr><td style=\"color:#64748b;padding:3px 16px 3px 0;white-space:nowrap;vertical-align:top\">{k}</td><td style=\"color:#0f172a\">{}</td></tr>",
                    esc(v)
                )
            })
            .collect();
        format!(
            "<div style=\"font-family:system-ui,Segoe UI,Roboto,sans-serif;font-size:14px\">\
             <div style=\"font-size:17px;font-weight:700;color:{};margin-bottom:10px\">{}</div>\
             <table style=\"border-collapse:collapse\">{rows}</table>\
             <p style=\"color:#94a3b8;font-size:12px;margin-top:14px\">Vantage</p></div>",
            self.color_hex(),
            esc(&self.title())
        )
    }
}

/// Strip `user:password@` from a URL-ish target before it leaves the building.
///
/// A monitor's target is not just an http URL — `postgres`, `mysql`, `mongodb`,
/// `redis` and `rabbitmq` monitors all take a connection string, and those routinely
/// embed a password. Notifications go to Discord/Slack/Telegram, i.e. somewhere far
/// less protected than the hub, so the credential is masked rather than sent.
/// Anything without a `//host` part is returned unchanged.
pub fn redact_endpoint(target: &str) -> String {
    let Some(scheme_end) = target.find("://") else {
        return target.to_string();
    };
    let (scheme, rest) = target.split_at(scheme_end + 3);
    // userinfo, if any, runs to the LAST '@' before the first '/' of the path.
    let host_part_end = rest.find('/').unwrap_or(rest.len());
    let Some(at) = rest[..host_part_end].rfind('@') else {
        return target.to_string();
    };
    let userinfo = &rest[..at];
    let after = &rest[at + 1..];
    // Keep the username (useful for debugging), drop anything after the first ':'.
    match userinfo.split_once(':') {
        Some((user, _pw)) => format!("{scheme}{user}:***@{after}"),
        None => format!("{scheme}{userinfo}@{after}"),
    }
}

fn now_utc() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M UTC").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(firing: bool, repeat: bool) -> Notification {
        Notification {
            firing,
            repeat,
            target: "api.shop".into(),
            kind_label: "Service".into(),
            workspace: "production".into(),
            condition: "is DOWN".into(),
            endpoint: "https://api.shop/health".into(),
            detail: "connection refused".into(),
            at: "2026-08-26 03:14 UTC".into(),
        }
    }

    /// The three words people actually read on their phone. Changing them changes every
    /// transport at once (headline, `{{status}}` in custom webhook templates, and the
    /// webhook JSON `status` field), so pin them.
    #[test]
    fn status_words_are_up_down_still_down() {
        assert_eq!(n(false, false).status_word(), "UP");
        assert_eq!(n(true, false).status_word(), "DOWN");
        assert_eq!(n(true, true).status_word(), "STILL DOWN");
        // repeat only matters while firing — a recovery is never "STILL DOWN".
        assert_eq!(n(false, true).status_word(), "UP");
    }

    #[test]
    fn title_pairs_the_word_with_an_emoji() {
        assert_eq!(n(true, false).title(), "🔴 api.shop — DOWN");
        assert_eq!(n(true, true).title(), "🔴 api.shop — STILL DOWN");
        assert_eq!(n(false, false).title(), "✅ api.shop — UP");
    }

    #[test]
    fn endpoint_is_shown_so_a_dev_can_click_it() {
        let t = n(true, false).text();
        assert!(t.contains("\nURL: https://api.shop/health"), "{t}");
        // host:port targets are not URLs — label them honestly
        let mut m = n(true, false);
        m.endpoint = "db.internal:5432".into();
        assert!(
            m.text().contains("\nTarget: db.internal:5432"),
            "{}",
            m.text()
        );
        // and an absent endpoint (host rule, push monitor) adds no row at all
        let mut e = n(true, false);
        e.endpoint = String::new();
        assert!(!e.text().contains("URL:") && !e.text().contains("Target:"));
    }

    /// Connection-string targets carry passwords; notifications land in Discord/Slack.
    #[test]
    fn endpoint_password_never_leaves_the_hub() {
        assert_eq!(
            redact_endpoint("postgres://app:s3cr3t@db.internal:5432/shop"),
            "postgres://app:***@db.internal:5432/shop"
        );
        assert_eq!(
            redact_endpoint("https://admin:hunter2@api.shop/health"),
            "https://admin:***@api.shop/health"
        );
        // no credentials → untouched
        assert_eq!(
            redact_endpoint("https://api.shop/health"),
            "https://api.shop/health"
        );
        assert_eq!(redact_endpoint("db.internal:5432"), "db.internal:5432");
        // a bare username is not a secret
        assert_eq!(
            redact_endpoint("redis://cache@r1:6379"),
            "redis://cache@r1:6379"
        );
        // an '@' inside the PATH must not be mistaken for userinfo
        assert_eq!(
            redact_endpoint("https://api.shop/users/me@example.com"),
            "https://api.shop/users/me@example.com"
        );
    }

    #[test]
    fn text_lists_the_fields_and_skips_empty_ones() {
        let t = n(true, false).text();
        assert!(t.starts_with("🔴 api.shop — DOWN\n"), "{t}");
        assert!(t.contains("\nCondition: is DOWN"), "{t}");
        assert!(t.ends_with("\n— Vantage"), "{t}");
        // the test payload has no type/workspace/condition — those rows must not appear
        let test = Notification::test().text();
        assert!(!test.contains("Workspace:"), "{test}");
        assert!(test.contains("When: "), "{test}");
    }
}
