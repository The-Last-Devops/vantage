//! last-monitor hub: ingest endpoint, JSON API, and the server-rendered web UI.
//!
//! Environment:
//!   CONFIG_DATABASE_URL  Postgres URL for the config DB (users, namespaces, configs)
//!   DATA_DATABASE_URL    Postgres URL for the data DB (metrics, TimescaleDB)
//!   BIND_ADDR            listen address, default 0.0.0.0:8080
#![allow(clippy::type_complexity, clippy::items_after_test_module)]

mod alert;
mod api;
mod auth;
mod data_admin;
mod db;
mod ingest;
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

    use axum::routing::{delete, patch, post};
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        // agent ingest (api-key-authenticated, not session)
        .route("/api/ingest", post(ingest::ingest))
        // auth + first-run setup
        .route(
            "/api/setup",
            get(auth::setup_status).post(auth::setup_create),
        )
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/me", get(auth::me))
        // admin user provisioning + data management
        .route("/api/users", post(api::create_user))
        .route("/api/admin/data", get(api::data_stats))
        .route("/api/admin/retention", post(api::set_retention))
        // management (session + RBAC)
        .route(
            "/api/namespaces",
            get(api::list_namespaces).post(api::create_namespace),
        )
        .route("/api/namespaces/{id}", delete(api::delete_namespace))
        .route("/api/namespaces/{id}/members", post(api::add_member))
        // API keys (reusable; systems auto-register)
        .route(
            "/api/namespaces/{id}/keys",
            get(api::list_keys).post(api::create_key),
        )
        .route("/api/keys/{id}", delete(api::delete_key))
        .route("/api/keys/{id}/systems", get(api::key_systems))
        .route("/api/namespaces/{id}/monitors", post(api::create_monitor))
        .route(
            "/api/namespaces/{id}/channels",
            get(api::list_channels).post(api::create_channel),
        )
        .route(
            "/api/namespaces/{id}/alerts",
            get(api::list_alerts).post(api::create_alert),
        )
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
            patch(api::patch_monitor).delete(api::delete_monitor),
        )
        .route("/api/channels/{id}", delete(api::delete_channel))
        .route("/api/alerts/{id}", delete(api::delete_alert))
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
        .with_state(state)
        // SPA: anything not matched above is served from the embedded Vue build.
        .fallback(spa::handler)
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .context("invalid BIND_ADDR")?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "hub listening");
    axum::serve(listener, app).await?;
    Ok(())
}
