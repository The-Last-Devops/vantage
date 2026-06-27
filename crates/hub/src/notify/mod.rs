//! Notification providers — where a notify channel "type" is defined.
//!
//! Split into three concerns:
//! - [`schema`] — each provider's display metadata + config-form field schema
//!   (served verbatim at `GET /api/channel-types`), plus validation and redaction.
//! - [`notification`] — the structured alert payload and its per-transport renderings.
//! - [`transports`] — the per-channel send logic ([`dispatch`]).
//!
//! Adding a provider = one entry in [`schema::manifest`] plus one arm in
//! [`dispatch`]; the UI updates from the manifest with no frontend changes.

mod notification;
mod schema;
mod transports;

pub use notification::Notification;
pub use schema::{is_valid_kind, manifest, redact_secrets, ProviderMeta};
pub use transports::dispatch;
