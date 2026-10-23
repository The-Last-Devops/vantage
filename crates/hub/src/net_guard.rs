//! SSRF egress guard. The hub makes outbound requests to user/operator-supplied
//! targets — service probes, notification webhooks, and the S3 backup endpoint.
//! Before connecting we resolve the host and reject internal / metadata
//! destinations, so an editor can't point a probe or webhook at the cloud
//! metadata endpoint (169.254.169.254), loopback, or other reserved ranges and
//! exfiltrate internal data (the probe response body is readable via the debug
//! endpoint).
//!
//! Policy:
//! - **Never allowed**: loopback, link-local (incl. metadata 169.254.169.254 /
//!   fe80::), unspecified, broadcast, multicast, and the Alibaba metadata IP
//!   100.100.100.200 — plus IPv4-mapped IPv6 forms of all of these. Nothing,
//!   not even `EGRESS_ALLOW_CIDRS`, opens these.
//! - **Blocked by default**: documentation ranges.
//! - **Private (RFC1918 / ULA / CGNAT 100.64/10)**: allowed by default, because
//!   monitoring internal hosts is the product's whole job — and CGNAT is where a
//!   Tailscale tailnet lives. Set `EGRESS_POLICY=strict` to block these too
//!   (public destinations only) for hardened/multi-tenant deployments.
//! - **`EGRESS_ALLOW_CIDRS`** (comma-separated, e.g. `100.64.0.0/10`): an IP
//!   inside a listed CIDR skips the default-blocked and strict-private checks
//!   above — but never the "never allowed" ones. Mainly for re-opening one range
//!   under `EGRESS_POLICY=strict`. An unparsable entry is ignored (fails closed).
//!
//! We resolve the host and check **every** resolved IP (a host that resolves to
//! any blocked address is rejected), which also defeats DNS-rebinding to an
//! internal IP. Redirects are re-checked at each hop (see the probe HTTP client).

use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::sync::OnceLock;

fn strict() -> bool {
    matches!(std::env::var("EGRESS_POLICY").as_deref(), Ok("strict"))
}

fn is_cgnat(o: [u8; 4]) -> bool {
    o[0] == 100 && (64..=127).contains(&o[1]) // 100.64.0.0/10
}

/// Unwrap IPv4-mapped IPv6 (::ffff:a.b.c.d) so it can't bypass the v4 checks.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(a) => a.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

/// Never a legitimate outbound target — no allowlist can open these.
fn blocked_hard(ip: IpAddr) -> bool {
    match canonical(ip) {
        IpAddr::V4(a) => {
            a.is_loopback()
                || a.is_link_local() // 169.254/16, includes the 169.254.169.254 metadata IP
                || a.is_unspecified()
                || a.is_broadcast()
                || a.is_multicast()
                || a == Ipv4Addr::new(100, 100, 100, 200) // Alibaba metadata, inside CGNAT
        }
        IpAddr::V6(a) => {
            a.is_loopback()
                || a.is_unspecified()
                || a.is_multicast()
                || (a.segments()[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        }
    }
}

/// Reserved ranges blocked unless the operator lists them in `EGRESS_ALLOW_CIDRS`.
fn blocked_default(ip: IpAddr) -> bool {
    match canonical(ip) {
        IpAddr::V4(a) => a.is_documentation(),
        IpAddr::V6(_) => false,
    }
}

/// RFC1918 / ULA / CGNAT — allowed by default, blocked only under `EGRESS_POLICY=strict`.
fn is_private(ip: IpAddr) -> bool {
    match canonical(ip) {
        IpAddr::V4(a) => a.is_private() || is_cgnat(a.octets()),
        IpAddr::V6(a) => (a.segments()[0] & 0xfe00) == 0xfc00, // ULA fc00::/7
    }
}

/// One `EGRESS_ALLOW_CIDRS` entry, stored as (network, prefix length).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Cidr {
    V4(u32, u8),
    V6(u128, u8),
}

impl Cidr {
    fn parse(s: &str) -> Option<Cidr> {
        let (addr, len) = match s.split_once('/') {
            Some((a, l)) => (a, Some(l.parse::<u8>().ok()?)),
            None => (s, None),
        };
        match addr.parse::<IpAddr>().ok()? {
            IpAddr::V4(a) => {
                let len = len.unwrap_or(32);
                (len <= 32).then(|| Cidr::V4(u32::from(a) & mask32(len), len))
            }
            IpAddr::V6(a) => {
                // A mapped v4 range (::ffff:a.b.c.d/n) is matched as plain v4.
                if let Some(v4) = a.to_ipv4_mapped() {
                    let len = len.unwrap_or(128).checked_sub(96)?;
                    return Some(Cidr::V4(u32::from(v4) & mask32(len), len));
                }
                let len = len.unwrap_or(128);
                (len <= 128).then(|| Cidr::V6(u128::from(a) & mask128(len), len))
            }
        }
    }

    fn contains(self, ip: IpAddr) -> bool {
        match (self, canonical(ip)) {
            (Cidr::V4(net, len), IpAddr::V4(a)) => u32::from(a) & mask32(len) == net,
            (Cidr::V6(net, len), IpAddr::V6(a)) => u128::from(a) & mask128(len) == net,
            _ => false,
        }
    }
}

fn mask32(len: u8) -> u32 {
    u32::MAX.checked_shl(32 - u32::from(len)).unwrap_or(0)
}

fn mask128(len: u8) -> u128 {
    u128::MAX.checked_shl(128 - u32::from(len)).unwrap_or(0)
}

/// Parse a comma-separated CIDR list; unparsable entries are dropped (fail closed).
fn parse_allow(list: &str) -> Vec<Cidr> {
    list.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| {
            let c = Cidr::parse(s);
            if c.is_none() {
                tracing::warn!("EGRESS_ALLOW_CIDRS: ignoring invalid entry {s:?}");
            }
            c
        })
        .collect()
}

/// `EGRESS_ALLOW_CIDRS`, parsed once (process env doesn't change at runtime).
fn allow_cidrs() -> &'static [Cidr] {
    static ALLOW: OnceLock<Vec<Cidr>> = OnceLock::new();
    ALLOW.get_or_init(|| {
        std::env::var("EGRESS_ALLOW_CIDRS")
            .map(|v| parse_allow(&v))
            .unwrap_or_default()
    })
}

/// The policy as a pure function of its inputs (testable without env vars).
fn check_ip_with(ip: IpAddr, strict: bool, allow: &[Cidr]) -> anyhow::Result<()> {
    if blocked_hard(ip) {
        anyhow::bail!("egress blocked: {ip} is a reserved/internal/metadata address");
    }
    if allow.iter().any(|c| c.contains(ip)) {
        return Ok(());
    }
    if blocked_default(ip) {
        anyhow::bail!("egress blocked: {ip} is a reserved/internal/metadata address");
    }
    if strict && is_private(ip) {
        anyhow::bail!("egress blocked (EGRESS_POLICY=strict): {ip} is a private/CGNAT address");
    }
    Ok(())
}

fn check_ip(ip: IpAddr) -> anyhow::Result<()> {
    check_ip_with(ip, strict(), allow_cidrs())
}

/// Resolve `host`:`port` and reject if **any** resolved IP is disallowed.
pub fn check_host(host: &str, port: u16) -> anyhow::Result<()> {
    // A bare IP literal still resolves here (getaddrinfo accepts it).
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|e| anyhow::anyhow!("cannot resolve {host}: {e}"))?;
    let mut any = false;
    for a in addrs {
        any = true;
        check_ip(a.ip())?;
    }
    if !any {
        anyhow::bail!("{host} did not resolve to any address");
    }
    Ok(())
}

/// Guard an outbound target given as a URL (`scheme://host[:port]/…`) or a bare
/// `host:port` (TCP probes). Resolves the host and applies [`check_host`].
pub fn check_target(target: &str) -> anyhow::Result<()> {
    if target.contains("://") {
        let u = reqwest::Url::parse(target).map_err(|e| anyhow::anyhow!("bad URL: {e}"))?;
        let host = u
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("target has no host"))?;
        let port = u.port_or_known_default().unwrap_or(0);
        check_host(host, port)
    } else {
        // host:port (or host) — resolve directly.
        let addrs = target
            .to_socket_addrs()
            .or_else(|_| (target, 0u16).to_socket_addrs())
            .map_err(|e| anyhow::anyhow!("cannot resolve {target}: {e}"))?;
        let mut any = false;
        for a in addrs {
            any = true;
            check_ip(a.ip())?;
        }
        if !any {
            anyhow::bail!("{target} did not resolve");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_loopback_and_metadata_and_linklocal() {
        assert!(check_host("127.0.0.1", 80).is_err());
        assert!(check_host("169.254.169.254", 80).is_err()); // cloud metadata
        assert!(check_host("::1", 80).is_err());
        assert!(check_target("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(check_target("http://localhost:8080/").is_err());
    }

    #[test]
    fn cgnat_and_mapped_blocked() {
        assert!(check_host("100.100.100.200", 80).is_err()); // Alibaba metadata (CGNAT)
        assert!(check_host("::ffff:127.0.0.1", 80).is_err()); // mapped loopback
    }

    #[test]
    fn private_allowed_by_default_blocked_in_strict() {
        // default policy: private is reachable (monitoring internal hosts)
        assert!(check_ip("10.0.0.5".parse().unwrap()).is_ok());
        assert!(check_ip("192.168.1.10".parse().unwrap()).is_ok());
        // public is always fine
        assert!(check_ip("1.1.1.1".parse().unwrap()).is_ok());
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn cgnat_allowed_by_default_blocked_in_strict() {
        // Tailscale tailnet addresses are reachable out of the box…
        assert!(check_ip_with(ip("100.126.99.64"), false, &[]).is_ok());
        assert!(check_ip_with(ip("::ffff:100.126.99.64"), false, &[]).is_ok());
        // …but not under strict, unless allow-listed.
        assert!(check_ip_with(ip("100.126.99.64"), true, &[]).is_err());
        assert!(check_ip_with(ip("::ffff:100.126.99.64"), true, &[]).is_err());
        // Alibaba metadata sits inside CGNAT and is always blocked.
        assert!(check_ip_with(ip("100.100.100.200"), false, &[]).is_err());
    }

    #[test]
    fn allowlist_opens_tailnet_under_strict() {
        let allow = parse_allow("100.64.0.0/10");
        assert_eq!(allow.len(), 1);
        assert!(check_ip_with(ip("100.126.99.64"), true, &allow).is_ok());
        assert!(check_ip_with(ip("100.71.247.99"), true, &allow).is_ok());
        // IPv4-mapped IPv6 is handled exactly like the v4 address.
        assert!(check_ip_with(ip("::ffff:100.126.99.64"), true, &allow).is_ok());
        // Outside the listed range the default policy still applies.
        assert!(check_ip_with(ip("192.0.2.1"), false, &allow).is_err()); // documentation
    }

    #[test]
    fn allowlist_never_opens_dangerous_ranges() {
        let allow = parse_allow("100.64.0.0/10, 169.254.0.0/16, 127.0.0.0/8, 0.0.0.0/0, ::/0");
        for bad in [
            "100.100.100.200",
            "::ffff:100.100.100.200",
            "169.254.169.254",
            "::ffff:169.254.169.254",
            "127.0.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "::1",
            "::",
            "fe80::1",
            "ff02::1",
        ] {
            assert!(
                check_ip_with(ip(bad), false, &allow).is_err(),
                "{bad} must stay blocked"
            );
        }
    }

    #[test]
    fn allowlist_overrides_strict_private() {
        let allow = parse_allow("10.20.0.0/16");
        assert!(check_ip_with(ip("10.20.1.1"), true, &allow).is_ok());
        assert!(check_ip_with(ip("10.21.1.1"), true, &allow).is_err());
        assert!(check_ip_with(ip("10.21.1.1"), false, &allow).is_ok());
    }

    #[test]
    fn parses_cidrs_and_drops_garbage() {
        assert_eq!(parse_allow(""), vec![]);
        assert_eq!(parse_allow("nope, 1.2.3.4/33, 1.2.3.4/x"), vec![]);
        assert_eq!(
            parse_allow("100.64.0.0/10"),
            vec![Cidr::V4(0x6440_0000, 10)]
        );
        assert_eq!(
            parse_allow("100.126.99.64"),
            vec![Cidr::V4(0x647e_6340, 32)]
        );
        assert!(parse_allow("fd7a:115c:a1e0::/48")[0].contains(ip("fd7a:115c:a1e0::1")));
        assert!(parse_allow("::ffff:100.64.0.0/106")[0].contains(ip("100.126.99.64")));
    }

    #[test]
    fn classifies_private_ranges() {
        assert!(is_private("10.1.2.3".parse().unwrap()));
        assert!(is_private("172.16.0.1".parse().unwrap()));
        assert!(is_private("192.168.0.1".parse().unwrap()));
        assert!(!is_private("8.8.8.8".parse().unwrap()));
    }
}
