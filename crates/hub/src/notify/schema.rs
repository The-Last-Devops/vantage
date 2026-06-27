//! Provider registry: the display metadata + config-form schema for every notify
//! channel "type", plus validation and secret redaction derived from it.

use serde::Serialize;
use serde_json::{json, Value};

fn is_false(b: &bool) -> bool {
    !*b
}

/// One input in a provider's config form.
#[derive(Serialize, Clone)]
pub struct FieldSpec {
    pub key: &'static str,
    pub label: &'static str,
    /// text | secret | url | number | select | toggle | textarea
    #[serde(rename = "type")]
    pub ty: &'static str,
    #[serde(skip_serializing_if = "is_false")]
    pub required: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub advanced: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<&'static str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<&'static str>,
}

impl FieldSpec {
    fn req(mut self) -> Self {
        self.required = true;
        self
    }
    fn adv(mut self) -> Self {
        self.advanced = true;
        self
    }
    fn ph(mut self, p: &'static str) -> Self {
        self.placeholder = Some(p);
        self
    }
    fn hint(mut self, h: &'static str) -> Self {
        self.hint = Some(h);
        self
    }
    fn opts(mut self, o: &[&'static str], default: &'static str) -> Self {
        self.options = Some(o.to_vec());
        self.default = Some(default);
        self
    }
    fn default(mut self, d: &'static str) -> Self {
        self.default = Some(d);
        self
    }
}

fn field(key: &'static str, label: &'static str, ty: &'static str) -> FieldSpec {
    FieldSpec {
        key,
        label,
        ty,
        required: false,
        advanced: false,
        placeholder: None,
        hint: None,
        options: None,
        default: None,
    }
}

/// Display metadata + field schema for one provider, as served to the UI.
#[derive(Serialize, Clone)]
pub struct ProviderMeta {
    pub kind: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub color: &'static str,
    pub fg: &'static str,
    /// icon name resolved to an SVG by the frontend (falls back to a generic glyph).
    pub icon: &'static str,
    pub desc: &'static str,
    pub fields: Vec<FieldSpec>,
}

#[allow(clippy::too_many_arguments)]
fn p(
    kind: &'static str,
    name: &'static str,
    category: &'static str,
    color: &'static str,
    fg: &'static str,
    icon: &'static str,
    desc: &'static str,
    fields: Vec<FieldSpec>,
) -> ProviderMeta {
    ProviderMeta {
        kind,
        name,
        category,
        color,
        fg,
        icon,
        desc,
        fields,
    }
}

/// Every supported provider. Order also drives the picker's category grouping.
pub fn manifest() -> Vec<ProviderMeta> {
    use field as f;
    vec![
        // ---- Generic (most universal first) ----
        p("webhook", "Webhook", "Generic", "#34E1C4", "#06231f", "webhook",
          "POST a JSON payload to any URL", vec![
            f("url", "Endpoint URL", "url").req().ph("https://example.com/hook")
                .hint("Where to POST. Receives the alert as a JSON body."),
            f("method", "Method", "select").adv().opts(&["POST", "PUT", "PATCH"], "POST")
                .hint("HTTP verb to send. POST fits almost every receiver."),
            f("headers", "Custom headers", "textarea").adv()
                .ph("Authorization: Bearer …\nX-Source: vantage")
                .hint("Optional. One `Key: Value` per line — e.g. an auth header."),
            f("body", "Body template", "textarea").adv().ph("{\"text\": \"{{message}}\"}")
                .hint("Optional JSON template; {{message}} is replaced with the alert text. Default: {\"text\":\"<message>\"}."),
        ]),
        p("apprise", "Apprise API", "Generic", "#FF9900", "#241a00", "webhook",
          "One endpoint → 50+ services via a self-hosted apprise-api", vec![
            f("server", "apprise-api URL", "url").req().ph("https://apprise.example.com")
                .hint("Base URL of your running apprise-api instance."),
            f("urls", "Apprise URL(s)", "textarea").req()
                .ph("tgram://bottoken/ChatID\nmailto://user:pass@gmail.com")
                .hint("One Apprise service URL per line; each one gets notified."),
        ]),
        // ---- Chat ----
        p("telegram", "Telegram", "Chat", "#229ED9", "#fff", "telegram",
          "Bot messages to a chat, group or channel", vec![
            f("bot_token", "Bot token", "secret").req().ph("123456:ABC-DEF…")
                .hint("Create a bot with @BotFather and paste the token it gives you."),
            f("chat_id", "Chat ID", "text").req().ph("-1001234567890")
                .hint("ID of the chat, group or channel. Message the bot, then read it from /getUpdates."),
            f("thread_id", "Message thread ID", "text").adv().ph("optional")
                .hint("Topic ID inside a forum supergroup. Leave empty for normal chats/groups."),
            f("silent", "Send silently", "toggle").adv()
                .hint("Deliver with no sound — a quiet notification."),
        ]),
        p("slack", "Slack", "Chat", "#4A154B", "#fff", "slack",
          "Incoming webhook to a Slack channel", vec![
            f("url", "Incoming webhook URL", "secret").req().ph("https://hooks.slack.com/services/…")
                .hint("Slack → Apps → Incoming Webhooks → Add to a channel, then copy the URL."),
        ]),
        p("discord", "Discord", "Chat", "#5865F2", "#fff", "discord",
          "Webhook to a Discord channel", vec![
            f("url", "Webhook URL", "secret").req().ph("https://discord.com/api/webhooks/…")
                .hint("Channel → Edit → Integrations → Webhooks → New Webhook → Copy URL."),
            f("username", "Override bot name", "text").adv().ph("Vantage")
                .hint("Optional. Posts under this name instead of the webhook's default."),
            f("thread_id", "Thread ID", "text").adv().ph("123456789012345678")
                .hint("Optional. Post into an existing thread — right-click the thread → Copy ID (needs Developer Mode)."),
        ]),
        p("mattermost", "Mattermost", "Chat", "#1B57C2", "#fff", "chat",
          "Incoming webhook to a Mattermost channel", vec![
            f("url", "Webhook URL", "secret").req().ph("https://mm.example.com/hooks/…")
                .hint("Integrations → Incoming Webhooks → Add, then copy the URL."),
            f("channel", "Channel override", "text").adv().ph("town-square")
                .hint("Optional. Post to this channel instead of the webhook's default."),
        ]),
        p("teams", "Microsoft Teams", "Chat", "#4B53BC", "#fff", "chat",
          "Incoming webhook to a Teams channel", vec![
            f("url", "Webhook URL", "secret").req().ph("https://outlook.office.com/webhook/…")
                .hint("Channel → … → Connectors → Incoming Webhook → Configure, then copy the URL."),
        ]),
        p("gchat", "Google Chat", "Chat", "#1A73E8", "#fff", "chat",
          "Webhook to a Google Chat space", vec![
            f("url", "Webhook URL", "secret").req().ph("https://chat.googleapis.com/v1/spaces/…")
                .hint("Space → Apps & integrations → Webhooks → Add, then copy the URL."),
        ]),
        p("matrix", "Matrix", "Chat", "#0DBD8B", "#fff", "chat",
          "Post to a Matrix room", vec![
            f("homeserver", "Homeserver URL", "url").req().ph("https://matrix.org")
                .hint("Base URL of your Matrix server."),
            f("room_id", "Room ID", "text").req().ph("!abc123:matrix.org")
                .hint("Internal room ID (starts with !), not the #alias:server name."),
            f("token", "Access token", "secret").req().ph("syt_…")
                .hint("Access token of the sending account — found in your client's settings."),
        ]),
        // ---- Push ----
        p("ntfy", "ntfy", "Push", "#56B6C2", "#062b30", "push",
          "Push to a ntfy topic (self-host or ntfy.sh)", vec![
            f("server", "Server", "url").ph("https://ntfy.sh").default("https://ntfy.sh")
                .hint("Leave as ntfy.sh, or point to your self-hosted server."),
            f("topic", "Topic", "text").req().ph("vantage-alerts")
                .hint("Topic to publish to — anyone subscribed to it receives the alert."),
            f("token", "Access token", "secret").adv().ph("tk_…")
                .hint("Bearer token; only needed for protected/private topics."),
            f("priority", "Priority", "select").adv()
                .opts(&["min", "low", "default", "high", "max"], "default")
                .hint("Higher = more intrusive on the device (sound / vibration)."),
        ]),
        p("pushover", "Pushover", "Push", "#249DF1", "#fff", "push",
          "Push to your Pushover devices", vec![
            f("token", "Application token", "secret").req().ph("azGD…")
                .hint("Your Pushover application's API token/key."),
            f("user", "User key", "secret").req().ph("uQiR…")
                .hint("Your user (or group) key from the Pushover dashboard."),
            f("priority", "Priority", "select").adv().opts(&["-2", "-1", "0", "1", "2"], "0")
                .hint("-2 silent · 0 normal · 2 emergency (requires acknowledgement)."),
        ]),
        p("gotify", "Gotify", "Push", "#52B5E6", "#06222b", "push",
          "Push to a self-hosted Gotify server", vec![
            f("server", "Server URL", "url").req().ph("https://gotify.example.com")
                .hint("Base URL of your Gotify server."),
            f("token", "App token", "secret").req().ph("A…")
                .hint("An application token created in Gotify (Apps → Create)."),
            f("priority", "Priority", "number").adv().ph("5")
                .hint("0–10; higher shows more prominently on the device."),
        ]),
        p("bark", "Bark", "Push", "#FF4F4F", "#fff", "push",
          "Push to Bark (iOS)", vec![
            f("server", "Server", "url").ph("https://api.day.app").default("https://api.day.app")
                .hint("Leave as the default, or your self-hosted Bark server."),
            f("device_key", "Device key", "secret").req().ph("your-device-key")
                .hint("The key shown in the Bark app for your device."),
        ]),
        // ---- Incident ----
        p("pagerduty", "PagerDuty", "Incident", "#06AC38", "#fff", "incident",
          "Trigger a PagerDuty incident", vec![
            f("routing_key", "Integration / routing key", "secret").req().ph("R0…")
                .hint("Events API v2 'Integration Key' from the PagerDuty service to alert."),
            f("severity", "Severity", "select").adv()
                .opts(&["critical", "error", "warning", "info"], "error")
                .hint("Severity stamped on the triggered incident."),
        ]),
        p("opsgenie", "Opsgenie", "Incident", "#172B4D", "#fff", "incident",
          "Create an Opsgenie alert", vec![
            f("api_key", "API key", "secret").req().ph("…")
                .hint("An Opsgenie API integration key with create-alert permission."),
            f("region", "Region", "select").adv().opts(&["us", "eu"], "us")
                .hint("Match your Opsgenie account region (US or EU)."),
        ]),
        // ---- SMS ----
        p("twilio", "Twilio SMS", "SMS", "#F22F46", "#fff", "sms",
          "Send an SMS via Twilio", vec![
            f("account_sid", "Account SID", "text").req().ph("AC…")
                .hint("From your Twilio Console dashboard."),
            f("auth_token", "Auth token", "secret").req().ph("…")
                .hint("From your Twilio Console — kept secret."),
            f("from", "From number", "text").req().ph("+1…")
                .hint("A Twilio number you own, in E.164 form (+countrycode…)."),
            f("to", "To number", "text").req().ph("+1…")
                .hint("Destination number in E.164 form (+countrycode…)."),
        ]),
        // ---- Email ----
        p("email", "Email (SMTP)", "Email", "#EA4335", "#fff", "email",
          "Send email through your own SMTP server", vec![
            f("host", "SMTP host", "text").req().ph("smtp.example.com")
                .hint("Hostname of your outgoing mail server."),
            f("port", "Port", "number").req().ph("587").default("587")
                .hint("587 for STARTTLS (typical), 465 for SSL, 25 unencrypted."),
            f("username", "Username", "text").ph("alerts@example.com")
                .hint("SMTP login. Leave empty if the server needs no auth."),
            f("password", "Password", "secret").ph("••••••••")
                .hint("SMTP password or app-specific password."),
            f("from", "From", "text").req().ph("Vantage <alerts@example.com>")
                .hint("Sender address. 'Name <addr>' format is allowed."),
            f("to", "To", "text").req().ph("oncall@example.com")
                .hint("Recipient address for the alert email."),
        ]),
    ]
}

/// Whether `kind` is a known provider (used to validate create/patch requests).
pub fn is_valid_kind(kind: &str) -> bool {
    manifest().iter().any(|m| m.kind == kind)
}

/// A copy of `config` with every `secret`-typed field masked. Shown to users who
/// can view a channel but not edit it, so credentials (tokens, passwords, webhook
/// URLs) never reach a viewer. Editors get the real config to populate the form.
pub fn redact_secrets(kind: &str, config: &Value) -> Value {
    let mut out = config.clone();
    let (Some(meta), Some(obj)) = (
        manifest().into_iter().find(|m| m.kind == kind),
        out.as_object_mut(),
    ) else {
        return out;
    };
    for f in meta.fields.iter().filter(|f| f.ty == "secret") {
        if let Some(v) = obj.get_mut(f.key) {
            if v.as_str().is_some_and(|s| !s.is_empty()) {
                *v = json!("••••••");
            }
        }
    }
    out
}

#[cfg(test)]
mod redact_tests {
    use super::{is_valid_kind, redact_secrets};
    use serde_json::json;

    #[test]
    fn masks_secret_fields() {
        let out = redact_secrets(
            "discord",
            &json!({ "url": "https://discord.com/api/webhooks/xyz" }),
        );
        assert_eq!(out["url"], json!("••••••"));
    }

    #[test]
    fn leaves_non_secret_fields_intact() {
        let out = redact_secrets(
            "telegram",
            &json!({ "bot_token": "123:abc", "chat_id": "-100999" }),
        );
        assert_eq!(out["bot_token"], json!("••••••")); // secret → masked
        assert_eq!(out["chat_id"], json!("-100999")); // text → untouched
    }

    #[test]
    fn empty_secret_is_not_masked() {
        let out = redact_secrets("discord", &json!({ "url": "" }));
        assert_eq!(out["url"], json!(""));
    }

    #[test]
    fn unknown_kind_is_passthrough() {
        let cfg = json!({ "url": "secret-ish" });
        assert_eq!(redact_secrets("not-a-kind", &cfg), cfg);
    }

    #[test]
    fn valid_kinds_recognized() {
        assert!(is_valid_kind("telegram"));
        assert!(is_valid_kind("discord"));
        assert!(is_valid_kind("webhook"));
    }

    #[test]
    fn invalid_kinds_rejected() {
        assert!(!is_valid_kind("nope"));
        assert!(!is_valid_kind(""));
    }
}
