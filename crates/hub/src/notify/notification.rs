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
    pub detail: String,    // the probe / evaluation message
    pub at: String,        // formatted UTC timestamp
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
            detail: "Your channel is wired up correctly — real alerts will arrive here.".into(),
            at: now_utc(),
        }
    }
    pub(crate) fn status_word(&self) -> &'static str {
        if !self.firing {
            "RECOVERED"
        } else if self.repeat {
            "STILL FIRING"
        } else {
            "ALERT"
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

fn now_utc() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M UTC").to_string()
}
