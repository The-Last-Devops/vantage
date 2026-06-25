use super::*;

#[derive(Deserialize)]
pub struct PatchServer {
    pub name: String,
}

/// PATCH /api/systems/:id — rename (cosmetic; namespace is governed by the token).
pub async fn patch_system(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchServer>,
) -> Result<StatusCode, StatusCode> {
    let ns = ns_of(&state, "SELECT namespace_id FROM systems WHERE id = $1", id).await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
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
    let ns = ns_of(&state, "SELECT namespace_id FROM systems WHERE id = $1", id).await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query("DELETE FROM systems WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}
