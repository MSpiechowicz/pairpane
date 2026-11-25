//! What is announced on the local network.

use std::net::IpAddr;

use iroh::TransportAddr;

use super::{announce_filter, announced_name, is_local_network};

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

#[test]
fn only_addresses_of_the_users_own_networks_are_announced() {
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
    ] {
        assert!(!is_local_network(ip(global)), "{global}");
    }
}

#[test]
fn the_filter_drops_global_addresses_and_relays() {
    let addrs = vec![
        TransportAddr::Ip("192.168.1.5:47620".parse().unwrap()),
        TransportAddr::Ip("203.0.113.9:47620".parse().unwrap()),
        TransportAddr::Ip("[2001:db8::1]:47620".parse().unwrap()),
        TransportAddr::Ip("[fd00::5]:47620".parse().unwrap()),
        TransportAddr::Relay("https://relay.example./".parse().unwrap()),
    ];

    let kept = announce_filter().apply(&addrs);

    assert_eq!(
        *kept,
        vec![
            TransportAddr::Ip("192.168.1.5:47620".parse().unwrap()),
            TransportAddr::Ip("[fd00::5]:47620".parse().unwrap()),
        ]
    );
}

#[test]
fn a_long_name_is_cut_to_what_an_announcement_can_hold() {
    let name = "é".repeat(64);

    let data = announced_name(&name).unwrap().to_string();

    assert!(data.len() <= 245 && name.starts_with(&data));
}
