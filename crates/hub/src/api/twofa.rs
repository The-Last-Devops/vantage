//! TOTP two-factor auth endpoints (opt-in, per user). The verification maths lives
//! in `crate::totp` (RFC 6238, unit-tested); this wires enrollment + the login check
//! to the DB. The shared secret is sealed under the application secret so a DB leak
//! can't mint codes. See docs/auth-2fa-passkey.md.

use super::*;
use crate::totp;
use rand::RngCore;
use sha2::{Digest, Sha256};

const ISSUER: &str = "Vantage";
const SECRET_LEN: usize = 20; // 160-bit, the RFC 6238 recommendation
const BACKUP_COUNT: usize = 10;

// ---- seal/open the secret under the app secret (same scheme as the master key) ----

fn seal_secret(state: &AppState, secret: &[u8]) -> Result<(Vec<u8>, Option<String>), StatusCode> {
    match state.app_secrets.current.as_ref() {
        Some(app) => app
            .seal(secret)
            .map(|b| (b, Some(app.kid.clone())))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR),
        None => Ok((secret.to_vec(), None)),
    }
}

fn open_secret(state: &AppState, blob: &[u8], kid: Option<&str>) -> Option<Vec<u8>> {
    match kid {
        Some(k) => state.app_secrets.find(k)?.open(blob).ok(),
        None => Some(blob.to_vec()),
    }
}

fn hash_code(code: &str) -> String {
    let norm: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    hex::encode(Sha256::digest(norm.to_lowercase().as_bytes()))
}

fn now_secs() -> u64 {
    chrono::Utc::now().timestamp().max(0) as u64
}

// ---- status ----

#[derive(Serialize)]
pub struct TwoFaStatus {
    enabled: bool,
    pending: bool, // a secret was generated but not yet verified
    backup_codes_remaining: usize,
}

/// GET /api/me/2fa — the caller's 2FA state.
pub async fn twofa_status(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<TwoFaStatus>, StatusCode> {
    let row: Option<(bool, Option<Vec<u8>>, Option<String>)> = sqlx::query_as(
        "SELECT totp_enabled, totp_secret_enc, totp_backup_codes FROM users WHERE id = $1",
    )
    .bind(user.id)
    .fetch_optional(&state.config)
    .await
    .map_err(internal)?;
    let (enabled, secret, backups) = row.ok_or(StatusCode::NOT_FOUND)?;
    let remaining = backups
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .map(|v| v.len())
        .unwrap_or(0);
    Ok(Json(TwoFaStatus {
        enabled,
        pending: !enabled && secret.is_some(),
        backup_codes_remaining: remaining,
    }))
}

// ---- start enrollment ----

#[derive(Serialize)]
pub struct TwoFaStart {
    secret: String,      // base32, for manual entry
    otpauth_uri: String, // for a QR / "add account" link
    qr_svg: String,      // the otpauth URI rendered as a scannable QR (inline SVG)
}

/// Render `data` as a QR code → a self-contained SVG string (black on white).
fn qr_svg(data: &str) -> String {
    let Ok(code) = qrcode::QrCode::new(data.as_bytes()) else {
        return String::new();
    };
    let w = code.width();
    let colors = code.to_colors();
    let (quiet, scale) = (4usize, 4usize); // quiet-zone modules, px per module
    let dim = (w + 2 * quiet) * scale;
    let mut rects = String::new();
    for y in 0..w {
        for x in 0..w {
            if colors[y * w + x] == qrcode::Color::Dark {
                let (px, py) = ((x + quiet) * scale, (y + quiet) * scale);
                rects.push_str(&format!(
                    "<rect x='{px}' y='{py}' width='{scale}' height='{scale}'/>"
                ));
            }
        }
    }
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{dim}' height='{dim}' viewBox='0 0 {dim} {dim}' shape-rendering='crispEdges'><rect width='{dim}' height='{dim}' fill='#fff'/><g fill='#000'>{rects}</g></svg>"
    )
}

/// POST /api/me/2fa/start — generate a fresh (pending) secret. Overwrites any earlier
/// pending secret. Refuses if 2FA is already enabled (disable first).
pub async fn twofa_start(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<TwoFaStart>, StatusCode> {
    let (enabled,): (bool,) = sqlx::query_as("SELECT totp_enabled FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_one(&state.config)
        .await
        .map_err(internal)?;
    if enabled {
        return Err(StatusCode::CONFLICT);
    }
    let mut secret = vec![0u8; SECRET_LEN];
    rand::thread_rng().fill_bytes(&mut secret);
    let b32 = totp::base32_encode(&secret);
    let uri = totp::provisioning_uri(&b32, &user.email, ISSUER);
    let (enc, kid) = seal_secret(&state, &secret)?;
    sqlx::query(
        "UPDATE users SET totp_secret_enc = $2, totp_kid = $3, totp_enabled = false, \
         totp_backup_codes = NULL WHERE id = $1",
    )
    .bind(user.id)
    .bind(&enc)
    .bind(kid.as_deref())
    .execute(&state.config)
    .await
    .map_err(internal)?;
    let qr = qr_svg(&uri);
    Ok(Json(TwoFaStart {
        secret: b32,
        otpauth_uri: uri,
        qr_svg: qr,
    }))
}

// ---- enable (verify the first code) ----

#[derive(Deserialize)]
pub struct CodeReq {
    code: String,
}

#[derive(Serialize)]
pub struct EnableResp {
    backup_codes: Vec<String>,
}

/// POST /api/me/2fa/enable — verify a code against the pending secret; on success turn
/// 2FA on and return one-time backup codes (shown once).
pub async fn twofa_enable(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<CodeReq>,
) -> Result<Json<EnableResp>, StatusCode> {
    let (enabled, enc, kid): (bool, Option<Vec<u8>>, Option<String>) =
        sqlx::query_as("SELECT totp_enabled, totp_secret_enc, totp_kid FROM users WHERE id = $1")
            .bind(user.id)
            .fetch_one(&state.config)
            .await
            .map_err(internal)?;
    if enabled {
        return Err(StatusCode::CONFLICT);
    }
    let secret = enc
        .and_then(|b| open_secret(&state, &b, kid.as_deref()))
        .ok_or(StatusCode::BAD_REQUEST)?; // no pending secret (call /start first)
    if !totp::verify(&secret, &req.code, now_secs()) {
        return Err(StatusCode::BAD_REQUEST); // wrong code
    }
    // mint backup codes (e.g. "a1b2c3d4") and store only their hashes
    let mut codes = Vec::with_capacity(BACKUP_COUNT);
    let mut hashes = Vec::with_capacity(BACKUP_COUNT);
    for _ in 0..BACKUP_COUNT {
        let mut b = [0u8; 5];
        rand::thread_rng().fill_bytes(&mut b);
        let code = hex::encode(b); // 10 hex chars
        hashes.push(hash_code(&code));
        codes.push(format!("{}-{}", &code[..5], &code[5..]));
    }
    let hashes_json = serde_json::to_string(&hashes).map_err(internal)?;
    sqlx::query("UPDATE users SET totp_enabled = true, totp_backup_codes = $2 WHERE id = $1")
        .bind(user.id)
        .bind(&hashes_json)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(Json(EnableResp {
        backup_codes: codes,
    }))
}

// ---- disable ----

#[derive(Deserialize)]
pub struct DisableReq {
    password: String,
}

/// POST /api/me/2fa/disable — turn 2FA off (requires the account password).
pub async fn twofa_disable(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<DisableReq>,
) -> Result<StatusCode, StatusCode> {
    if !crate::auth::verify_user_password(&state.config, user.id, &req.password)
        .await
        .map_err(internal)?
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    sqlx::query(
        "UPDATE users SET totp_enabled = false, totp_secret_enc = NULL, totp_kid = NULL, \
         totp_backup_codes = NULL WHERE id = $1",
    )
    .bind(user.id)
    .execute(&state.config)
    .await
    .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- login-time verification (called from auth::login) ----

/// Whether `user_id` has 2FA enabled.
pub async fn is_enabled(pool: &sqlx::PgPool, user_id: Uuid) -> Result<bool, sqlx::Error> {
    let (e,): (bool,) = sqlx::query_as("SELECT totp_enabled FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    Ok(e)
}

/// Verify a login `code` for `user_id`: a current TOTP code, or an unused backup code
/// (which is then consumed). Returns Ok(true) on success.
pub async fn verify_login(state: &AppState, user_id: Uuid, code: &str) -> Result<bool, StatusCode> {
    let (enc, kid, backups): (Option<Vec<u8>>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT totp_secret_enc, totp_kid, totp_backup_codes FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&state.config)
    .await
    .map_err(internal)?;

    // 1) a live TOTP code
    if let Some(secret) = enc.and_then(|b| open_secret(state, &b, kid.as_deref())) {
        if totp::verify(&secret, code, now_secs()) {
            return Ok(true);
        }
    }
    // 2) a one-time backup code — consume it on match
    if let Some(list) = backups.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()) {
        let want = hash_code(code);
        if let Some(pos) = list.iter().position(|h| h == &want) {
            let mut remaining = list;
            remaining.remove(pos);
            let json = serde_json::to_string(&remaining).map_err(internal)?;
            sqlx::query("UPDATE users SET totp_backup_codes = $2 WHERE id = $1")
                .bind(user_id)
                .bind(&json)
                .execute(&state.config)
                .await
                .map_err(internal)?;
            return Ok(true);
        }
    }
    Ok(false)
}
