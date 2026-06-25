//! last-monitor hub: ingest endpoint, JSON API, and the server-rendered web UI.
//!
//! Environment:
//!   CONFIG_DATABASE_URL  Postgres URL for the config DB (users, namespaces, configs)
//!   DATA_DATABASE_URL    Postgres URL for the data DB (metrics, TimescaleDB)
//!   BIND_ADDR            listen address, default 0.0.0.0:8080
#![allow(clippy::type_complexity, clippy::items_after_test_module)]

mod alert;
mod api;
mod audit;
mod auth;
mod backup;
mod data_admin;
mod db;
mod ingest;
mod install;
mod mcp;
mod notify;
mod probe;
mod rbac;
mod spa;
mod web;

use std::net::SocketAddr;

use anyhow::{Context, Result};
use axum::{routing::get, Router};
use tower_http::trace::TraceLayer;

/// Shared application state handed to every handler.
#[derive(Clone)]
pub struct AppState {
    /// Pool for the config DB (plain Postgres).
    pub config: sqlx::PgPool,
    /// Pool for the data DB (Postgres + TimescaleDB).
    pub data: sqlx::PgPool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();

    let state = db::connect().await?;
    db::migrate(&state).await?;
    auth::bootstrap_admin(&state.config).await?;
    db::bootstrap_local_server(&state.config).await?;
    data_admin::setup(&state.data).await;

    // Background engines.
    probe::spawn(state.clone());
    alert::spawn(state.clone());
    backup::spawn(state.clone());

    use axum::routing::{delete, patch, post};
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        // Everything machines hit (no human session) lives under /pub so a single
        // Cloudflare Access "bypass" rule on /pub covers them all. They self-auth:
        // ingest by api-key; install assets only echo caller-supplied values.
        .route("/pub/ingest", post(ingest::ingest))
        .route("/pub/push/{token}", get(ingest::push).post(ingest::push))
        .route("/pub/agent.yaml", get(install::k8s_agent_yaml))
        .route("/pub/install.sh", get(install::install_sh))
        // auth + first-run setup
        .route(
            "/api/setup",
            get(auth::setup_status).post(auth::setup_create),
        )
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/me", get(auth::me))
        // admin user provisioning + data management
        .route("/mcp", post(mcp::handle))
        .route("/api/pats", get(api::list_pats).post(api::create_pat))
        .route("/api/pats/{id}", delete(api::delete_pat))
        .route("/api/users", get(api::list_users).post(api::create_user))
        .route(
            "/api/users/{id}",
            patch(api::patch_user).delete(api::delete_user),
        )
        .route("/api/users/{id}/memberships", get(api::user_memberships))
        .route("/api/admin/data", get(api::data_stats))
        .route("/api/admin/retention", post(api::set_retention))
        // backup / restore (admin)
        .route("/api/admin/backup", get(backup::download))
        .route(
            "/api/admin/restore",
            post(backup::restore).layer(axum::extract::DefaultBodyLimit::max(512 * 1024 * 1024)),
        )
        .route(
            "/api/admin/backup/s3",
            get(backup::s3_get).put(backup::s3_put),
        )
        .route("/api/admin/backup/s3/test", post(backup::s3_test))
        .route("/api/admin/backup/s3/upload", post(backup::s3_upload))
        .route("/api/admin/backup/s3/list", get(backup::s3_list))
        .route("/api/admin/backup/s3/restore", post(backup::s3_restore))
        .route(
            "/api/admin/backup/schedule",
            get(backup::schedule_get).put(backup::schedule_put),
        )
        .route("/api/audit", get(audit::list))
        .route("/api/about", get(api::about))
        // management (session + RBAC)
        .route(
            "/api/namespaces",
            get(api::list_namespaces).post(api::create_namespace),
        )
        .route("/api/namespaces/{id}", delete(api::delete_namespace))
        .route("/api/thresholds", get(api::list_thresholds))
        .route(
            "/api/namespaces/{id}/thresholds",
            axum::routing::put(api::set_thresholds),
        )
        .route(
            "/api/namespaces/{id}/members",
            get(api::list_members).post(api::add_member),
        )
        // API keys (reusable; systems auto-register)
        .route(
            "/api/namespaces/{id}/keys",
            get(api::list_keys).post(api::create_key),
        )
        .route("/api/keys/{id}", delete(api::delete_key))
        .route("/api/keys/{id}/systems", get(api::key_systems))
        .route("/api/namespaces/{id}/monitors", post(api::create_monitor))
        .route("/api/channel-types", get(api::channel_types))
        .route("/api/channels", get(api::list_all_channels))
        .route("/api/channels/{id}/alerts", get(api::channel_alerts))
        .route("/api/monitors/{id}/alerts", get(api::monitor_alerts))
        .route("/api/systems/{id}/alerts", get(api::system_alerts))
        .route(
            "/api/namespaces/{id}/channels",
            get(api::list_channels).post(api::create_channel),
        )
        .route(
            "/api/namespaces/{id}/alerts",
            get(api::list_alerts).post(api::create_alert),
        )
        .route("/api/namespaces/{id}/alert-events", get(api::alert_events))
        .route(
            "/api/namespaces/{id}/status-pages",
            post(api::create_status_page),
        )
        // edit / delete resources
        .route(
            "/api/systems/{id}",
            patch(api::patch_system).delete(api::delete_system),
        )
        .route(
            "/api/monitors/{id}",
            get(web::monitor_detail)
                .patch(api::patch_monitor)
                .delete(api::delete_monitor),
        )
        .route("/api/monitors/{id}/debug", get(api::monitor_debug))
        .route(
            "/api/monitors/{id}/heartbeats",
            get(web::monitor_heartbeats),
        )
        .route("/api/monitors/{id}/events", get(web::monitor_events))
        .route("/api/events", get(web::recent_events))
        .route(
            "/api/channels/{id}",
            patch(api::patch_channel).delete(api::delete_channel),
        )
        .route("/api/channels/{id}/test", post(api::test_channel))
        .route(
            "/api/alerts/{id}",
            patch(api::patch_alert).delete(api::delete_alert),
        )
        .route("/api/alerts/{id}/test", post(api::test_alert))
        .route("/api/status-pages/{id}", delete(api::delete_status_page))
        .route(
            "/api/namespaces/{id}/members/{user_id}",
            delete(api::delete_member),
        )
        // read views (scoped to caller)
        .route("/api/systems", get(web::list_systems))
        .route("/api/fleet", get(web::fleet))
        .route("/api/systems/{id}/metrics", get(web::system_metrics_series))
        .route("/api/systems/{id}/containers", get(web::system_containers))
        .route("/api/systems/{id}/temps", get(web::system_temps))
        .route("/api/systems/{id}/gpu", get(web::system_gpu))
        .route("/api/monitors", get(web::list_monitors))
        // SPA: anything not matched above is served from the embedded Vue build.
        .fallback(spa::handler)
        // Audit middleware logs mutating /api calls (needs state; before with_state).
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            audit::record,
        ))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .context("invalid BIND_ADDR")?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "hub listening");
    // Drain in-flight requests on SIGTERM/Ctrl-C instead of dropping them — lets
    // orchestrators (Docker/k8s) stop the hub cleanly within their grace period.
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("hub stopped");
    Ok(())
}

/// Resolves on the first Ctrl-C or SIGTERM, so callers can shut down gracefully.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = term => {}
    }
    tracing::info!("shutdown signal received — draining");
}
