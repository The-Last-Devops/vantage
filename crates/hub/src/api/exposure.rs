//! Public-exposure self-check. The hub fetches its own configured public URL at a
//! marker path that lives OUTSIDE `/pub/*`. If that request comes back 200 with the
//! marker, the hub is reachable from the internet with no auth gate in front — we then
//! advise putting it behind nginx basic-auth / Cloudflare Zero Trust (allowing `/pub/*`
//! through for agents). A gate (Access/basic-auth) returns 302/401/403 for the
//! gate-less request → "protected". See docs/exposure.md.

use super::*;
use axum::http::HeaderMap;
use std::time::Duration;

/// Marker returned by the unauthenticated, non-`/pub` probe endpoint.
const MARKER: &str = "vantage-exposure-ok";

/// GET /exposure-check — unauthenticated, returns a constant marker. Used only by the
/// exposure self-check; carries no data. NOT under `/pub`, so a correctly configured
/// gate (which only bypasses `/pub`) blocks it from the outside.
pub async fn exposure_marker() -> &'static str {
    MARKER
}

/// The hub's public base URL from env (`PUBLIC_URL`, else the first `WEBAUTHN_ORIGIN`).
fn public_url_env() -> Option<String> {
    let pick = |v: String| {
        v.split(',')
            .next()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
    };
    std::env::var("PUBLIC_URL")
        .ok()
        .and_then(pick)
        .or_else(|| std::env::var("WEBAUTHN_ORIGIN").ok().and_then(pick))
}

/// Derive the public base URL dynamically from the request the admin just made — it
/// arrived via the same proxy/gate, so its `X-Forwarded-Proto`/`-Host` (or `Host`)
/// reflect the real external URL. No env needed. Returns e.g. `https://mon.example.com`.
fn public_url_from_req(h: &HeaderMap) -> Option<String> {
    let first = |v: &str| v.split(',').next().unwrap_or(v).trim().to_string();
    let host = h
        .get("x-forwarded-host")
        .or_else(|| h.get("host"))
        .and_then(|v| v.to_str().ok())
        .map(first)
        .filter(|s| !s.is_empty())?;
    let proto = h
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(first)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http".to_string());
    Some(format!("{proto}://{host}"))
}

#[derive(Serialize)]
pub struct ExposureResult {
    configured: bool,
    public_url: Option<String>,
    exposed: Option<bool>, // None when we couldn't determine it
    status: Option<u16>,
    error: Option<String>,
}

/// POST /api/admin/exposure-check — probe our own public URL and report whether the
/// app is reachable without an auth gate. Admin-only; makes one short outbound request.
pub async fn exposure_check(
    State(_state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
) -> Result<Json<ExposureResult>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    // Prefer the URL the browser actually used (dynamic); fall back to env.
    let Some(base) = public_url_from_req(&headers).or_else(public_url_env) else {
        return Ok(Json(ExposureResult {
            configured: false,
            public_url: None,
            exposed: None,
            status: None,
            error: None,
        }));
    };
    let url = format!("{base}/exposure-check");
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none()) // a gate's login redirect = protected
        .timeout(Duration::from_secs(6))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return Ok(Json(ExposureResult {
                configured: true,
                public_url: Some(base),
                exposed: None,
                status: None,
                error: Some(e.to_string()),
            }))
        }
    };
    match client.get(&url).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            // Exposed only if the gate-less request actually reached our marker.
            let exposed = status == 200 && body.contains(MARKER);
            Ok(Json(ExposureResult {
                configured: true,
                public_url: Some(base),
                exposed: Some(exposed),
                status: Some(status),
                error: None,
            }))
        }
        Err(e) => Ok(Json(ExposureResult {
            configured: true,
            public_url: Some(base),
            exposed: None,
            status: None,
            error: Some(e.to_string()),
        })),
    }
}
