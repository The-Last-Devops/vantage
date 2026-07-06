//! Shell/exec API: per-host shell config, the caller's **account-level** SSH key
//! library, and the step-up ticket that opens a console (docs/exec-design.md).
//! Keys belong to the account (reusable across hosts), not the server. Reads redact
//! keys; a private key is only ever accepted (to seal) or unsealed transiently.

use super::*;
use crate::console::{ExecAuth, ExecTicket};
use crate::exec_crypto;

// ---- per-host shell config --------------------------------------------------

/// Fetch the bits of a system row the shell endpoints need.
async fn system_shell_row(
    state: &AppState,
    id: Uuid,
) -> Result<(Uuid, Uuid, String, i32), StatusCode> {
    sqlx::query_as::<_, (Uuid, Uuid, String, i32)>(
        "SELECT workspace_id, key_id, hostname, ssh_port FROM systems WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.config)
    .await
    .map_err(internal)?
    .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Serialize)]
pub struct ShellStatus {
    ssh_port: i32,
    tunnel_online: bool,
    can_exec: bool,
    /// Whether the caller has any SSH keys in their library (so the UI can offer key auth).
    has_keys: bool,
}

/// GET /api/systems/:id/shell — status for the calling user (any workspace member).
pub async fn get_shell(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ShellStatus>, StatusCode> {
    let (ws, key_id, hostname, ssh_port) = system_shell_row(&state, id).await?;
    rbac::require_role(&state, &user, ws, Role::Viewer).await?;
    let can_exec = rbac::require_exec(&state, &user, ws).await.is_ok();
    let tunnel_online = state.tunnels.has(key_id, &hostname).await;
    let (key_count,): (i64,) = sqlx::query_as("SELECT count(*) FROM ssh_keys WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&state.config)
        .await
        .map_err(internal)?;
    Ok(Json(ShellStatus {
        ssh_port,
        tunnel_online,
        can_exec,
        has_keys: key_count > 0,
    }))
}

#[derive(Deserialize)]
pub struct PutShell {
    ssh_port: i32,
}

/// PUT /api/systems/:id/shell — owner sets the host's SSH port. (The shell is always
/// available; there is no enable/disable flag — see migration 0021.)
pub async fn put_shell(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<PutShell>,
) -> Result<StatusCode, StatusCode> {
    let (ws, ..) = system_shell_row(&state, id).await?;
    rbac::require_role(&state, &user, ws, Role::Owner).await?;
    if !(1..=65535).contains(&req.ssh_port) {
        return Err(StatusCode::BAD_REQUEST);
    }
    sqlx::query("UPDATE systems SET ssh_port = $2 WHERE id = $1")
        .bind(id)
        .bind(req.ssh_port)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- account-level SSH key library -----------------------------------------

#[derive(Serialize)]
pub struct SshKeyRow {
    id: Uuid,
    name: String,
    key_fingerprint: String,
    created_at: String,
}

/// GET /api/ssh-keys — the caller's own key library (redacted: no private key).
pub async fn list_ssh_keys(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<SshKeyRow>>, StatusCode> {
    let rows = sqlx::query_as::<_, (Uuid, String, String, String)>(
        "SELECT id, name, key_fingerprint, created_at::text FROM ssh_keys \
         WHERE user_id = $1 ORDER BY name",
    )
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, key_fingerprint, created_at)| SshKeyRow {
                id,
                name,
                key_fingerprint,
                created_at,
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct CreateKey {
    name: String,
    private_key: String,
    /// The caller's account password — the KEK is derived from it (step-up to unseal
    /// later uses the same password).
    password: String,
    /// Optional passphrase if the private key itself is encrypted.
    #[serde(default)]
    passphrase: Option<String>,
}

/// What we actually seal under the account-password KEK: the key PEM plus its own
/// passphrase (if encrypted), so the key works at connect regardless of format. JSON
/// so it stays forward-compatible; unseal falls back to treating bytes as a raw PEM.
#[derive(Serialize, Deserialize)]
struct SealedKey {
    pem: String,
    #[serde(default)]
    passphrase: Option<String>,
}

/// POST /api/ssh-keys — add a key to the caller's library, sealed under their password.
pub async fn create_ssh_key(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<CreateKey>,
) -> Result<Json<SshKeyRow>, (StatusCode, String)> {
    // Distinct messages so the user knows which input was wrong (the api helper now
    // surfaces the body). `bad` = 400 with a reason.
    let bad = |m: &str| (StatusCode::BAD_REQUEST, m.to_string());
    let oops = |e: sqlx::Error| {
        tracing::error!(error = %e, "ssh-key DB error");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Server error.".to_string(),
        )
    };

    let name = req.name.trim().to_string();
    if !valid_name(&name, 64) {
        return Err(bad("Give the key a name (1–64 characters)."));
    }
    if !crate::auth::verify_user_password(&state.config, user.id, &req.password)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "ssh-key password verify");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Server error.".to_string(),
            )
        })?
    {
        return Err(bad("Wrong account password."));
    }
    // Accept any format russh reads: OpenSSH, PEM RSA (PKCS#1), PKCS#8 — encrypted or not.
    let passphrase = req.passphrase.as_deref().filter(|s| !s.is_empty());
    let key = russh::keys::decode_secret_key(&req.private_key, passphrase).map_err(|_| {
        if passphrase.is_none() {
            bad(
                "Couldn't read that private key. Supported: OpenSSH, PEM RSA, or PKCS#8. \
                 If the key is encrypted, also enter its passphrase.",
            )
        } else {
            bad(
                "Couldn't unlock that key — check the key passphrase (and that it's a \
                 supported format: OpenSSH, PEM RSA, or PKCS#8).",
            )
        }
    })?;
    let fingerprint = key
        .clone_public_key()
        .map(|p| p.fingerprint())
        .map_err(|_| bad("Couldn't derive the key's fingerprint."))?;
    // Seal the PEM + its passphrase so it works at connect regardless of format.
    let sealed = serde_json::to_vec(&SealedKey {
        pem: req.private_key.clone(),
        passphrase: passphrase.map(|s| s.to_string()),
    })
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encode failed.".to_string(),
        )
    })?;
    // Seal under the user's master key (provisioned if this is their first key).
    let master =
        crate::masterkey::ensure(&state.config, &state.app_secrets, user.id, &req.password)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "master key for ssh-key seal");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Server error.".to_string(),
                )
            })?;
    let key_enc = exec_crypto::seal_with_key(&master, &sealed).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption failed.".to_string(),
        )
    })?;

    let row = sqlx::query_as::<_, (Uuid, String)>(
        "INSERT INTO ssh_keys (user_id, name, key_enc, key_fingerprint, enc_ver) \
         VALUES ($1, $2, $3, $4, 2) RETURNING id, created_at::text",
    )
    .bind(user.id)
    .bind(&name)
    .bind(&key_enc)
    .bind(&fingerprint)
    .fetch_one(&state.config)
    .await
    .map_err(|e| {
        if e.as_database_error()
            .map(|d| d.is_unique_violation())
            .unwrap_or(false)
        {
            (
                StatusCode::CONFLICT,
                "A key with that name already exists.".to_string(),
            )
        } else {
            oops(e)
        }
    })?;

    Ok(Json(SshKeyRow {
        id: row.0,
        name,
        key_fingerprint: fingerprint,
        created_at: row.1,
    }))
}

/// DELETE /api/ssh-keys/:id — remove one of the caller's keys.
pub async fn delete_ssh_key(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    sqlx::query("DELETE FROM ssh_keys WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- console ticket (step-up) ----------------------------------------------

#[derive(Deserialize)]
pub struct TicketReq {
    ssh_user: String,
    /// "password" — SSH password auth; "key" — publickey from the caller's library.
    auth: String,
    /// auth=password: the host SSH password (typed at connect, never stored).
    ssh_password: Option<String>,
    /// auth=key: the caller's account password, to unseal the chosen key.
    account_password: Option<String>,
    /// auth=key: which key from the caller's library.
    key_id: Option<Uuid>,
}

#[derive(Serialize)]
pub struct TicketResp {
    ticket: String,
}

/// POST /api/systems/:id/console/ticket — step-up: validate the chosen auth, build a
/// single-use console ticket. 400s describe the blocker.
pub async fn console_ticket(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<TicketReq>,
) -> Result<Json<TicketResp>, StatusCode> {
    let (ws, key_id, hostname, ssh_port) = system_shell_row(&state, id).await?;
    rbac::require_exec(&state, &user, ws).await?;
    // (shell is always available; access is gated by require_exec + step-up + the
    // host's own SSH auth — the per-host enable/disable toggle was removed in 0021.)
    if !state.tunnels.has(key_id, &hostname).await {
        return Err(StatusCode::BAD_REQUEST); // agent offline
    }

    let ssh_user = req.ssh_user.trim().to_string();
    // Linux usernames: 1–32 chars, no control/space/colon.
    if ssh_user.is_empty()
        || ssh_user.len() > 32
        || ssh_user
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == ':')
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    let auth = match req.auth.as_str() {
        "password" => {
            let pw = req.ssh_password.unwrap_or_default();
            if pw.is_empty() {
                return Err(StatusCode::BAD_REQUEST);
            }
            ExecAuth::Password(pw)
        }
        "key" => {
            let account_password = req.account_password.unwrap_or_default();
            let key_id_sel = req.key_id.ok_or(StatusCode::BAD_REQUEST)?;
            // The account password must be correct (it derives the KEK).
            if !crate::auth::verify_user_password(&state.config, user.id, &account_password)
                .await
                .map_err(internal)?
            {
                return Err(StatusCode::BAD_REQUEST);
            }
            let (key_enc, kdf_salt, enc_ver) =
                sqlx::query_as::<_, (Vec<u8>, Option<Vec<u8>>, i16)>(
                    "SELECT key_enc, kdf_salt, enc_ver FROM ssh_keys WHERE id = $1 AND user_id = $2",
                )
                .bind(key_id_sel)
                .bind(user.id)
                .fetch_optional(&state.config)
                .await
                .map_err(internal)?
                .ok_or(StatusCode::BAD_REQUEST)?; // no such key for this user

            let raw = if enc_ver >= 2 {
                // sealed under the user's master key
                let master = crate::masterkey::ensure(
                    &state.config,
                    &state.app_secrets,
                    user.id,
                    &account_password,
                )
                .await
                .map_err(internal)?;
                exec_crypto::open_with_key(&master, &key_enc)
                    .map_err(|_| StatusCode::BAD_REQUEST)?
            } else {
                // legacy: sealed directly under the password KEK. Open it, then transparently
                // re-seal under the master key so it's never opened the old way again.
                let salt = kdf_salt.ok_or(StatusCode::BAD_REQUEST)?;
                let raw = exec_crypto::open(&account_password, &salt, &key_enc)
                    .map_err(|_| StatusCode::BAD_REQUEST)?;
                if let Ok(master) = crate::masterkey::ensure(
                    &state.config,
                    &state.app_secrets,
                    user.id,
                    &account_password,
                )
                .await
                {
                    if let Ok(new_enc) = exec_crypto::seal_with_key(&master, &raw) {
                        let _ = sqlx::query(
                            "UPDATE ssh_keys SET key_enc = $2, enc_ver = 2, kdf_salt = NULL WHERE id = $1",
                        )
                        .bind(key_id_sel)
                        .bind(&new_enc)
                        .execute(&state.config)
                        .await;
                    }
                }
                raw
            };
            // New keys seal a JSON {pem, passphrase}; older ones sealed the raw PEM.
            match serde_json::from_slice::<SealedKey>(&raw) {
                Ok(sk) => ExecAuth::Key {
                    pem: sk.pem,
                    passphrase: sk.passphrase,
                },
                Err(_) => ExecAuth::Key {
                    pem: String::from_utf8_lossy(&raw).into_owned(),
                    passphrase: None,
                },
            }
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    let ticket = state
        .exec_tickets
        .issue(ExecTicket {
            system_id: id,
            user_id: user.id,
            user_email: user.email.clone(),
            ssh_user,
            auth,
            key_id,
            hostname,
            ssh_port: ssh_port.clamp(1, 65535) as u16,
            created: std::time::Instant::now(),
        })
        .await;
    Ok(Json(TicketResp { ticket }))
}
