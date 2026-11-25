//! Which addresses belong to the user's own networks. Addresses of other networks are not announced on the local
//! network, not taken from what mDNS reports, and not accepted as the source of a connection: this link is for devices
//! that are near each other or on a VPN of the user's.

use std::net::IpAddr;

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

/// What a source address is counted as when connections are limited per source: the address itself, with an IPv4
/// address that came on an IPv6 socket read as IPv4. An IPv6 address is not counted by its /64 prefix: only addresses
/// of the user's own networks are accepted (link-local, unique local, Tailscale's), and there one /64 is shared by every
/// device of the network, so a prefix would let one of them use up the share of all the others. The cap on all
/// connections together still bounds a source that has many addresses.
pub fn source_key(ip: IpAddr) -> IpAddr {
    ip.to_canonical()
}

#[cfg(test)]
#[path = "local_net_tests.rs"]
mod tests;
