//! Database connection + migration wiring for the two separate pools.

use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;

use crate::AppState;

pub async fn connect() -> Result<AppState> {
    let config_url =
        std::env::var("CONFIG_DATABASE_URL").context("CONFIG_DATABASE_URL is required")?;
    let data_url = std::env::var("DATA_DATABASE_URL").context("DATA_DATABASE_URL is required")?;

    let config = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config_url)
        .await
        .context("connecting to config DB")?;
    let data = PgPoolOptions::new()
        .max_connections(10)
        .connect(&data_url)
        .await
        .context("connecting to data DB")?;

    let app_secrets = crate::exec_crypto::AppSecrets::from_env();
    if app_secrets.enabled() {
        tracing::info!("EXEC_APP_SECRET set — SSH key master keys are wrapped with the app secret");
    } else {
        tracing::warn!(
            "EXEC_APP_SECRET is not set — users' SSH key master keys are protected by password \
             ONLY. A DB leak + a guessed password would expose keys. Set EXEC_APP_SECRET in \
             production (see README), then run `vantage-hub rotate-app-secret`."
        );
    }

    Ok(AppState {
        config,
        data,
        tunnels: crate::tunnel::TunnelRegistry::new(),
        exec_tickets: crate::console::ExecTickets::new(),
        app_secrets: std::sync::Arc::new(app_secrets),
        passkey: std::sync::Arc::new(crate::passkey::PasskeyState::from_env()),
        login_throttle: std::sync::Arc::new(crate::auth::LoginThrottle::new()),
    })
}

/// If `LOCAL_API_KEY` is set, ensure a `default` workspace and a `local`
/// server enrolled with that key exist (idempotent). Lets the bundled
/// docker-compose agent report out of the box with no manual provisioning.
pub async fn bootstrap_local_server(pool: &sqlx::PgPool) -> Result<()> {
    let api_key = match std::env::var("LOCAL_API_KEY") {
        Ok(t) if !t.is_empty() => t,
        _ => return Ok(()),
    };
    let (ws,): (uuid::Uuid,) = sqlx::query_as(
        "INSERT INTO workspaces (name) VALUES ('default') \
         ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name RETURNING id",
    )
    .fetch_one(pool)
    .await?;
    // Reusable API key; systems auto-register on first push.
    sqlx::query(
        "INSERT INTO api_keys (workspace_id, name, key) VALUES ($1, 'local', $2) \
         ON CONFLICT (key) DO NOTHING",
    )
    .bind(ws)
    .bind(&api_key)
    .execute(pool)
    .await?;
    // Give every admin owner access to the default workspace.
    sqlx::query(
        "INSERT INTO memberships (user_id, workspace_id, role) \
         SELECT id, $1, 'owner' FROM users WHERE is_admin = true \
         ON CONFLICT (user_id, workspace_id) DO NOTHING",
    )
    .bind(ws)
    .execute(pool)
    .await?;
    tracing::info!("bootstrapped default workspace + local token");
    Ok(())
}

/// Apply migrations from the on-disk migration directories. Kept at runtime
/// (not the compile-time macro) so the two DBs stay independently relocatable.
pub async fn migrate(state: &AppState) -> Result<()> {
    sqlx::migrate::Migrator::new(std::path::Path::new("migrations/config"))
        .await?
        .run(&state.config)
        .await
        .context("running config migrations")?;
    sqlx::migrate::Migrator::new(std::path::Path::new("migrations/data"))
        .await?
        .run(&state.data)
        .await
        .context("running data migrations")?;
    tracing::info!("migrations applied");
    Ok(())
}
