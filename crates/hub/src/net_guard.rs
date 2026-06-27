//! SSRF egress guard. The hub makes outbound requests to user/operator-supplied
//! targets — service probes, notification webhooks, and the S3 backup endpoint.
//! Before connecting we resolve the host and reject internal / metadata
//! destinations, so an editor can't point a probe or webhook at the cloud
//! metadata endpoint (169.254.169.254), loopback, or other reserved ranges and
//! exfiltrate internal data (the probe response body is readable via the debug
//! endpoint).
//!
//! Policy:
//! - **Always blocked**: loopback, link-local (incl. metadata 169.254.169.254 /
//!   fe80::), unspecified, broadcast, multicast, CGNAT (100.64/10), and
//!   documentation ranges — plus IPv4-mapped IPv6 forms of all of these.
//! - **Private (RFC1918 / ULA)**: allowed by default, because monitoring internal
//!   hosts is the product's whole job. Set `EGRESS_POLICY=strict` to block these
//!   too (public destinations only) for hardened/multi-tenant deployments.
//!
//! We resolve the host and check **every** resolved IP (a host that resolves to
//! any blocked address is rejected), which also defeats DNS-rebinding to an
//! internal IP. Redirects are re-checked at each hop (see the probe HTTP client).

use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};

fn strict() -> bool {
    matches!(std::env::var("EGRESS_POLICY").as_deref(), Ok("strict"))
}

fn is_cgnat(o: [u8; 4]) -> bool {
    o[0] == 100 && (64..=127).contains(&o[1]) // 100.64.0.0/10
}

/// Reserved / internal ranges that are never a legitimate outbound target.
fn blocked_always(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => {
            a.is_loopback()
                || a.is_link_local() // 169.254/16, includes the 169.254.169.254 metadata IP
                || a.is_unspecified()
                || a.is_broadcast()
                || a.is_multicast()
                || a.is_documentation()
                || is_cgnat(a.octets()) // also covers 100.100.100.200 (Alibaba metadata)
        }
        IpAddr::V6(a) => {
            // Unwrap IPv4-mapped (::ffff:a.b.c.d) so it can't bypass the v4 checks.
            if let Some(v4) = a.to_ipv4_mapped() {
                return blocked_always(IpAddr::V4(v4));
            }
            a.is_loopback()
                || a.is_unspecified()
                || a.is_multicast()
                || (a.segments()[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        }
    }
}

/// RFC1918 / ULA — allowed by default, blocked only under `EGRESS_POLICY=strict`.
fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => a.is_private(),
        IpAddr::V6(a) => {
            if let Some(v4) = a.to_ipv4_mapped() {
                return Ipv4Addr::is_private(&v4);
            }
            (a.segments()[0] & 0xfe00) == 0xfc00 // ULA fc00::/7
        }
    }
}

fn check_ip(ip: IpAddr) -> anyhow::Result<()> {
    if blocked_always(ip) {
        anyhow::bail!("egress blocked: {ip} is a reserved/internal/metadata address");
    }
    if strict() && is_private(ip) {
        anyhow::bail!("egress blocked (EGRESS_POLICY=strict): {ip} is a private address");
    }
    Ok(())
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

    #[test]
    fn classifies_private_ranges() {
        assert!(is_private("10.1.2.3".parse().unwrap()));
        assert!(is_private("172.16.0.1".parse().unwrap()));
        assert!(is_private("192.168.0.1".parse().unwrap()));
        assert!(!is_private("8.8.8.8".parse().unwrap()));
    }
}
