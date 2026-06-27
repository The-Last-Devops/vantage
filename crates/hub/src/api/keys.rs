use super::*;

#[derive(Serialize)]
pub struct CreatedKey {
    pub id: Uuid,
    pub name: String,
    /// The reusable API key (sent as x-api-key on any number of hosts).
    pub key: String,
}

#[derive(Deserialize)]
pub struct CreateKey {
    pub name: String,
}

/// POST /api/namespaces/:id/keys — editors+ mint a reusable API key.
pub async fn create_key(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
    Json(req): Json<CreateKey>,
) -> Result<Json<CreatedKey>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    let name = req.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(StatusCode::BAD_REQUEST);
    }
    // 32-char key (UUIDv4 hex, ~122 bits of entropy).
    let key = Uuid::new_v4().simple().to_string();
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO api_keys (namespace_id, name, key) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(ns)
    .bind(&name)
    .bind(&key)
    .fetch_one(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(CreatedKey { id, name, key }))
}

#[derive(Serialize)]
pub struct KeyRow {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    pub system_count: i64,
}

/// GET /api/namespaces/:id/keys
pub async fn list_keys(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
) -> Result<Json<Vec<KeyRow>>, StatusCode> {
    let role = rbac::require_role(&state, &user, ns, Role::Viewer).await?;
    // The enrollment key is a secret that grants metric ingest / auto-registration
    // into the namespace; only editors+ may see it in full. Viewers get a masked
    // preview that can't be used to enroll an agent.
    let can_see_secret = role >= Role::Editor;
    let rows: Vec<(Uuid, String, String, i64)> = sqlx::query_as(
        "SELECT k.id, k.name, k.key, count(s.id) \
         FROM api_keys k LEFT JOIN systems s ON s.key_id = k.id \
         WHERE k.namespace_id = $1 GROUP BY k.id ORDER BY k.name",
    )
    .bind(ns)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, key, system_count)| KeyRow {
                id,
                name,
                key: if can_see_secret { key } else { mask_key(&key) },
                system_count,
            })
            .collect(),
    ))
}

/// Non-reversible preview of an enrollment key for viewers: first 6 chars + "…".
fn mask_key(key: &str) -> String {
    let prefix: String = key.chars().take(6).collect();
    format!("{prefix}…")
}

#[derive(Serialize)]
pub struct KeySystems {
    pub key_name: String,
    pub systems: Vec<String>,
}

/// GET /api/keys/:id/systems — which systems a key enrolled (for delete warning).
pub async fn key_systems(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<KeySystems>, StatusCode> {
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM api_keys WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Viewer).await?;
    let key_name: (String,) = sqlx::query_as("SELECT name FROM api_keys WHERE id = $1")
        .bind(id)
        .fetch_one(&state.config)
        .await
        .map_err(internal)?;
    let systems: Vec<(String, String)> =
        sqlx::query_as("SELECT name, hostname FROM systems WHERE key_id = $1 ORDER BY hostname")
            .bind(id)
            .fetch_all(&state.config)
            .await
            .map_err(internal)?;
    Ok(Json(KeySystems {
        key_name: key_name.0,
        systems: systems.into_iter().map(|(_, h)| h).collect(),
    }))
}

/// DELETE /api/keys/:id — removes the key and cascades to every system it enrolled.
pub async fn delete_key(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM api_keys WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query("DELETE FROM api_keys WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- monitors ---------------------------------------------------------------
