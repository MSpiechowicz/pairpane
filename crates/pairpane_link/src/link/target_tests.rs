//! The text form of a device to connect to.

use std::net::SocketAddr;

use iroh::SecretKey;
use pretty_assertions::assert_eq;

use super::{MAX_TARGET_ADDRS, Target};
use crate::error::Error;

#[test]
fn a_target_with_addresses_reads_back_as_it_was_shown() {
    let node_id = SecretKey::generate().public();
    let target = Target {
        node_id,
        addrs: vec!["100.64.0.7:47620".parse().unwrap(), "[fd7a::1]:47620".parse().unwrap()],
    };

    let text = target.to_string();

    assert_eq!(text, format!("{node_id}@100.64.0.7:47620,[fd7a::1]:47620"));
    assert_eq!(text.parse::<Target>().unwrap(), target);
}

#[test]
fn a_node_id_alone_is_a_target_without_addresses() {
    let node_id = SecretKey::generate().public();

    let target: Target = format!("  {node_id}  ").parse().unwrap();

    assert_eq!(
        target,
        Target {
            node_id,
            addrs: Vec::new()
        }
    );
    assert_eq!(target.to_string(), node_id.to_string());
}

#[test]
fn spaces_around_the_parts_are_ignored() {
    let node_id = SecretKey::generate().public();

    let target: Target = format!("{node_id} @ 10.0.0.2:1000 , 10.0.0.3:1000").parse().unwrap();

    let expected: Vec<SocketAddr> = vec!["10.0.0.2:1000".parse().unwrap(), "10.0.0.3:1000".parse().unwrap()];
    assert_eq!(target.addrs, expected);
}

#[test]
fn text_that_is_not_a_target_is_refused() {
    let node_id = SecretKey::generate().public();
    let many = vec!["10.0.0.2:1"; MAX_TARGET_ADDRS + 1].join(",");

    for text in [
        String::new(),
        "not a node id".to_owned(),
        "abcd".to_owned(),
        format!("{node_id}@"),
        format!("{node_id}@10.0.0.2"),
        format!("{node_id}@10.0.0.2:1,"),
        format!("{node_id}@host.example:1"),
        format!("{node_id}@{many}"),
    ] {
        assert!(matches!(text.parse::<Target>(), Err(Error::InvalidTarget)), "{text:?}");
    }
}
