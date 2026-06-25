use super::*;

/// Conservative email check: ASCII only, no whitespace, exactly one `@`, a dotted
/// domain. Rejects junk like "kiên béo ngu dốt @gmail.com". Not full RFC 5322 —
/// intentionally strict so display/login stays clean.
fn valid_email(e: &str) -> bool {
    if e.is_empty() || e.len() > 254 {
        return false;
    }
    let mut parts = e.split('@');
    let (local, domain) = match (parts.next(), parts.next(), parts.next()) {
        (Some(l), Some(d), None) => (l, d),
        _ => return false, // zero or more than one '@'
    };
    if local.is_empty() || domain.len() < 3 || domain.starts_with('.') || domain.ends_with('.') {
        return false;
    }
    if !domain.contains('.') {
        return false;
    }
    let local_ok = |c: char| c.is_ascii_alphanumeric() || "._%+-".contains(c);
    let domain_ok = |c: char| c.is_ascii_alphanumeric() || ".-".contains(c);
    local.chars().all(local_ok) && domain.chars().all(domain_ok)
}

#[derive(Deserialize)]
pub struct CreateUser {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub is_admin: bool,
    #[serde(default)]
    pub read_all: bool,
}

/// POST /api/users — admins provision accounts (no open registration).
pub async fn create_user(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<CreateUser>,
) -> Result<Json<Uuid>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let email = req.email.trim().to_lowercase();
    if !valid_email(&email) || req.password.len() < 6 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let hash = crate::auth::hash_password(&req.password).map_err(internal)?;
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO users (email, password_hash, is_admin, read_all) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(&email)
    .bind(&hash)
    .bind(req.is_admin)
    .bind(req.read_all)
    .fetch_one(&state.config)
    .await
    .map_err(|_| StatusCode::CONFLICT)?; // unique email violation → 409
    Ok(Json(id))
}

#[derive(Serialize)]
pub struct UserRow {
    pub id: Uuid,
    pub email: String,
    pub is_admin: bool,
    pub read_all: bool,
    pub created_at: String,
    pub namespaces: i64,
}

/// GET /api/users — admins list all accounts with their system role + ns count.
pub async fn list_users(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<UserRow>>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let rows: Vec<(Uuid, String, bool, bool, String, i64)> = sqlx::query_as(
        "SELECT u.id, u.email, u.is_admin, u.read_all, u.created_at::text, \
            (SELECT count(*) FROM memberships m WHERE m.user_id = u.id) \
         FROM users u ORDER BY u.email",
    )
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(
                |(id, email, is_admin, read_all, created_at, namespaces)| UserRow {
                    id,
                    email,
                    is_admin,
                    read_all,
                    created_at,
                    namespaces,
                },
            )
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct PatchUser {
    pub is_admin: Option<bool>,
    pub read_all: Option<bool>,
    pub password: Option<String>,
}

/// PATCH /api/users/:id — admins change system role / reset password.
pub async fn patch_user(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchUser>,
) -> Result<StatusCode, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    // don't let an admin remove their own admin rights (avoids locking everyone out)
    if id == user.id && req.is_admin == Some(false) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if let Some(is_admin) = req.is_admin {
        sqlx::query("UPDATE users SET is_admin = $1 WHERE id = $2")
            .bind(is_admin)
            .bind(id)
            .execute(&state.config)
            .await
            .map_err(internal)?;
    }
    if let Some(read_all) = req.read_all {
        sqlx::query("UPDATE users SET read_all = $1 WHERE id = $2")
            .bind(read_all)
            .bind(id)
            .execute(&state.config)
            .await
            .map_err(internal)?;
    }
    if let Some(password) = req.password {
        if password.len() < 6 {
            return Err(StatusCode::BAD_REQUEST);
        }
        let hash = crate::auth::hash_password(&password).map_err(internal)?;
        sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
            .bind(&hash)
            .bind(id)
            .execute(&state.config)
            .await
            .map_err(internal)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct UserMembership {
    pub namespace_id: Uuid,
    pub namespace: String,
    pub role: String,
}

/// GET /api/users/:id/memberships — admins list a user's per-namespace roles
/// (for the user editor). Namespaces the user isn't in are simply absent.
pub async fn user_memberships(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<UserMembership>>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT n.id, n.name, m.role::text FROM memberships m \
         JOIN namespaces n ON n.id = m.namespace_id WHERE m.user_id = $1 ORDER BY n.name",
    )
    .bind(id)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(namespace_id, namespace, role)| UserMembership {
                namespace_id,
                namespace,
                role,
            })
            .collect(),
    ))
}

/// DELETE /api/users/:id — admins remove an account (not their own).
pub async fn delete_user(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    if id == user.id {
        return Err(StatusCode::BAD_REQUEST);
    }
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- data management (admin) ------------------------------------------------
