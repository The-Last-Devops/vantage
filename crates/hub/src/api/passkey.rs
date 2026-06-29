//! Passkey (WebAuthn) endpoints: register a passkey (session-authed, from Settings →
//! Security) and the login-time second-factor helpers used by `auth.rs`. The relying
//! party + ceremony state live in `crate::passkey`. Credentials persist as serde JSON
//! in `webauthn_credentials`. See docs/auth-2fa-passkey.md.

use super::*;
use crate::passkey::PasskeyState;
use webauthn_rs::prelude::{Passkey, PublicKeyCredential, RegisterPublicKeyCredential};

/// The configured passkey RP, or 503 if WebAuthn is disabled/misconfigured.
fn rp(state: &AppState) -> Result<&PasskeyState, StatusCode> {
    state
        .passkey
        .as_ref()
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)
}

/// Load a user's stored passkeys (deserialized).
pub async fn user_passkeys(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<Vec<(Uuid, Passkey)>, sqlx::Error> {
    let rows: Vec<(Uuid, Vec<u8>)> =
        sqlx::query_as("SELECT id, cred_data FROM webauthn_credentials WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, data)| {
            serde_json::from_slice::<Passkey>(&data)
                .ok()
                .map(|pk| (id, pk))
        })
        .collect())
}

pub async fn has_passkeys(pool: &sqlx::PgPool, user_id: Uuid) -> Result<bool, sqlx::Error> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM webauthn_credentials WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(pool)
            .await?;
    Ok(n > 0)
}

// ---- list / delete ----

#[derive(Serialize)]
pub struct PasskeyRow {
    id: Uuid,
    name: String,
    created_at: String,
}

/// GET /api/me/passkeys — the caller's registered passkeys (no secrets).
pub async fn list_passkeys(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<PasskeyRow>>, StatusCode> {
    let rows = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id, name, created_at::text FROM webauthn_credentials WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, created_at)| PasskeyRow {
                id,
                name,
                created_at,
            })
            .collect(),
    ))
}

/// DELETE /api/me/passkeys/:id — remove one of the caller's passkeys.
pub async fn delete_passkey(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    sqlx::query("DELETE FROM webauthn_credentials WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- registration ceremony (session-authed) ----

/// POST /api/me/passkeys/register/start — begin registering a new passkey.
pub async fn register_start(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let pk = rp(&state)?;
    let exclude = user_passkeys(&state.config, user.id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|(_, p)| p.cred_id().clone())
        .collect::<Vec<_>>();
    let (challenge, reg_state) = pk
        .webauthn
        .start_passkey_registration(user.id, &user.email, &user.email, Some(exclude))
        .map_err(|e| {
            tracing::error!(error = %e, "passkey register start");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    pk.put_reg(user.id, reg_state);
    Ok(Json(serde_json::to_value(challenge).map_err(internal)?))
}

#[derive(Deserialize)]
pub struct RegisterFinish {
    name: String,
    credential: RegisterPublicKeyCredential,
}

/// POST /api/me/passkeys/register/finish — verify + store the new passkey.
pub async fn register_finish(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<RegisterFinish>,
) -> Result<StatusCode, StatusCode> {
    let pk = rp(&state)?;
    let name = req.name.trim();
    if !valid_name(name, 64) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let reg_state = pk.take_reg(user.id).ok_or(StatusCode::BAD_REQUEST)?; // expired / no ceremony
    let passkey = pk
        .webauthn
        .finish_passkey_registration(&req.credential, &reg_state)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let cred_id = passkey.cred_id().as_ref().to_vec();
    let data = serde_json::to_vec(&passkey).map_err(internal)?;
    sqlx::query(
        "INSERT INTO webauthn_credentials (user_id, cred_id, cred_data, name) VALUES ($1, $2, $3, $4)",
    )
    .bind(user.id)
    .bind(&cred_id)
    .bind(&data)
    .bind(name)
    .execute(&state.config)
    .await
    .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- login second-factor helpers (called from auth.rs) ----

/// Start a passkey assertion for `user_id` (used during login when the password
/// already checked out). Returns the challenge JSON, or None if the user has no
/// passkeys / WebAuthn is disabled.
pub async fn start_login(
    state: &AppState,
    user_id: Uuid,
) -> Result<Option<serde_json::Value>, StatusCode> {
    let Some(pk) = state.passkey.as_ref().as_ref() else {
        return Ok(None);
    };
    let creds: Vec<Passkey> = user_passkeys(&state.config, user_id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|(_, p)| p)
        .collect();
    if creds.is_empty() {
        return Ok(None);
    }
    let (challenge, auth_state) =
        pk.webauthn
            .start_passkey_authentication(&creds)
            .map_err(|e| {
                tracing::error!(error = %e, "passkey auth start");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    pk.put_auth(user_id, auth_state);
    Ok(Some(serde_json::to_value(challenge).map_err(internal)?))
}

/// Finish a login passkey assertion. On success, updates the stored credential's
/// counter and returns true.
pub async fn finish_login(
    state: &AppState,
    user_id: Uuid,
    cred: PublicKeyCredential,
) -> Result<bool, StatusCode> {
    let Some(pk) = state.passkey.as_ref().as_ref() else {
        return Ok(false);
    };
    let Some(auth_state) = pk.take_auth(user_id) else {
        return Ok(false); // no/expired ceremony
    };
    let result = match pk
        .webauthn
        .finish_passkey_authentication(&cred, &auth_state)
    {
        Ok(r) => r,
        Err(_) => return Ok(false),
    };
    // persist the rolling counter for the matching credential
    for (id, mut stored) in user_passkeys(&state.config, user_id)
        .await
        .map_err(internal)?
    {
        if stored.cred_id() == result.cred_id() {
            if stored.update_credential(&result).is_some() {
                if let Ok(data) = serde_json::to_vec(&stored) {
                    let _ = sqlx::query(
                        "UPDATE webauthn_credentials SET cred_data = $2, last_used = now() WHERE id = $1",
                    )
                    .bind(id)
                    .bind(&data)
                    .execute(&state.config)
                    .await;
                }
            }
            break;
        }
    }
    Ok(true)
}
