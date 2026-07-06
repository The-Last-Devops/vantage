//! Alerting API — alert rules, notify channels, and public status pages.
//!
//! Split into three concerns, each re-exported here so callers keep using
//! `api::create_alert`, `api::list_channels`, etc. unchanged:
//! - [`alerts`] — alert-rule logic (list/get/create/patch/delete/test rules,
//!   alert events, and the monitor/system "covering rules" views).
//! - [`channels`] — notify-channel logic (list/create/patch/delete/test channels,
//!   channel types, unsaved-config test).
//! - [`status_pages`] — public status-page create/delete.

// Re-export the parent `api` module's helpers and shared imports (`internal`,
// `ws_of`, `valid_name`, `valid_ws_name`, `AppState`, `CurrentUser`, `rbac`,
// the axum/serde/uuid re-exports, …) so the submodules below reach them via
// `use super::*;` exactly as the un-split file did with `super::valid_name`, etc.
pub use super::*;

mod alerts;
mod channels;
mod status_pages;

pub use alerts::*;
pub use channels::*;
pub use status_pages::*;
