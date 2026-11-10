//! Cloudflare Access identity for the MCP endpoint.
//!
//! When the hub sits behind Cloudflare Access with **Managed OAuth** turned on, an
//! MCP client such as the Claude.ai custom connector signs in through Access's own
//! OAuth server and never holds a hub PAT. Access then forwards each request with
//! a signed `Cf-Access-Jwt-Assertion` header carrying the user's email. This module
//! verifies that JWT (RS256 against the team's JWKS, `iss`, `aud`, `exp`) and maps
//! the email to an **existing** hub user, so RBAC stays exactly as configured in
//! the hub.
//!
//! Scope is deliberately narrow: the JWT is honoured only on `POST /mcp` and on the
//! MCP server's in-process re-entry into the API (marked by the [`McpInProcess`]
//! request extension, which an outside caller cannot set). The browser UI keeps its
//! own login, 2FA and passkeys.
//!
//! Disabled unless both are set:
//!   `CF_ACCESS_TEAM_DOMAIN` — e.g. `myteam.cloudflareaccess.com`
//!   `CF_ACCESS_AUD`         — the Access application's AUD tag(s), comma-separated

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;

pub const JWT_HEADER: &str = "cf-access-jwt-assertion";

/// Request extension set only by the MCP server's in-process dispatch.
#[derive(Clone, Copy, Debug)]
pub struct McpInProcess;

const JWKS_TTL: Duration = Duration::from_secs(3600);
/// Minimum gap between JWKS refetches triggered by an unknown `kid`.
const REFETCH_MIN: Duration = Duration::from_secs(60);
const LEEWAY_SECS: i64 = 60;

struct Config {
    issuer: String,
    certs_url: String,
    auds: Vec<String>,
}

#[derive(Default)]
struct Keys {
    fetched: Option<Instant>,
    by_kid: HashMap<String, (Vec<u8>, Vec<u8>)>, // kid -> (n, e)
}

static CONFIG: OnceLock<Option<Config>> = OnceLock::new();
static KEYS: LazyLock<Mutex<Keys>> = LazyLock::new(Mutex::default);
static HTTP: OnceLock<reqwest::Client> = OnceLock::new();

fn config() -> Option<&'static Config> {
    CONFIG
        .get_or_init(|| {
            let team = std::env::var("CF_ACCESS_TEAM_DOMAIN").ok()?;
            let team = team
                .trim()
                .trim_start_matches("https://")
                .trim_end_matches('/')
                .to_string();
            let auds: Vec<String> = std::env::var("CF_ACCESS_AUD")
                .ok()?
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if team.is_empty() || auds.is_empty() {
                return None;
            }
            tracing::info!(team = %team, auds = auds.len(), "Cloudflare Access JWT enabled for /mcp");
            Some(Config {
                issuer: format!("https://{team}"),
                certs_url: format!("https://{team}/cdn-cgi/access/certs"),
                auds,
            })
        })
        .as_ref()
}

pub fn enabled() -> bool {
    config().is_some()
}

#[derive(Deserialize)]
struct Header {
    alg: String,
    kid: Option<String>,
}

#[derive(Deserialize)]
struct Claims {
    #[serde(default)]
    aud: Aud,
    #[serde(default)]
    iss: String,
    exp: i64,
    #[serde(default)]
    nbf: Option<i64>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(untagged)]
enum Aud {
    One(String),
    Many(Vec<String>),
    #[default]
    None,
}

impl Aud {
    fn contains_any(&self, wanted: &[String]) -> bool {
        match self {
            Aud::One(a) => wanted.iter().any(|w| w == a),
            Aud::Many(v) => v.iter().any(|a| wanted.iter().any(|w| w == a)),
            Aud::None => false,
        }
    }
}

#[derive(Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
struct Jwk {
    kid: String,
    kty: String,
    #[serde(default)]
    n: String,
    #[serde(default)]
    e: String,
}

fn cached_key(kid: &str) -> (Option<(Vec<u8>, Vec<u8>)>, bool, bool) {
    let k = KEYS.lock().unwrap_or_else(|p| p.into_inner());
    let stale = k.fetched.is_none_or(|t| t.elapsed() > JWKS_TTL);
    let may_refetch = k.fetched.is_none_or(|t| t.elapsed() > REFETCH_MIN);
    (k.by_kid.get(kid).cloned(), stale, may_refetch)
}

async fn refresh(cfg: &Config) {
    let client = HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default()
    });
    let jwks: Jwks = match client.get(&cfg.certs_url).send().await {
        Ok(r) => match r.error_for_status() {
            Ok(r) => match r.json().await {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!(error = %e, "cf access: bad JWKS body");
                    return;
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "cf access: JWKS fetch failed");
                return;
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, "cf access: JWKS fetch failed");
            return;
        }
    };
    let mut by_kid = HashMap::new();
    for k in jwks.keys.into_iter().filter(|k| k.kty == "RSA") {
        if let (Ok(n), Ok(e)) = (URL_SAFE_NO_PAD.decode(&k.n), URL_SAFE_NO_PAD.decode(&k.e)) {
            by_kid.insert(k.kid, (n, e));
        }
    }
    let mut keys = KEYS.lock().unwrap_or_else(|p| p.into_inner());
    keys.by_kid = by_kid;
    keys.fetched = Some(Instant::now());
}

async fn key_for(cfg: &Config, kid: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let (found, stale, may_refetch) = cached_key(kid);
    if found.is_some() && !stale {
        return found;
    }
    if stale || may_refetch {
        refresh(cfg).await;
        return cached_key(kid).0;
    }
    found
}

/// Verify a Cloudflare Access JWT and return the (lower-cased) email it carries.
/// `None` for anything invalid, for service-token JWTs (no email), or when the
/// feature is disabled.
pub async fn verify(token: &str) -> Option<String> {
    let cfg = config()?;
    let kid = header_kid(token)?;
    let (n, e) = key_for(cfg, &kid).await?;
    let claims = verify_signature(token, &n, &e)?;
    check_claims(cfg, &claims, chrono::Utc::now().timestamp())
}

/// The `kid` of an RS256 JWT; `None` for any other `alg` (no `none`/HS256 confusion).
fn header_kid(token: &str) -> Option<String> {
    let h64 = token.split('.').next()?;
    let header: Header = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(h64).ok()?).ok()?;
    if header.alg != "RS256" {
        return None;
    }
    header.kid
}

/// Check the RS256 signature against the RSA key `(n, e)` and decode the claims.
fn verify_signature(token: &str, n: &[u8], e: &[u8]) -> Option<Claims> {
    let mut parts = token.split('.');
    let (h64, c64, s64) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let sig = URL_SAFE_NO_PAD.decode(s64).ok()?;
    let signed = &token[..h64.len() + 1 + c64.len()];
    ring::signature::RsaPublicKeyComponents { n, e }
        .verify(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            signed.as_bytes(),
            &sig,
        )
        .ok()?;
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(c64).ok()?).ok()
}

fn check_claims(cfg: &Config, c: &Claims, now: i64) -> Option<String> {
    if c.iss != cfg.issuer || !c.aud.contains_any(&cfg.auds) {
        return None;
    }
    if c.exp + LEEWAY_SECS < now || c.nbf.is_some_and(|nbf| nbf - LEEWAY_SECS > now) {
        return None;
    }
    c.email
        .as_deref()
        .map(str::trim)
        .filter(|e| e.contains('@'))
        .map(str::to_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config {
            issuer: "https://team.cloudflareaccess.com".into(),
            certs_url: String::new(),
            auds: vec!["aud1".into(), "aud2".into()],
        }
    }

    fn claims(aud: Aud, iss: &str, exp: i64, email: Option<&str>) -> Claims {
        Claims {
            aud,
            iss: iss.into(),
            exp,
            nbf: None,
            email: email.map(Into::into),
        }
    }

    #[test]
    fn accepts_valid_claims() {
        let c = claims(
            Aud::Many(vec!["x".into(), "aud2".into()]),
            "https://team.cloudflareaccess.com",
            1_000,
            Some("A@Example.com"),
        );
        assert_eq!(
            check_claims(&cfg(), &c, 900).as_deref(),
            Some("a@example.com")
        );
    }

    #[test]
    fn rejects_wrong_aud_iss_expired_or_no_email() {
        let iss = "https://team.cloudflareaccess.com";
        let now = 1_000;
        assert!(check_claims(
            &cfg(),
            &claims(Aud::One("other".into()), iss, 2_000, Some("a@b.c")),
            now
        )
        .is_none());
        assert!(check_claims(&cfg(), &claims(Aud::None, iss, 2_000, Some("a@b.c")), now).is_none());
        assert!(check_claims(
            &cfg(),
            &claims(
                Aud::One("aud1".into()),
                "https://evil",
                2_000,
                Some("a@b.c")
            ),
            now
        )
        .is_none());
        assert!(check_claims(
            &cfg(),
            &claims(Aud::One("aud1".into()), iss, now - 61, Some("a@b.c")),
            now
        )
        .is_none());
        // service-token JWTs carry no email
        assert!(check_claims(
            &cfg(),
            &claims(Aud::One("aud1".into()), iss, 2_000, None),
            now
        )
        .is_none());
    }

    #[test]
    fn aud_deserialises_string_or_array() {
        let one: Claims = serde_json::from_str(r#"{"aud":"aud1","iss":"i","exp":1}"#).unwrap();
        assert!(one.aud.contains_any(&["aud1".into()]));
        let many: Claims =
            serde_json::from_str(r#"{"aud":["z","aud1"],"iss":"i","exp":1}"#).unwrap();
        assert!(many.aud.contains_any(&["aud1".into()]));
    }

    // --- signature: a throwaway RSA-2048 key generated for these tests only
    // (`openssl genpkey -algorithm RSA`, PKCS#8 DER, base64). It signs nothing real.
    const TEST_KEY_B64: &str = include_str!("testdata/cf_access_test_rsa.pk8.b64");

    fn key() -> ring::signature::RsaKeyPair {
        use base64::engine::general_purpose::STANDARD;
        let der = STANDARD.decode(TEST_KEY_B64.trim()).unwrap();
        ring::signature::RsaKeyPair::from_pkcs8(&der).unwrap()
    }

    fn pub_ne(k: &ring::signature::RsaKeyPair) -> (Vec<u8>, Vec<u8>) {
        let c = ring::rsa::PublicKeyComponents::<Vec<u8>>::from(k.public());
        (c.n, c.e)
    }

    fn sign(k: &ring::signature::RsaKeyPair, header: &str, claims: &str) -> String {
        let msg = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header),
            URL_SAFE_NO_PAD.encode(claims)
        );
        let mut sig = vec![0u8; k.public().modulus_len()];
        k.sign(
            &ring::signature::RSA_PKCS1_SHA256,
            &ring::rand::SystemRandom::new(),
            msg.as_bytes(),
            &mut sig,
        )
        .unwrap();
        format!("{msg}.{}", URL_SAFE_NO_PAD.encode(sig))
    }

    const HDR: &str = r#"{"alg":"RS256","kid":"k1","typ":"JWT"}"#;
    const CLM: &str = r#"{"aud":["aud1"],"iss":"https://team.cloudflareaccess.com","exp":2000,"email":"Kien@Example.com"}"#;

    #[test]
    fn valid_signature_round_trips_to_email() {
        let k = key();
        let (n, e) = pub_ne(&k);
        let tok = sign(&k, HDR, CLM);
        assert_eq!(header_kid(&tok).as_deref(), Some("k1"));
        let c = verify_signature(&tok, &n, &e).expect("signature should verify");
        assert_eq!(
            check_claims(&cfg(), &c, 1_000).as_deref(),
            Some("kien@example.com")
        );
    }

    #[test]
    fn tampered_or_foreign_signature_is_rejected() {
        let k = key();
        let (n, e) = pub_ne(&k);
        let tok = sign(&k, HDR, CLM);
        // swap in different claims, keep the original signature
        let mut parts: Vec<&str> = tok.split('.').collect();
        let forged = URL_SAFE_NO_PAD.encode(CLM.replace("Kien@", "admin@"));
        parts[1] = &forged;
        assert!(verify_signature(&parts.join("."), &n, &e).is_none());
        // right token, wrong key
        let mut other_n = n.clone();
        *other_n.last_mut().unwrap() ^= 0x02;
        assert!(verify_signature(&tok, &other_n, &e).is_none());
        // malformed
        assert!(verify_signature("a.b", &n, &e).is_none());
        assert!(verify_signature(&format!("{tok}.x"), &n, &e).is_none());
    }

    #[test]
    fn only_rs256_is_accepted() {
        let k = key();
        for h in [
            r#"{"alg":"none","kid":"k1"}"#,
            r#"{"alg":"HS256","kid":"k1"}"#,
            r#"{"alg":"RS512","kid":"k1"}"#,
        ] {
            assert!(header_kid(&sign(&k, h, CLM)).is_none(), "{h}");
        }
        // RS256 without a kid can't pick a key
        assert!(header_kid(&sign(&k, r#"{"alg":"RS256"}"#, CLM)).is_none());
    }

    #[test]
    fn nbf_in_the_future_is_rejected() {
        let iss = "https://team.cloudflareaccess.com";
        let mut c = claims(Aud::One("aud1".into()), iss, 5_000, Some("a@b.c"));
        c.nbf = Some(1_100);
        assert!(check_claims(&cfg(), &c, 1_000).is_none());
        c.nbf = Some(1_050); // inside the 60s leeway
        assert!(check_claims(&cfg(), &c, 1_000).is_some());
        // expired, but within leeway
        let c = claims(Aud::One("aud1".into()), iss, 950, Some("a@b.c"));
        assert!(check_claims(&cfg(), &c, 1_000).is_some());
    }
}
