//! Public status-page API: create and delete the public status pages a
//! workspace publishes.

use super::*;

/// DELETE /api/status-pages/:id
pub async fn delete_status_page(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let ws = ws_of(
        &state,
        "SELECT workspace_id FROM status_pages WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ws, Role::Editor).await?;
    sqlx::query("DELETE FROM status_pages WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CreateStatusPage {
    pub slug: String,
    pub title: String,
    /// Optional {"monitor_ids":[...]}; empty => all monitors in the workspace.
    #[serde(default)]
    pub config: Option<Value>,
}

/// POST /api/workspaces/:id/status-pages — editors+ publish a public status page.
pub async fn create_status_page(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ws): Path<Uuid>,
    Json(req): Json<CreateStatusPage>,
) -> Result<Json<Uuid>, StatusCode> {
    rbac::require_role(&state, &user, ws, Role::Editor).await?;
    // slug is part of the public URL → strict identifier; title is a display name.
    if !super::valid_ws_name(&req.slug) || !super::valid_name(&req.title, 120) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO status_pages (workspace_id, slug, title, config) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(ws)
    .bind(&req.slug)
    .bind(req.title.trim())
    .bind(sqlx::types::Json(
        req.config.unwrap_or_else(|| serde_json::json!({})),
    ))
    .fetch_one(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(id))
}
