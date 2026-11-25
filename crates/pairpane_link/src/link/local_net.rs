//! Which addresses belong to the user's own networks. Addresses of other networks are not announced on the local
//! network, not taken from what mDNS reports, and not accepted as the source of a connection: this link is for devices
//! that are near each other or on a VPN of the user's.

use std::net::{IpAddr, Ipv6Addr};

/// Whether an address is on a network of the user's own: a private IPv4 range, the shared address space of Tailscale and
/// carrier-grade NAT (100.64.0.0/10), link-local, or an IPv6 unique local (fc00::/7) or link-local (fe80::/10) address.
/// Global addresses are not announced to the local network.
pub fn is_local_network(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => {
            ip.is_private() || ip.is_link_local() || (ip.octets()[0] == 100 && ip.octets()[1] & 0xC0 == 64)
        }
        IpAddr::V6(ip) => (ip.segments()[0] & 0xFE00) == 0xFC00 || (ip.segments()[0] & 0xFFC0) == 0xFE80,
    }
}

/// Whether a peer at this address may be talked to: it is on one of the user's own networks, or on this machine
/// (loopback, which is also what the tests use).
pub fn is_local_peer(ip: IpAddr) -> bool {
    is_local_network(ip) || ip.to_canonical().is_loopback()
}

/// What a source address is counted as when connections are limited per source: the address itself for IPv4, and for IPv6
/// its /64 prefix, because one host usually owns a whole /64 and can change the rest of the address at will.
pub fn source_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(ip) => {
            let [a, b, c, d, ..] = ip.segments();
            IpAddr::V6(Ipv6Addr::new(a, b, c, d, 0, 0, 0, 0))
        }
        ip => ip,
    }
}

#[cfg(test)]
#[path = "local_net_tests.rs"]
mod tests;
