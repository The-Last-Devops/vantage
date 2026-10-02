use super::*;

#[derive(Deserialize)]
pub struct PatchServer {
    pub name: String,
}

/// PATCH /api/systems/:id — rename (cosmetic; workspace is governed by the token).
pub async fn patch_system(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchServer>,
) -> Result<StatusCode, StatusCode> {
    let ws = ws_of(&state, "SELECT workspace_id FROM systems WHERE id = $1", id).await?;
    rbac::require_role(&state, &user, ws, Role::Editor).await?;
    if !super::valid_name(&req.name, 80) {
        return Err(StatusCode::BAD_REQUEST);
    }
    sqlx::query("UPDATE systems SET name = $2 WHERE id = $1")
        .bind(id)
        .bind(req.name.trim())
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/systems/:id — removes a single server row (it re-registers if its
/// agent is still pushing; use token delete to stop enrollment entirely).
pub async fn delete_system(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let ws = ws_of(&state, "SELECT workspace_id FROM systems WHERE id = $1", id).await?;
    rbac::require_role(&state, &user, ws, Role::Editor).await?;
    sqlx::query("DELETE FROM systems WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    // The two databases are linked only by id at the application layer, so deleting the
    // config row leaves every metric this host ever pushed behind — invisible, nameless,
    // and alive until retention expires, which for the hourly tier is a year. Clean it up
    // here; this is the only moment anything knows the id is finished with.
    //
    // Best-effort on purpose: the system IS deleted either way, and anything missed still
    // ages out exactly as before. Failing the request over leftover rows would be worse
    // than leaving them.
    crate::data_admin::purge_system(&state.data, id).await;
    Ok(StatusCode::NO_CONTENT)
}
