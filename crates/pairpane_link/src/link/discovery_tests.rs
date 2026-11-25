//! What is announced on the local network.

use std::collections::{BTreeSet, HashMap, HashSet};

use iroh::{EndpointId, SecretKey, TransportAddr};

use super::{FoundEntry, announce_filter, announced_name, make_room};

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

fn entry(seen: u64) -> FoundEntry {
    FoundEntry {
        name: None,
        addrs: BTreeSet::new(),
        seen,
    }
}

fn list_of(count: u64) -> (HashMap<EndpointId, FoundEntry>, Vec<EndpointId>) {
    let ids: Vec<EndpointId> = (0..count).map(|_| SecretKey::generate().public()).collect();
    let found = ids.iter().zip(0..).map(|(id, seen)| (*id, entry(seen))).collect();

    (found, ids)
}

#[test]
fn the_oldest_device_that_is_not_paired_makes_room_first() {
    let (mut found, ids) = list_of(4);
    // The oldest entry is paired and stays; the oldest of the others goes.
    let paired = HashSet::from([ids[0]]);

    assert!(make_room(&mut found, &paired, false));

    assert_eq!(found.len(), 3);
    assert!(found.contains_key(&ids[0]) && !found.contains_key(&ids[1]));
}

#[test]
fn a_newcomer_that_is_not_paired_does_not_push_out_a_list_of_paired_devices() {
    let (mut found, ids) = list_of(3);
    let paired: HashSet<EndpointId> = ids.iter().copied().collect();

    assert!(!make_room(&mut found, &paired, false), "there is no room for it");

    assert_eq!(found.len(), 3, "nothing was pushed out");
}

#[test]
fn a_paired_newcomer_may_take_the_place_of_the_oldest_paired_device_of_a_full_list() {
    let (mut found, ids) = list_of(3);
    let paired: HashSet<EndpointId> = ids.iter().copied().collect();

    assert!(make_room(&mut found, &paired, true));

    assert!(!found.contains_key(&ids[0]) && found.len() == 2);
}
