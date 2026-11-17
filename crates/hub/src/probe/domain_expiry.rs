//! `domain_expiry` — when does a domain's REGISTRATION run out?
//!
//! Not the TLS certificate (that is the `tls` kind): a lapsed registration takes the
//! whole domain away — the registrar parks it, every name under it stops resolving to
//! us, and whoever picks it up receives whatever our clients keep sending there.
//!
//! Lookup order:
//! 1. **RDAP** — the registry's JSON API, found through IANA's bootstrap file
//!    (`data.iana.org/rdap/dns.json`, cached for a day). Covers gTLDs and many ccTLDs.
//! 2. **WHOIS** (port 43) for TLDs without RDAP: ask `whois.iana.org` which server
//!    holds the TLD, fall back to [`WHOIS_FALLBACK`] for registries IANA lists without
//!    one (e.g. `.lc`), then `whois.nic.<tld>`. If the registry answer carries no
//!    expiry but names a registrar WHOIS server, that one is asked too.
//!
//! The target itself is never contacted — only RDAP/WHOIS servers are, and each of
//! them goes through the SSRF guard first, because the registrar WHOIS server name
//! comes out of a text answer we do not control.
//!
//! Config: `warn_days` (default 14) — Down once the registration has that many days
//! left or fewer; `timeout_secs` (default 20); `whois_server` — skip discovery and
//! ask this server directly.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, Semaphore};
use tokio::time::timeout;

use super::{cfg_str, cfg_u64, err_beat, Beat, Monitor, DEFAULT_UA};

const BOOTSTRAP_URL: &str = "https://data.iana.org/rdap/dns.json";
const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);
/// Registries are touchy about WHOIS rate: after a restart every domain check fires
/// at once, so only a few lookups run at a time.
const MAX_CONCURRENT: usize = 4;
const WHOIS_CAP: u64 = 64 * 1024;
const RDAP_CAP: usize = 512 * 1024;

/// Registry WHOIS servers for TLDs whose IANA record has no `whois:` line.
/// Verified by hand — add one only after a real lookup returned an expiry date.
const WHOIS_FALLBACK: &[(&str, &str)] = &[("lc", "whois.afilias-grs.info")];

/// Expiry field names seen across registries/registrars, lowercase.
const EXPIRY_KEYS: &[&str] = &[
    "registry expiry date",
    "registrar registration expiration date",
    "expiration date",
    "expiry date",
    "expire date",
    "expires on",
    "expires",
    "expiration time",
    "paid-till",
    "renewal date",
];

#[derive(Default)]
struct Cache {
    rdap: Option<(Instant, HashMap<String, String>)>,
    whois: HashMap<String, (Instant, Option<String>)>,
}

fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Cache::default()))
}

fn limiter() -> &'static Semaphore {
    static S: OnceLock<Semaphore> = OnceLock::new();
    S.get_or_init(|| Semaphore::new(MAX_CONCURRENT))
}

/// A registrable-looking domain name: lowercase, no trailing dot, 2+ labels of
/// `[a-z0-9-]` (punycode allowed), no URL, port or path. `None` = not a domain.
pub fn normalize_domain(s: &str) -> Option<String> {
    let d = s.trim().trim_end_matches('.').to_ascii_lowercase();
    if d.is_empty() || d.len() > 253 {
        return None;
    }
    let labels: Vec<&str> = d.split('.').collect();
    if labels.len() < 2 {
        return None;
    }
    let ok = labels.iter().all(|l| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    });
    // The TLD is never all digits — that would be an IP address.
    let tld_ok = !labels[labels.len() - 1].bytes().all(|b| b.is_ascii_digit());
    (ok && tld_ok).then_some(d)
}

struct Found {
    expires: DateTime<Utc>,
    source: String,
}

pub(super) async fn probe_domain_expiry(m: &Monitor, start: Instant) -> Beat {
    let Some(domain) = normalize_domain(&m.target) else {
        return err_beat(start, &m.target, "target is not a domain name".into());
    };
    let warn = cfg_u64(&m.config, "warn_days", 14) as i64;
    let to = cfg_u64(&m.config, "timeout_secs", 20).clamp(1, 60);
    let server = cfg_str(&m.config, "whois_server").map(str::to_owned);

    let Ok(_permit) = limiter().acquire().await else {
        return err_beat(start, &domain, "lookup limiter closed".into());
    };
    let found = match timeout(Duration::from_secs(to), lookup(&domain, server)).await {
        Ok(Ok(f)) => f,
        Ok(Err(e)) => return err_beat(start, &domain, e.to_string()),
        Err(_) => return err_beat(start, &domain, "lookup timeout".into()),
    };
    let days = (found.expires - Utc::now()).num_days();
    let date = found.expires.format("%Y-%m-%d");
    let msg = if days < 0 {
        format!(
            "domain EXPIRED {} days ago ({date}, {})",
            -days, found.source
        )
    } else {
        format!("domain expires in {days} days ({date}, {})", found.source)
    };
    Beat {
        up: days > warn,
        latency_ms: Some(start.elapsed().as_millis() as i32),
        status_code: Some(days as i32),
        message: Some(msg),
        debug: Some(json!({
            "target": domain, "expires": found.expires.to_rfc3339(),
            "days_left": days, "warn_days": warn, "source": found.source,
        })),
    }
}

async fn lookup(domain: &str, server: Option<String>) -> anyhow::Result<Found> {
    if let Some(s) = server {
        return whois_from(domain, &s).await;
    }
    let tld = domain.rsplit('.').next().unwrap_or_default().to_string();
    let mut rdap_err = None;
    if let Some(base) = rdap_base(&tld).await {
        match rdap(&base, domain).await {
            Ok(f) => return Ok(f),
            // A registry with RDAP may still answer over WHOIS — try that before failing.
            Err(e) => rdap_err = Some(e),
        }
    }
    let server = match registry_whois(&tld).await {
        Some(s) => s,
        None => {
            return Err(rdap_err
                .unwrap_or_else(|| anyhow::anyhow!("no RDAP or WHOIS server known for .{tld}")))
        }
    };
    whois_from(domain, &server)
        .await
        .map_err(|e| match rdap_err {
            Some(r) => anyhow::anyhow!("rdap: {r}; whois: {e}"),
            None => e,
        })
}

// ---- RDAP -------------------------------------------------------------------

fn http() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(DEFAULT_UA)
        // Re-check every hop: a redirect must not bounce us to an internal address.
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 3 {
                attempt.stop()
            } else if crate::net_guard::check_target(attempt.url().as_str()).is_err() {
                attempt.error("blocked redirect to internal/metadata address".to_string())
            } else {
                attempt.follow()
            }
        }))
        .build()?)
}

async fn get_capped(url: &str, accept: &str, cap: usize) -> anyhow::Result<(u16, Vec<u8>)> {
    let u = url.to_string();
    tokio::task::spawn_blocking(move || crate::net_guard::check_target(&u)).await??;
    let mut resp = http()?.get(url).header("accept", accept).send().await?;
    let status = resp.status().as_u16();
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        body.extend_from_slice(&chunk);
        if body.len() > cap {
            anyhow::bail!("response from {url} larger than {cap} bytes");
        }
    }
    Ok((status, body))
}

/// TLD → RDAP base URL from IANA's bootstrap file, refreshed daily. A failed
/// refresh keeps the previous map (stale beats none).
async fn rdap_base(tld: &str) -> Option<String> {
    let mut c = cache().lock().await;
    let fresh = c
        .rdap
        .as_ref()
        .is_some_and(|(t, _)| t.elapsed() < CACHE_TTL);
    if !fresh {
        match get_capped(BOOTSTRAP_URL, "application/json", 2 * 1024 * 1024).await {
            Ok((200, body)) => match serde_json::from_slice::<Value>(&body) {
                Ok(v) => c.rdap = Some((Instant::now(), parse_bootstrap(&v))),
                Err(e) => tracing::warn!(error = %e, "rdap bootstrap: bad json"),
            },
            Ok((s, _)) => tracing::warn!(status = s, "rdap bootstrap fetch"),
            Err(e) => tracing::warn!(error = %e, "rdap bootstrap fetch"),
        }
    }
    c.rdap.as_ref().and_then(|(_, m)| m.get(tld).cloned())
}

fn parse_bootstrap(v: &Value) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for svc in v["services"].as_array().into_iter().flatten() {
        let (Some(tlds), Some(urls)) = (svc[0].as_array(), svc[1].as_array()) else {
            continue;
        };
        // Prefer an https base when several are listed.
        let base = urls
            .iter()
            .filter_map(|u| u.as_str())
            .find(|u| u.starts_with("https://"))
            .or_else(|| urls.iter().filter_map(|u| u.as_str()).next());
        if let Some(b) = base {
            for t in tlds.iter().filter_map(|t| t.as_str()) {
                map.insert(t.to_ascii_lowercase(), b.to_string());
            }
        }
    }
    map
}

async fn rdap(base: &str, domain: &str) -> anyhow::Result<Found> {
    let url = format!("{}/domain/{domain}", base.trim_end_matches('/'));
    let (status, body) = get_capped(&url, "application/rdap+json", RDAP_CAP).await?;
    if status == 404 {
        anyhow::bail!("rdap: {domain} not found at registry (unregistered?)");
    }
    if status != 200 {
        anyhow::bail!("rdap: HTTP {status} from {url}");
    }
    let v: Value = serde_json::from_slice(&body)?;
    let expires = rdap_expiry(&v).ok_or_else(|| anyhow::anyhow!("rdap: no expiration event"))?;
    let host = reqwest::Url::parse(&url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    Ok(Found {
        expires,
        source: format!("rdap {host}"),
    })
}

fn rdap_expiry(v: &Value) -> Option<DateTime<Utc>> {
    v["events"]
        .as_array()?
        .iter()
        .find(|e| e["eventAction"].as_str() == Some("expiration"))
        .and_then(|e| e["eventDate"].as_str())
        .and_then(parse_date)
}

// ---- WHOIS ------------------------------------------------------------------

/// The registry WHOIS server for a TLD: IANA's record, then the hand-verified
/// fallback table, then the `whois.nic.<tld>` convention. Cached daily.
async fn registry_whois(tld: &str) -> Option<String> {
    {
        let c = cache().lock().await;
        if let Some((t, s)) = c.whois.get(tld) {
            if t.elapsed() < CACHE_TTL {
                return s.clone();
            }
        }
    }
    let from_iana = match whois_query("whois.iana.org", tld).await {
        Ok(txt) => field(&txt, "whois"),
        Err(e) => {
            tracing::warn!(error = %e, tld, "whois.iana.org");
            None
        }
    };
    let server = from_iana
        .or_else(|| {
            WHOIS_FALLBACK
                .iter()
                .find(|(t, _)| *t == tld)
                .map(|(_, s)| s.to_string())
        })
        .or_else(|| Some(format!("whois.nic.{tld}")));
    cache()
        .lock()
        .await
        .whois
        .insert(tld.to_string(), (Instant::now(), server.clone()));
    server
}

async fn whois_from(domain: &str, server: &str) -> anyhow::Result<Found> {
    let txt = whois_query(server, domain).await?;
    if let Some(expires) = parse_expiry(&txt) {
        return Ok(Found {
            expires,
            source: format!("whois {server}"),
        });
    }
    // Thin registries only point at the registrar; ask it once.
    if let Some(reg) = field(&txt, "registrar whois server") {
        let reg = reg
            .trim_start_matches("whois://")
            .trim_end_matches('/')
            .to_string();
        if !reg.eq_ignore_ascii_case(server) {
            let txt2 = whois_query(&reg, domain).await?;
            if let Some(expires) = parse_expiry(&txt2) {
                return Ok(Found {
                    expires,
                    source: format!("whois {reg}"),
                });
            }
        }
    }
    if looks_unregistered(&txt) {
        anyhow::bail!("whois {server}: {domain} not found (unregistered?)");
    }
    anyhow::bail!("whois {server}: no expiry date in answer")
}

async fn whois_query(server: &str, query: &str) -> anyhow::Result<String> {
    // The server name may come from a registry's text answer: accept a hostname only.
    if normalize_domain(server).is_none() {
        anyhow::bail!("invalid WHOIS server name");
    }
    let target = format!("{server}:43");
    tokio::task::spawn_blocking({
        let t = target.clone();
        move || crate::net_guard::check_target(&t)
    })
    .await??;
    let mut s = tokio::net::TcpStream::connect(&target).await?;
    s.write_all(format!("{query}\r\n").as_bytes()).await?;
    let mut buf = Vec::new();
    s.take(WHOIS_CAP).read_to_end(&mut buf).await?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// First `key: value` line whose key matches (case-insensitive).
fn field(txt: &str, key: &str) -> Option<String> {
    txt.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        let v = v.trim();
        (k.trim().eq_ignore_ascii_case(key) && !v.is_empty()).then(|| v.to_string())
    })
}

fn parse_expiry(txt: &str) -> Option<DateTime<Utc>> {
    for key in EXPIRY_KEYS {
        for l in txt.lines() {
            let Some((k, v)) = l.split_once(':') else {
                continue;
            };
            if k.trim().eq_ignore_ascii_case(key) {
                if let Some(d) = parse_date(v.trim()) {
                    return Some(d);
                }
            }
        }
    }
    None
}

fn looks_unregistered(txt: &str) -> bool {
    let t = txt.to_ascii_lowercase();
    [
        "no match",
        "not found",
        "no data found",
        "no entries found",
        "status: free",
    ]
    .iter()
    .any(|p| t.contains(p))
}

/// Dates as registries actually write them: RFC 3339 (with or without
/// fractions), `YYYY-MM-DD hh:mm:ss`, or a bare date in a handful of layouts.
fn parse_date(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some(d.with_timezone(&Utc));
    }
    for f in [
        "%Y-%m-%dT%H:%M:%S%.fZ",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
    ] {
        if let Ok(d) = NaiveDateTime::parse_from_str(s, f) {
            return Some(d.and_utc());
        }
    }
    let first = s.split_whitespace().next()?;
    for f in [
        "%Y-%m-%d", "%d-%b-%Y", "%Y.%m.%d", "%d.%m.%Y", "%Y/%m/%d", "%d/%m/%Y",
    ] {
        if let Ok(d) = NaiveDate::parse_from_str(first, f) {
            return d.and_hms_opt(0, 0, 0).map(|d| d.and_utc());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_names_are_validated_and_normalised() {
        assert_eq!(normalize_domain(" Coo.LC. ").as_deref(), Some("coo.lc"));
        assert_eq!(
            normalize_domain("xn--bcher-kva.de").as_deref(),
            Some("xn--bcher-kva.de")
        );
        for bad in [
            "",
            "localhost",
            "https://coo.lc",
            "coo.lc/path",
            "coo.lc:443",
            "a..b",
            "-a.com",
            "a-.com",
            "1.2.3.4",
            "has space.com",
        ] {
            assert!(normalize_domain(bad).is_none(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn whois_expiry_from_registry_and_registrar_answers() {
        // Identity Digital (.lc) registry answer.
        let registry = "Domain Name: coo.lc\r\nRegistrar WHOIS Server: whois.dynadot.com\r\n\
                        Updated Date: 2026-11-16T16:10:01Z\r\nRegistry Expiry Date: 2027-11-13T13:21:12Z\r\n";
        assert_eq!(
            parse_expiry(registry).unwrap().to_rfc3339(),
            "2027-11-13T13:21:12+00:00"
        );
        // Dynadot writes fractional seconds.
        let registrar = "Registrar Registration Expiration Date: 2026-11-13T13:21:12.0Z\n";
        assert_eq!(
            parse_expiry(registrar).unwrap().to_rfc3339(),
            "2026-11-13T13:21:12+00:00"
        );
        // Nominet-style and RU-style.
        assert!(parse_expiry("    Expiry date:  13-Nov-2027\n").is_some());
        assert!(parse_expiry("paid-till:     2027.03.01\n").is_some());
        // An "Updated Date" alone is not an expiry.
        assert!(parse_expiry("Updated Date: 2026-11-16T16:10:01Z\n").is_none());
        assert_eq!(
            field(registry, "registrar whois server").as_deref(),
            Some("whois.dynadot.com")
        );
    }

    #[test]
    fn rdap_expiration_event_and_bootstrap() {
        let v = json!({ "events": [
            { "eventAction": "registration", "eventDate": "2019-01-01T00:00:00Z" },
            { "eventAction": "expiration", "eventDate": "2027-01-02T03:04:05Z" } ] });
        assert_eq!(
            rdap_expiry(&v).unwrap().to_rfc3339(),
            "2027-01-02T03:04:05+00:00"
        );
        assert!(rdap_expiry(&json!({ "events": [] })).is_none());

        let b = json!({ "services": [
            [["com", "net"], ["http://x.example/", "https://rdap.verisign.com/com/v1/"]],
            [["xyz"], ["https://rdap.centralnic.com/xyz/"]] ] });
        let m = parse_bootstrap(&b);
        assert_eq!(m["net"], "https://rdap.verisign.com/com/v1/");
        assert_eq!(m["xyz"], "https://rdap.centralnic.com/xyz/");
    }

    /// Real lookups against live registries — `cargo test -p vantage-hub live_lookup -- --ignored`.
    /// Covers both paths: RDAP (.com/.dev/.xyz) and WHOIS-only TLDs (.lc via the fallback
    /// table, .us via whois.nic.us).
    #[tokio::test]
    #[ignore = "network"]
    async fn live_lookup() {
        for d in [
            "senprints.com",
            "senhub.dev",
            "senprints.xyz",
            "coo.lc",
            "s3n.us",
        ] {
            let f = lookup(d, None).await.unwrap_or_else(|e| panic!("{d}: {e}"));
            let days = (f.expires - Utc::now()).num_days();
            println!(
                "{d}: {} ({days} days, {})",
                f.expires.format("%Y-%m-%d"),
                f.source
            );
            assert!(days > -3650 && days < 3650 * 2, "{d}: implausible {days}");
        }
    }
}
