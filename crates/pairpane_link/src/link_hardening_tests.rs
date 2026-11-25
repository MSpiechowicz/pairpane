//! What keeps the service from being abused or surprised: what is never stored, what is bounded, what a misbehaving peer
//! cannot do, and how the service ends.

use std::net::SocketAddr;
use std::time::Duration;

use iroh::SecretKey;
use iroh::endpoint_info::EndpointInfo;
use iroh_mdns_address_lookup::DiscoveryEvent;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::tests::Node;
use super::*;
use crate::wire::Channel;

fn stranger() -> EndpointId {
    SecretKey::generate().public()
}

/// An address that is on a private network, as an address that a hostile device on the local network would announce.
fn evil() -> SocketAddr {
    "192.168.250.66:9".parse().unwrap()
}

/// Tells the service that mDNS found a device at `addrs`.
fn announce(node: &Node, node_id: EndpointId, addrs: Vec<SocketAddr>) {
    let event = DiscoveryEvent::Discovered {
        endpoint_info: EndpointInfo::new(node_id).with_ip_addrs(addrs),
        last_updated: None,
    };

    discovery::record(&node.link.ctx, event);
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
    assert!(!file.contains("192.168.250.66"), "{file}");
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
async fn a_join_by_a_found_device_keeps_the_address_it_used_and_not_what_mdns_said() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    let real = host.target().addrs;
    announce(&joiner, host.id(), [real.clone(), vec![evil()]].concat());
    let offer = host.link.begin_pairing().unwrap();

    let found = joiner.link.discovered().unwrap().remove(0);
    let by_id = Target::from(&found);
    assert!(by_id.addrs.is_empty(), "the target of a found device has no addresses");
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

#[tokio::test]
async fn a_found_device_lists_only_addresses_of_the_users_networks_and_at_most_eight() {
    let node = Node::start("node").await;
    let (many, global_only, mixed) = (stranger(), stranger(), stranger());
    let private: Vec<SocketAddr> = (1..=12u16)
        .map(|port| SocketAddr::from(([192, 168, 1, 7], port)))
        .collect();
    let global: Vec<SocketAddr> = vec![
        "203.0.113.9:47620".parse().unwrap(),
        "[2001:db8::1]:47620".parse().unwrap(),
    ];

    announce(&node, many, private.clone());
    announce(&node, global_only, global.clone());
    announce(&node, mixed, [global.clone(), vec![evil()]].concat());

    let list = node.link.discovered().unwrap();
    let addrs_of = |id: EndpointId| list.iter().find(|found| found.node_id == id).unwrap().addrs.clone();
    assert_eq!(addrs_of(many).len(), crate::devices::MAX_ADDRS);
    assert!(addrs_of(many).iter().all(|addr| private.contains(addr)));
    assert!(
        addrs_of(global_only).is_empty(),
        "the device is listed, with no address to dial"
    );
    assert_eq!(addrs_of(mixed), vec![evil()]);
    assert!(
        node.link.ctx.dial(global_only, &[]).ip_addrs().next().is_none(),
        "an address of another network is not dialled either"
    );
}

#[tokio::test]
async fn addresses_given_before_pairing_are_kept_when_the_join_succeeds() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    let typed_earlier: SocketAddr = "10.9.8.7:1234".parse().unwrap();
    joiner.link.remember_addresses(host.id(), vec![typed_earlier]).unwrap();
    let offer = host.link.begin_pairing().unwrap();

    joiner.link.join(&offer.code, &host.target()).await.unwrap();

    let used = host.target().addrs;
    assert_eq!(
        joiner.link.devices().unwrap()[0].addrs,
        [used, vec![typed_earlier]].concat()
    );
}

#[tokio::test]
async fn pairing_and_remembering_addresses_at_once_leave_no_stale_memory_behind() {
    let node = Node::start("node").await;
    let ctx = &node.link.ctx;
    let remembered: SocketAddr = "10.9.8.7:1234".parse().unwrap();

    for _ in 0..50 {
        let id = stranger();
        std::thread::scope(|scope| {
            scope.spawn(|| ctx.remember(id, [remembered]));
            scope.spawn(|| ctx.add_device(id, "device", &[]).map(drop).unwrap());
        });

        // Whichever came first, the address ended in the device list, and nothing is left in memory for a device that
        // is paired (it would be dialled for ever, and unpairing would not know to forget it).
        assert_eq!(ctx.devices().get(&id).unwrap().addrs, vec![remembered]);
        assert!(!ctx.has_remembered(&id));
    }
}

#[tokio::test]
async fn nothing_is_written_to_the_device_list_after_the_shutdown() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    host.pair_with(&joiner).await;
    let file = host.dir.path().join("link-devices.json");
    let before = std::fs::read(&file).unwrap();
    host.link.shutdown().await;

    host.link.ctx.remember(joiner.id(), [evil()]);

    assert!(matches!(host.link.ctx.writer(), Err(Error::Closed)));
    assert_eq!(std::fs::read(&file).unwrap(), before);
}

#[tokio::test]
async fn the_connections_of_a_source_are_counted_for_the_64_bit_prefix_of_an_ipv6_address() {
    let node = Node::start("node").await;
    let ctx = &node.link.ctx;
    let in_prefix = |last: u16| -> std::net::IpAddr { format!("fd00:1:2:3::{last:x}").parse().unwrap() };

    let guards: Vec<_> = (1..=ctx::MAX_CONNECTIONS_PER_IP as u16)
        .map(|last| ctx.admit(in_prefix(last)).unwrap())
        .collect();

    assert!(
        ctx.admit(in_prefix(0x99)).is_none(),
        "another address of the same /64 is the same source"
    );
    assert!(
        ctx.admit("fd00:1:2:4::1".parse().unwrap()).is_some(),
        "another /64 is another source"
    );
    drop(guards);
    assert!(ctx.admit(in_prefix(0x99)).is_some());
}
