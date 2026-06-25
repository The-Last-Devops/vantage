use super::*;

#[derive(Deserialize)]
pub struct CreateMonitor {
    pub name: String,
    pub kind: String, // http | tcp | ping | keyword
    pub target: String,
    #[serde(default)]
    pub interval_secs: Option<i32>,
    #[serde(default)]
    pub config: Option<Value>,
}

/// POST /api/namespaces/:id/monitors — editors+ add a service check.
pub async fn create_monitor(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
    Json(req): Json<CreateMonitor>,
) -> Result<Json<Uuid>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    if !matches!(
        req.kind.as_str(),
        "http"
            | "tcp"
            | "ping"
            | "keyword"
            | "postgres"
            | "redis"
            | "dns"
            | "rabbitmq"
            | "mysql"
            | "mongodb"
            | "tls"
            | "push"
    ) {
        return Err(StatusCode::BAD_REQUEST);
    }
    // A push monitor has no target (it's a generated URL); everything else needs one.
    if !super::valid_name(&req.name, 80) || (req.kind != "push" && req.target.trim().is_empty()) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut config = req.config.unwrap_or_else(|| serde_json::json!({}));
    // Push monitors get a generated token; the agent/cron calls /pub/push/<token>.
    if req.kind == "push" {
        if let Some(obj) = config.as_object_mut() {
            obj.entry("push_token")
                .or_insert_with(|| serde_json::json!(Uuid::new_v4().simple().to_string()));
        }
    }

    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO monitors (namespace_id, name, kind, target, interval_secs, config) \
         VALUES ($1, $2, $3::monitor_kind, $4, $5, $6) RETURNING id",
    )
    .bind(ns)
    .bind(req.name.trim())
    .bind(&req.kind)
    .bind(req.target.trim())
    .bind(req.interval_secs.unwrap_or(60).max(1))
    .bind(sqlx::types::Json(config))
    .fetch_one(&state.config)
    .await
    .map_err(internal)?;

    Ok(Json(id))
}

/// GET /api/monitors/:id/debug — last ok + last err request/response detail.
pub async fn monitor_debug(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM monitors WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Viewer).await?;
    let rows: Vec<(String, Value, String)> =
        sqlx::query_as("SELECT outcome, detail, at::text FROM monitor_debug WHERE monitor_id = $1")
            .bind(id)
            .fetch_all(&state.config)
            .await
            .map_err(internal)?;
    let (mut ok, mut err) = (Value::Null, Value::Null);
    for (outcome, detail, at) in rows {
        let entry = serde_json::json!({ "detail": detail, "at": at });
        if outcome == "ok" {
            ok = entry;
        } else {
            err = entry;
        }
    }
    Ok(Json(serde_json::json!({ "ok": ok, "err": err })))
}

// ---- notification channels --------------------------------------------------

#[derive(Deserialize)]
pub struct PatchMonitor {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub interval_secs: Option<i32>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub config: Option<Value>,
}

/// PATCH /api/monitors/:id — edit fields / toggle enabled.
pub async fn patch_monitor(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchMonitor>,
) -> Result<StatusCode, StatusCode> {
    let (ns, kind, existing): (Uuid, String, sqlx::types::Json<Value>) =
        sqlx::query_as("SELECT namespace_id, kind::text, config FROM monitors WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.config)
            .await
            .map_err(internal)?
            .ok_or(StatusCode::NOT_FOUND)?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    // Validate only the fields actually being changed.
    if req
        .name
        .as_deref()
        .is_some_and(|n| !super::valid_name(n, 80))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if req
        .target
        .as_deref()
        .is_some_and(|t| kind != "push" && t.trim().is_empty())
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Push monitors are addressed by their token; never let an edit drop it. The
    // edit form rebuilds config from its fields and may omit push_token, so carry
    // the existing one over (or mint one if somehow missing) whenever kind=push.
    let config = if kind == "push" {
        let mut cfg = req.config.clone().unwrap_or_else(|| existing.0.clone());
        if !cfg.is_object() {
            cfg = serde_json::json!({});
        }
        let obj = cfg.as_object_mut().expect("ensured object above");
        // The token is server-owned: always take the stored one (mint if absent)
        // and ignore whatever the client sent — clients can't set or change it.
        let token = existing
            .0
            .get("push_token")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string());
        obj.insert("push_token".into(), serde_json::json!(token));
        Some(cfg)
    } else {
        req.config.clone()
    };

    sqlx::query(
        "UPDATE monitors SET name = COALESCE($2, name), target = COALESCE($3, target), \
         interval_secs = COALESCE($4, interval_secs), enabled = COALESCE($5, enabled), \
         config = COALESCE($6, config) WHERE id = $1",
    )
    .bind(id)
    .bind(req.name.map(|n| n.trim().to_string()))
    .bind(req.target.map(|t| t.trim().to_string()))
    .bind(req.interval_secs)
    .bind(req.enabled)
    .bind(config.map(sqlx::types::Json))
    .execute(&state.config)
    .await
    .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/monitors/:id
pub async fn delete_monitor(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM monitors WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query("DELETE FROM monitors WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}
