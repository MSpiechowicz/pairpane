//! Which addresses are the user's own and how sources are counted.

use std::net::IpAddr;

use super::{is_local_network, is_local_peer, source_key};

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

#[test]
fn only_addresses_of_the_users_own_networks_are_local() {
    for local in [
        "10.1.2.3",
        "172.16.0.9",
        "172.31.255.1",
        "192.168.178.54",
        "169.254.10.1",
        "100.64.0.7",
        "100.127.255.254",
        "fd7a:115c:a1e0::1",
        "fc00::1",
        "fe80::1",
        "::ffff:192.168.1.5",
    ] {
        assert!(is_local_network(ip(local)), "{local}");
    }

    for global in [
        "8.8.8.8",
        "93.184.216.34",
        "100.63.255.255",
        "100.128.0.1",
        "172.32.0.1",
        "2001:db8::1",
        "2a00:1450:4001::1",
        "127.0.0.1",
        "::1",
        "::ffff:8.8.8.8",
    ] {
        assert!(!is_local_network(ip(global)), "{global}");
    }
}

#[test]
fn a_peer_may_be_on_the_local_network_a_vpn_or_this_machine_and_nowhere_else() {
    for peer in [
        "192.168.1.9",
        "100.100.100.100",
        "fd00::9",
        "fe80::1",
        "127.0.0.1",
        "::1",
        "::ffff:127.0.0.1",
    ] {
        assert!(is_local_peer(ip(peer)), "{peer}");
    }

    for stranger in [
        "8.8.8.8",
        "203.0.113.5",
        "2001:db8::1",
        "::ffff:203.0.113.5",
        "0.0.0.0",
        "::",
    ] {
        assert!(!is_local_peer(ip(stranger)), "{stranger}");
    }
}

#[test]
fn an_ipv6_source_is_counted_by_its_64_bit_prefix_and_an_ipv4_one_by_itself() {
    assert_eq!(source_key(ip("fd00:1:2:3:aaaa:bbbb:cccc:dddd")), ip("fd00:1:2:3::"));
    assert_eq!(source_key(ip("fd00:1:2:3::9")), source_key(ip("fd00:1:2:3:ffff::1")));
    assert!(source_key(ip("fd00:1:2:3::9")) != source_key(ip("fd00:1:2:4::9")));

    assert_eq!(source_key(ip("192.168.1.9")), ip("192.168.1.9"));
    assert!(source_key(ip("192.168.1.9")) != source_key(ip("192.168.1.10")));
    // An IPv4 address that arrives on an IPv6 socket is the same source as on an IPv4 one.
    assert_eq!(source_key(ip("::ffff:192.168.1.9")), ip("192.168.1.9"));
}
