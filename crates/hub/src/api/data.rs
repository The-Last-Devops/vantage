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
    crate::data_admin::set_retention(&state.data, &req.table, req.value)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- workspaces -------------------------------------------------------------
