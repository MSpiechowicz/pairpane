//! What keeps the service from being abused or surprised: what is never stored, what is bounded, what a misbehaving peer
//! cannot do, and how the service ends.

use std::net::SocketAddr;
use std::time::Duration;

use iroh::SecretKey;
use iroh::endpoint_info::EndpointInfo;
use iroh_mdns_address_lookup::DiscoveryEvent;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::tests::{Node, closed_with, wrong_code};
use super::*;
use crate::pairing::{Handshake, MAX_FAILURES, MESSAGE_LEN, PAIR_VERSION, PairingEndReason, Role, TAG_LEN};
use crate::wire::Channel;

fn stranger() -> EndpointId {
    SecretKey::generate().public()
}

fn evil() -> SocketAddr {
    "192.0.2.66:9".parse().unwrap()
}

/// Tells the service that mDNS found a device at `addrs`.
fn announce(node: &Node, node_id: EndpointId, addrs: Vec<SocketAddr>) {
    let event = DiscoveryEvent::Discovered {
        endpoint_info: EndpointInfo::new(node_id).with_ip_addrs(addrs),
        last_updated: None,
    };

    discovery::record(&node.link.ctx, event);
}

async fn next_ending(events: &mut broadcast::Receiver<LinkEvent>) -> PairingEndReason {
    loop {
        let event = timeout(Duration::from_secs(10), events.recv()).await.unwrap().unwrap();
        if let LinkEvent::PairingEnded { reason } = event {
            return reason;
        }
    }
}

#[tokio::test]
async fn an_mdns_announcement_is_tried_but_never_stored() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    host.pair_with(&joiner).await;
    let stored = joiner.link.devices().unwrap()[0].addrs.clone();

    announce(&joiner, host.id(), vec![evil()]);

    assert_eq!(joiner.link.devices().unwrap()[0].addrs, stored);
    let file = std::fs::read_to_string(joiner.dir.path().join("link-devices.json")).unwrap();
    assert!(!file.contains("192.0.2.66"), "{file}");
    // It is listed and dialled, since it may be right.
    assert_eq!(joiner.link.discovered().unwrap()[0].addrs, vec![evil()]);
    assert!(
        joiner
            .link
            .ctx
            .dial(host.id(), &[])
            .ip_addrs()
            .any(|addr| *addr == evil())
    );
}

#[tokio::test]
async fn a_join_by_node_id_keeps_the_address_it_used_and_not_what_mdns_said() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    let real = host.target().addrs;
    announce(&joiner, host.id(), [real.clone(), vec![evil()]].concat());
    let offer = host.link.begin_pairing().unwrap();

    let by_id = Target {
        node_id: host.id(),
        addrs: Vec::new(),
    };
    joiner.link.join(&offer.code, &by_id).await.unwrap();

    assert_eq!(joiner.link.devices().unwrap()[0].addrs, real);
}

#[tokio::test]
async fn the_found_list_makes_room_for_new_devices_and_keeps_paired_ones() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    host.pair_with(&joiner).await;
    announce(&joiner, host.id(), vec![evil()]);

    for _ in 0..ctx::MAX_REMEMBERED + 20 {
        announce(&joiner, stranger(), vec![evil()]);
    }

    let list = joiner.link.discovered().unwrap();
    assert_eq!(list.len(), ctx::MAX_REMEMBERED);
    assert!(list.iter().any(|found| found.node_id == host.id() && found.paired));
}

#[tokio::test]
async fn remembered_addresses_are_bounded_and_the_oldest_device_makes_room() {
    let node = Node::start("node").await;
    let ctx = &node.link.ctx;
    let first = stranger();
    let many: Vec<SocketAddr> = (1..=20u16)
        .map(|port| SocketAddr::from(([10, 0, 0, 1], port)))
        .collect();

    ctx.remember(first, many.clone());
    assert_eq!(ctx.dial(first, &[]).ip_addrs().count(), crate::devices::MAX_ADDRS);

    for _ in 0..ctx::MAX_REMEMBERED {
        ctx.remember(stranger(), [evil()]);
    }

    assert_eq!(
        ctx.dial(first, &[]).ip_addrs().count(),
        0,
        "the oldest device made room"
    );
}

#[tokio::test]
async fn a_join_works_when_the_table_of_remembered_addresses_is_full() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    for _ in 0..ctx::MAX_REMEMBERED + 5 {
        joiner.link.remember_addresses(stranger(), vec![evil()]).unwrap();
    }
    let offer = host.link.begin_pairing().unwrap();

    joiner.link.join(&offer.code, &host.target()).await.unwrap();

    assert_eq!(joiner.link.devices().unwrap()[0].addrs, host.target().addrs);
    assert_eq!(
        joiner.link.ctx.dial(host.id(), &[]).ip_addrs().count(),
        host.target().addrs.len()
    );
}

#[tokio::test]
async fn unpairing_forgets_where_the_device_was() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    host.pair_with(&joiner).await;
    let mut events = host.link.subscribe().unwrap();
    host.link.remember_addresses(joiner.id(), vec![evil()]).unwrap();

    host.link.unpair(&joiner.id()).unwrap();

    assert_eq!(host.link.ctx.dial(joiner.id(), &[]).ip_addrs().count(), 0);
    assert_eq!(events.recv().await.unwrap(), LinkEvent::Unpaired(joiner.id()));
}

#[tokio::test]
async fn the_device_is_saved_even_when_the_joiner_does_not_take_the_hosts_name() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    let mut events = host.link.subscribe().unwrap();
    let offer = host.link.begin_pairing().unwrap();
    let code = PairingCode::parse(&offer.code).unwrap();

    // A joiner that does the whole exchange and then tells the host to stop sending to it.
    let conn = joiner
        .link
        .ctx
        .endpoint
        .connect(host.target().into_addr(), PAIR_ALPN)
        .await
        .unwrap();
    let handshake = Handshake::start(Role::Join, &code);
    let mut channel = Channel::open(&conn).await.unwrap();
    channel
        .send(&[&[PAIR_VERSION], handshake.message()].concat())
        .await
        .unwrap();
    let host_message = channel.recv_exact(MESSAGE_LEN).await.unwrap();
    let confirmation = handshake.finish(&host_message, &joiner.id(), &host.id()).unwrap();
    channel.send(&confirmation.tag(Role::Join)).await.unwrap();
    channel.recv_exact(TAG_LEN).await.unwrap();
    channel.stop_receiving();
    tokio::time::sleep(Duration::from_millis(300)).await;
    channel.send(b"joiner").await.unwrap();

    let device = timeout(Duration::from_secs(10), async {
        loop {
            if let LinkEvent::Paired(device) = events.recv().await.unwrap() {
                return device;
            }
        }
    })
    .await
    .expect("the pairing is announced");
    assert_eq!((device.node_id, device.name.as_str()), (joiner.id(), "joiner"));
    assert_eq!(host.link.devices().unwrap(), vec![device]);
}

#[tokio::test]
async fn a_connection_that_sends_nothing_does_not_use_up_a_try() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    let offer = host.link.begin_pairing().unwrap();

    let silent = joiner
        .link
        .ctx
        .endpoint
        .connect(host.target().into_addr(), PAIR_ALPN)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        host.link.ctx.host.start_attempt().is_none(),
        "the silent connection holds the attempt for now"
    );

    tokio::time::sleep(crate::pairing::FIRST_FRAME_TIMEOUT + Duration::from_millis(700)).await;

    let PairingState::Waiting { attempts_left, .. } = host.link.pairing_state().unwrap() else {
        panic!("the code is still active");
    };
    assert_eq!(attempts_left, MAX_FAILURES);
    joiner.link.join(&offer.code, &host.target()).await.unwrap();
    drop(silent);
}

#[tokio::test]
async fn an_attempt_that_sent_the_key_exchange_message_still_counts() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    host.link.begin_pairing().unwrap();

    let conn = joiner
        .link
        .ctx
        .endpoint
        .connect(host.target().into_addr(), PAIR_ALPN)
        .await
        .unwrap();
    let mut channel = Channel::open(&conn).await.unwrap();
    channel.send(&[PAIR_VERSION; 1 + MESSAGE_LEN]).await.unwrap();
    closed_with(&conn).await;

    let PairingState::Waiting { attempts_left, .. } = host.link.pairing_state().unwrap() else {
        panic!("the code is still active");
    };
    assert_eq!(attempts_left, MAX_FAILURES - 1);
}

#[tokio::test]
async fn a_hello_of_another_version_is_not_answered() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    host.pair_with(&joiner).await;

    let conn = joiner
        .link
        .ctx
        .endpoint
        .connect(host.target().into_addr(), LINK_ALPN)
        .await
        .unwrap();
    let mut channel = Channel::open(&conn).await.unwrap();
    channel.send(&[HELLO_VERSION + 1, b'x']).await.unwrap();

    assert!(channel.recv().await.is_err());
}

#[tokio::test]
async fn one_address_may_hold_only_a_few_connections_at_once() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    host.pair_with(&joiner).await;
    let dial = || joiner.link.ctx.endpoint.connect(host.target().into_addr(), LINK_ALPN);

    // Each of these waits for a hello that never comes.
    let mut held = Vec::new();
    for _ in 0..ctx::MAX_CONNECTIONS_PER_IP {
        held.push(dial().await.unwrap());
    }
    let fifth = timeout(Duration::from_secs(10), dial()).await.unwrap();

    assert!(fifth.is_err(), "a fifth connection from one address is refused");
    drop(held);
}

#[tokio::test]
async fn the_count_of_connections_of_an_address_goes_down_when_they_end() {
    let node = Node::start("node").await;
    let ip: std::net::IpAddr = [10, 0, 0, 7].into();

    let guards: Vec<_> = (0..ctx::MAX_CONNECTIONS_PER_IP)
        .map(|_| node.link.ctx.admit(ip).unwrap())
        .collect();
    assert!(node.link.ctx.admit(ip).is_none());
    assert!(
        node.link.ctx.admit([10, 0, 0, 8].into()).is_some(),
        "another address is not affected"
    );

    drop(guards);
    assert!(node.link.ctx.admit(ip).is_some());
}

#[tokio::test]
async fn the_end_of_a_code_is_reported_with_its_reason() {
    let host = Node::start_with("host", |config| config.pairing_ttl = Duration::from_millis(300)).await;
    let joiner = Node::start("joiner").await;
    let mut events = host.link.subscribe().unwrap();

    // Used up.
    host.pair_with(&joiner).await;
    assert_eq!(next_ending(&mut events).await, PairingEndReason::Used);

    // Cancelled, and replaced.
    host.link.begin_pairing().unwrap();
    host.link.begin_pairing().unwrap();
    assert_eq!(next_ending(&mut events).await, PairingEndReason::Cancelled);
    host.link.cancel_pairing().unwrap();
    assert_eq!(next_ending(&mut events).await, PairingEndReason::Cancelled);

    // Expired, with nobody calling anything.
    host.link.begin_pairing().unwrap();
    assert_eq!(next_ending(&mut events).await, PairingEndReason::Expired);

    // Failed too often.
    let offer = host.link.begin_pairing().unwrap();
    let wrong = wrong_code(&offer);
    for _ in 0..MAX_FAILURES {
        joiner.link.join(wrong, &host.target()).await.unwrap_err();
    }
    assert_eq!(next_ending(&mut events).await, PairingEndReason::TooManyAttempts);
}

#[tokio::test]
async fn a_second_service_on_the_same_folder_is_refused_until_the_first_ends() {
    let first = Node::start("first").await;
    let folder = || {
        let mut config = LinkConfig::new(first.dir.path(), "second");
        (config.ports, config.loopback_only, config.discovery) = (vec![0], true, Discovery::Off);
        config
    };

    let second = Link::start(folder(), KeyStores::single(crate::identity::MemoryStore::new())).await;
    assert!(matches!(second, Err(Error::AlreadyRunning)));

    first.link.shutdown().await;
    // The folder is free. What stops this start is the marker: the new memory store does not hold the key it names.
    let third = Link::start(folder(), KeyStores::single(crate::identity::MemoryStore::new())).await;
    assert!(matches!(third, Err(Error::KeyUnavailable(_))), "{:?}", third.err());
}

#[tokio::test]
async fn every_call_after_the_shutdown_is_closed() {
    let node = Node::start("node").await;
    let (id, target) = (node.id(), node.target());
    node.link.shutdown().await;
    let link = &node.link;

    assert!(matches!(link.this_device(), Err(Error::Closed)));
    assert!(matches!(link.begin_pairing(), Err(Error::Closed)));
    assert!(matches!(link.cancel_pairing(), Err(Error::Closed)));
    assert!(matches!(link.pairing_state(), Err(Error::Closed)));
    assert!(matches!(link.devices(), Err(Error::Closed)));
    assert!(matches!(link.rename_device(&id, "x"), Err(Error::Closed)));
    assert!(matches!(link.unpair(&id), Err(Error::Closed)));
    assert!(matches!(link.remember_addresses(id, Vec::new()), Err(Error::Closed)));
    assert!(matches!(link.discovered(), Err(Error::Closed)));
    assert!(matches!(link.subscribe(), Err(Error::Closed)));
    assert!(matches!(link.status(&id).await, Err(Error::Closed)));
    assert!(matches!(link.join("7K2M-9QXZ", &target).await, Err(Error::Closed)));
    // A second shutdown is harmless.
    link.shutdown().await;
}

#[tokio::test]
async fn dropping_the_service_closes_its_endpoint() {
    let node = Node::start("node").await;
    let endpoint = node.link.ctx.endpoint.clone();
    let Node { link, dir: _dir } = node;

    drop(link);

    timeout(Duration::from_secs(10), endpoint.closed())
        .await
        .expect("the endpoint is closed");
}

#[tokio::test]
async fn a_service_with_a_bad_name_does_not_start_and_takes_no_lock() {
    let dir = TempDir::new().unwrap();
    let mut config = LinkConfig::new(dir.path(), "bad\u{202E}name");
    (config.ports, config.loopback_only, config.discovery) = (vec![0], true, Discovery::Off);

    let result = Link::start(config, KeyStores::single(crate::identity::MemoryStore::new())).await;

    assert!(matches!(result, Err(Error::InvalidName)));
    assert!(!dir.path().join("link.lock").exists());
}

#[tokio::test]
async fn the_identity_marker_names_the_device_and_holds_no_secret() {
    let node = Node::start("node").await;

    let marker = std::fs::read_to_string(node.dir.path().join("link-identity.json")).unwrap();

    assert!(marker.contains(&node.id().to_string()));
    assert!(marker.contains("memory"));
}
