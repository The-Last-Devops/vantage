use super::*;

#[derive(Serialize)]
pub struct DataPage {
    data: crate::data_admin::DataDbStats,
    config: crate::data_admin::DbStats,
}

/// GET /api/admin/data — both databases: the Data DB (hypertables + retention + cap)
/// and the config DB (relational tables, read-only).
pub async fn data_stats(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<DataPage>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(Json(DataPage {
        data: crate::data_admin::data_stats(&state.config, &state.data).await,
        config: crate::data_admin::config_stats(&state.config).await,
    }))
}

/// Hub-decided push cadence (seconds): `host` = node/host metrics ("realtime"),
/// `kube` = k8s cluster scrape (heavier, so higher). Agents obey these via IngestAck.
#[derive(Serialize, Deserialize)]
pub struct IngestIntervals {
    pub host: i64,
    pub kube: i64,
}

pub async fn get_ingest_intervals(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<IngestIntervals>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(Json(IngestIntervals {
        host: crate::settings::get(&state.config, "ingest_interval_secs", 5_i64).await,
        kube: crate::settings::get(&state.config, "kube_interval_secs", 15_i64).await,
    }))
}

pub async fn set_ingest_intervals(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<IngestIntervals>,
) -> Result<StatusCode, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    if !(1..=3600).contains(&req.host) || !(5..=3600).contains(&req.kube) {
        return Err(StatusCode::BAD_REQUEST);
    }
    crate::settings::set(&state.config, "ingest_interval_secs", &req.host)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    crate::settings::set(&state.config, "kube_interval_secs", &req.kube)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct LogsPage {
    lines: Vec<String>,
}

/// GET /api/admin/logs — recent hub log lines from the in-memory ring buffer, for
/// debugging from the UI. Admin-only: logs can reveal operational detail.
pub async fn admin_logs(user: CurrentUser) -> Result<Json<LogsPage>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(Json(LogsPage {
        lines: crate::logbuf::recent(2000),
    }))
}

#[derive(Deserialize)]
pub struct SetCap {
    pub limit_bytes: i64,
    pub enabled: bool,
}

#[derive(Deserialize)]
pub struct SetConfigRetention {
    pub table: String,
    pub days: i64,
}

/// POST /api/admin/config-retention — set a config-DB log table's cleanup window.
pub async fn set_config_retention(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<SetConfigRetention>,
) -> Result<StatusCode, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    crate::data_admin::set_config_retention(&state.config, &req.table, req.days)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/admin/data-cap — set the Data DB size cap + auto-evict toggle.
pub async fn set_data_cap(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<SetCap>,
) -> Result<StatusCode, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    crate::data_admin::set_data_cap(&state.config, req.limit_bytes, req.enabled)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/admin/data-cap/enforce — run one eviction pass now and report what it freed.
/// A no-op (freed 0) when the cap is disabled or the DB is already under the limit.
pub async fn enforce_data_cap(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<crate::data_admin::EvictionResult>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(Json(
        crate::data_admin::enforce_cap(&state.config, &state.data).await,
    ))
}

#[derive(Deserialize)]
pub struct SetRetention {
    pub table: String,
    /// Interpreted in the tier's unit (hours for raw, days for rollups).
    pub value: i64,
}

/// POST /api/admin/retention — change a tier's retention window.
pub async fn set_retention(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<SetRetention>,
) -> Result<StatusCode, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    crate::data_admin::set_retention(&state.config, &state.data, &req.table, req.value)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- workspaces -------------------------------------------------------------
