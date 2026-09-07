use std::net::{IpAddr, SocketAddr};

use crate::error::{AppError, AppResult};

/// Resolve a host, and refuse anything that is not a public address.
///
/// This is the guard that stops the server browser from being an SSRF tool.
/// A row in the browser is data the API handed us, but a plugin's query
/// template is data a third party wrote, and either could name `127.0.0.1`,
/// `169.254.169.254` or an RFC1918 host. Probing those would let a remote party
/// map the user's LAN — or reach a cloud metadata endpoint — with the app as
/// the sensor.
///
/// **Resolution happens here, once, and the resolved address is what gets
/// connected to.** Checking a hostname and then connecting by name again would
/// be a DNS-rebinding hole: the second lookup can return a different answer.
/// Every caller in this crate therefore takes the `SocketAddr` from here rather
/// than passing a string to `connect`.
pub async fn resolve_public(host: &str, port: u16) -> AppResult<SocketAddr> {
    if host.is_empty() || host.len() > 253 {
        return Err(AppError::invalid("Bad host."));
    }

    if port == 0 {
        return Err(AppError::invalid("Bad port."));
    }

    let mut addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| AppError::Network(format!("Could not resolve {host}.")))?;

    addrs
        .find(|a| is_public(&a.ip()))
        .ok_or_else(|| AppError::jail(format!("{host} does not resolve to a public address.")))
}

pub fn is_public(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();

            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                // 100.64.0.0/10 — carrier-grade NAT, and Tailscale's range.
                || (o[0] == 100 && (64..128).contains(&o[1]))
                /*
                 * 224.0.0.0/4. A multicast destination is not one host, it is
                 * every listener on the segment — so a single datagram sent to
                 * 239.255.255.250 (SSDP) or 224.0.0.1 (all-hosts) is exactly
                 * the LAN sweep this guard exists to prevent, and it is one
                 * packet rather than a scan anybody would notice.
                 */
                || v4.is_multicast()
                // 0.0.0.0/8, and 240.0.0.0/4 (reserved, includes broadcast).
                || o[0] == 0
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();

            !(v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7 unique-local and fe80::/10 link-local.
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                // ff00::/8. The v4 reasoning applies unchanged: ff02::1 is
                // every node on the link.
                || v6.is_multicast()
                // IPv4-mapped: re-check the embedded address rather than
                // letting ::ffff:127.0.0.1 through as "a v6 address".
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| !is_public(&IpAddr::V4(v4)))
                /*
                 * IPv4-COMPATIBLE (::a.b.c.d), the deprecated sibling of the
                 * mapped form above. `to_ipv4_mapped` returns None for it, so
                 * without this ::127.0.0.1 reaches loopback through a stack
                 * that still honours the format. The first six groups being
                 * zero is the whole test; ::/128 and ::1 are already refused
                 * above, so what remains is a real embedded address.
                 */
                || (segments[..6].iter().all(|g| *g == 0)
                    && !is_public(&IpAddr::V4(std::net::Ipv4Addr::new(
                        (segments[6] >> 8) as u8,
                        segments[6] as u8,
                        (segments[7] >> 8) as u8,
                        segments[7] as u8,
                    )))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_and_special_ranges_are_refused() {
        for bad in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.5",
            "172.16.4.4",
            "169.254.169.254",
            "100.64.1.1",
            "0.0.0.0",
            "255.255.255.255",
            "240.0.0.1",
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            // Multicast: one datagram, every listener on the segment.
            "224.0.0.1",
            "239.255.255.250",
            "ff02::1",
            "ff02::c",
            // IPv4-compatible, the deprecated sibling of ::ffff:.
            "::127.0.0.1",
            "::192.168.1.1",
        ] {
            let ip: IpAddr = bad.parse().expect(bad);
            assert!(!is_public(&ip), "{bad} should be refused");
        }
    }

    #[test]
    fn ordinary_public_addresses_pass() {
        for good in [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.216.34",
            "172.32.0.1",
            "100.128.0.1",
            "2606:4700::1111",
        ] {
            let ip: IpAddr = good.parse().expect(good);
            assert!(is_public(&ip), "{good} should be allowed");
        }
    }
}
