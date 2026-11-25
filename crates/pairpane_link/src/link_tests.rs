//! Two or three services on the loopback address, pairing and linking with each other. Each has its own folder and a
//! key in memory, so nothing touches the OS keychain, and nothing leaves the machine.

use std::time::Duration;

use iroh::endpoint::{ConnectionError, VarInt};
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::*;
use crate::pairing::{MAX_FAILURES, PairingEndReason};
use crate::wire::CLOSE_NOT_ALLOWED;

pub(super) struct Node {
    pub(super) link: Link,
    pub(super) dir: TempDir,
}

impl Node {
    pub(super) async fn start(name: &str) -> Self {
        Self::start_with(name, |_| {}).await
    }

    pub(super) async fn start_with(name: &str, change: impl FnOnce(&mut LinkConfig)) -> Self {
        Self::start_in(TempDir::new().unwrap(), name, change).await
    }

    /// Starts a service on the folder `dir`, which it keeps.
    pub(super) async fn start_in(dir: TempDir, name: &str, change: impl FnOnce(&mut LinkConfig)) -> Self {
        let mut config = LinkConfig::new(dir.path(), name);
        config.ports = vec![0];
        config.loopback_only = true;
        config.discovery = Discovery::Off;
        change(&mut config);

        let link = Link::start(config, KeyStores::single(crate::identity::MemoryStore::new()))
            .await
            .unwrap();
        Self { link, dir }
    }

    pub(super) fn id(&self) -> EndpointId {
        self.link.this_device().unwrap().node_id
    }

    pub(super) fn target(&self) -> Target {
        self.link.this_device().unwrap().target()
    }

    /// Pairs `joiner` with this node by the code it shows.
    pub(super) async fn pair_with(&self, joiner: &Node) {
        let offer = self.link.begin_pairing().unwrap();
        joiner.link.join(&offer.code, &self.target()).await.unwrap();
    }
}

/// The setup of a service that is started again in the same folder: the folder is what it keeps.
fn loopback_config(dir: &std::path::Path) -> LinkConfig {
    let mut config = LinkConfig::new(dir, "joiner");
    (config.ports, config.loopback_only, config.discovery) = (vec![0], true, Discovery::Off);
    config
}

/// A well-formed code that is not the offered one.
pub(super) fn wrong_code(offer: &PairingOffer) -> &'static str {
    if offer.code.starts_with('0') {
        "1234-5678"
    } else {
        "0000-0000"
    }
}

pub(super) async fn closed_with(conn: &Connection) -> VarInt {
    match timeout(Duration::from_secs(5), conn.closed()).await.unwrap() {
        ConnectionError::ApplicationClosed(close) => close.error_code,
        other => panic!("closed in another way: {other}"),
    }
}

#[tokio::test]
async fn this_device_reports_its_id_name_and_addresses() {
    let node = Node::start("alpha").await;

    let device = node.link.this_device().unwrap();

    assert_eq!(device.name, "alpha");
    assert_eq!(device.key_store, StoreKind::Memory);
    assert_eq!(device.bound.len(), 1);
    assert!(
        device.bound[0].ip().is_loopback() && device.bound[0].port() != 0,
        "{:?}",
        device.bound
    );
    assert!(!device.addrs.is_empty(), "no address to reach the device at");
    assert!(
        device.addrs.iter().all(|addr| addr.ip().is_loopback()),
        "{:?}",
        device.addrs
    );
    assert_eq!(device.target().to_string().parse::<Target>().unwrap(), device.target());
}

#[tokio::test]
async fn the_right_code_pairs_and_both_sides_pin_each_other() {
    let host = Node::start("host-device").await;
    let joiner = Node::start("joiner-device").await;
    let mut events = host.link.subscribe().unwrap();

    let offer = host.link.begin_pairing().unwrap();
    let paired = joiner
        .link
        .join(&offer.code.to_lowercase(), &host.target())
        .await
        .unwrap();

    assert_eq!((paired.node_id, paired.name.as_str()), (host.id(), "host-device"));
    let on_host = host.link.devices().unwrap();
    let on_joiner = joiner.link.devices().unwrap();
    assert_eq!(on_host.len(), 1);
    assert_eq!(
        (on_host[0].node_id, on_host[0].name.as_str()),
        (joiner.id(), "joiner-device")
    );
    assert_eq!(on_joiner, vec![paired]);
    assert!(on_host[0].paired_at > 1_700_000_000_000);
    assert_eq!(host.link.pairing_state().unwrap(), PairingState::Idle);
    // The code is used up first, then the device is announced.
    assert_eq!(
        events.recv().await.unwrap(),
        LinkEvent::PairingEnded {
            reason: PairingEndReason::Used
        }
    );
    assert_eq!(events.recv().await.unwrap(), LinkEvent::Paired(on_host[0].clone()));
}

#[tokio::test]
async fn pairing_and_the_address_of_the_device_are_remembered_after_a_restart() {
    let dir = TempDir::new().unwrap();
    let store = std::sync::Arc::new(crate::identity::MemoryStore::new());
    let config = || loopback_config(dir.path());
    let host = Node::start("host").await;
    let first = Link::start(config(), KeyStores::single(store.clone())).await.unwrap();
    let offer = host.link.begin_pairing().unwrap();
    first.join(&offer.code, &host.target()).await.unwrap();
    let id = first.this_device().unwrap().node_id;
    first.shutdown().await;

    let second = Link::start(config(), KeyStores::single(store)).await.unwrap();

    assert_eq!(second.this_device().unwrap().node_id, id);
    assert_eq!(second.devices().unwrap().len(), 1);
    assert_eq!(second.devices().unwrap()[0].node_id, host.id());
    // Nothing is known in memory and nothing finds the host (discovery is off): only the device list has its address.
    assert_eq!(second.devices().unwrap()[0].addrs, host.target().addrs);
    assert_eq!(
        second.status(&host.id()).await.unwrap(),
        Status::Online {
            name: "host".to_owned()
        }
    );
}

#[tokio::test]
async fn a_wrong_code_fails_and_counts_as_an_attempt() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    let offer = host.link.begin_pairing().unwrap();
    let wrong = wrong_code(&offer);

    let result = joiner.link.join(wrong, &host.target()).await;

    assert!(matches!(result, Err(Error::PairingRejected)), "{result:?}");
    assert!(host.link.devices().unwrap().is_empty() && joiner.link.devices().unwrap().is_empty());
    let PairingState::Waiting { attempts_left, .. } = host.link.pairing_state().unwrap() else {
        panic!("the code is still active after one failure");
    };
    assert_eq!(attempts_left, MAX_FAILURES - 1);

    // The right code still works within the tries that are left.
    joiner.link.join(&offer.code, &host.target()).await.unwrap();
    assert_eq!(host.link.devices().unwrap().len(), 1);
}

#[tokio::test]
async fn three_failures_cancel_the_code_and_then_even_the_right_one_is_refused() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    let offer = host.link.begin_pairing().unwrap();
    let wrong = wrong_code(&offer);

    for _ in 0..MAX_FAILURES {
        assert!(matches!(
            joiner.link.join(wrong, &host.target()).await,
            Err(Error::PairingRejected)
        ));
    }

    assert_eq!(host.link.pairing_state().unwrap(), PairingState::Idle);
    let result = joiner.link.join(&offer.code, &host.target()).await;
    assert!(matches!(result, Err(Error::PairingRejected)), "{result:?}");
    assert!(host.link.devices().unwrap().is_empty() && joiner.link.devices().unwrap().is_empty());
}

#[tokio::test]
async fn an_expired_code_is_refused() {
    let host = Node::start_with("host", |config| config.pairing_ttl = Duration::from_millis(200)).await;
    let joiner = Node::start("joiner").await;
    let offer = host.link.begin_pairing().unwrap();

    tokio::time::sleep(Duration::from_millis(400)).await;

    let result = joiner.link.join(&offer.code, &host.target()).await;
    assert!(matches!(result, Err(Error::PairingRejected)), "{result:?}");
    assert_eq!(host.link.pairing_state().unwrap(), PairingState::Idle);
    assert!(host.link.devices().unwrap().is_empty());
}

#[tokio::test]
async fn a_used_code_cannot_be_used_again() {
    let host = Node::start("host").await;
    let first = Node::start("first").await;
    let second = Node::start("second").await;
    let offer = host.link.begin_pairing().unwrap();
    first.link.join(&offer.code, &host.target()).await.unwrap();

    let result = second.link.join(&offer.code, &host.target()).await;

    assert!(matches!(result, Err(Error::PairingRejected)), "{result:?}");
    assert_eq!(host.link.devices().unwrap().len(), 1);
    assert!(second.link.devices().unwrap().is_empty());
}

#[tokio::test]
async fn a_cancelled_code_and_a_host_that_never_showed_one_refuse_pairing() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;

    let never = joiner.link.join("7K2M-9QXZ", &host.target()).await;
    let offer = host.link.begin_pairing().unwrap();
    host.link.cancel_pairing().unwrap();
    let cancelled = joiner.link.join(&offer.code, &host.target()).await;

    assert!(matches!(never, Err(Error::PairingRejected)), "{never:?}");
    assert!(matches!(cancelled, Err(Error::PairingRejected)), "{cancelled:?}");
    assert!(host.link.devices().unwrap().is_empty() && joiner.link.devices().unwrap().is_empty());
}

#[tokio::test]
async fn a_text_that_is_not_a_code_is_refused_before_any_connection() {
    let joiner = Node::start("joiner").await;
    let host = Node::start("host").await;

    assert!(matches!(
        joiner.link.join("hello", &host.target()).await,
        Err(Error::InvalidCode)
    ));
}

#[tokio::test]
async fn an_unpaired_device_is_closed_on_the_link_protocol() {
    let host = Node::start("host").await;
    let stranger = Node::start("stranger").await;

    let conn = stranger
        .link
        .ctx
        .endpoint
        .connect(host.target().into_addr(), LINK_ALPN)
        .await
        .unwrap();

    assert_eq!(closed_with(&conn).await, CLOSE_NOT_ALLOWED);
    assert_eq!(stranger.link.status(&host.id()).await.unwrap(), Status::Offline);
}

#[tokio::test]
async fn a_paired_device_gets_the_hello_and_shows_as_online() {
    let host = Node::start("host-device").await;
    let joiner = Node::start("joiner-device").await;
    host.pair_with(&joiner).await;

    assert_eq!(
        joiner.link.status(&host.id()).await.unwrap(),
        Status::Online {
            name: "host-device".to_owned()
        }
    );
    assert_eq!(
        host.link.status(&joiner.id()).await.unwrap(),
        Status::Online {
            name: "joiner-device".to_owned()
        }
    );
}

#[tokio::test]
async fn a_device_that_was_unpaired_is_refused_and_one_that_stopped_is_offline() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    host.pair_with(&joiner).await;
    assert!(matches!(
        joiner.link.status(&host.id()).await.unwrap(),
        Status::Online { .. }
    ));

    host.link.unpair(&joiner.id()).unwrap();
    assert_eq!(joiner.link.status(&host.id()).await.unwrap(), Status::Refused);
    assert!(matches!(host.link.unpair(&joiner.id()), Err(Error::UnknownDevice)));

    host.pair_with(&joiner).await;
    assert!(matches!(
        joiner.link.status(&host.id()).await.unwrap(),
        Status::Online { .. }
    ));
    let host_id = host.id();
    host.link.shutdown().await;
    assert_eq!(joiner.link.status(&host_id).await.unwrap(), Status::Offline);
}

#[tokio::test]
async fn a_device_that_is_not_paired_is_offline_without_a_connection() {
    let joiner = Node::start("joiner").await;
    let other = Node::start("other").await;

    assert_eq!(joiner.link.status(&other.id()).await.unwrap(), Status::Offline);
}

#[tokio::test]
async fn the_pairing_protocol_is_closed_when_no_code_is_active() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;

    let conn = joiner
        .link
        .ctx
        .endpoint
        .connect(host.target().into_addr(), PAIR_ALPN)
        .await
        .unwrap();

    assert_eq!(closed_with(&conn).await, CLOSE_NOT_ALLOWED);
}

#[tokio::test]
async fn renaming_and_unpairing_a_device_is_kept() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    host.pair_with(&joiner).await;

    host.link.rename_device(&joiner.id(), "Kitchen laptop").unwrap();

    assert_eq!(host.link.devices().unwrap()[0].name, "Kitchen laptop");
    assert!(matches!(
        host.link.rename_device(&joiner.id(), ""),
        Err(Error::InvalidName)
    ));
}

#[tokio::test]
async fn the_service_binds_to_the_next_port_when_the_first_is_taken() {
    let first = Node::start("first").await;
    let taken = first.link.this_device().unwrap().bound[0].port();

    let second = Node::start_with("second", |config| config.ports = vec![taken, 0]).await;

    let second_port = second.link.this_device().unwrap().bound[0].port();
    assert!(second_port != 0);
    // Where a second socket can share a port (Windows and macOS let it), the first port is used and this holds nothing;
    // only Linux is sure to refuse it.
    if cfg!(target_os = "linux") {
        assert!(second_port != taken);
    }
}

/// Needs multicast on a real network interface, so it does not run by default: `cargo test -p pairpane_link -- --ignored`.
#[tokio::test]
#[ignore = "needs mDNS multicast on the local network"]
async fn devices_on_the_local_network_find_each_other_and_pair_by_node_id() {
    let announce = |config: &mut LinkConfig| {
        config.loopback_only = false;
        config.discovery = Discovery::Find;
    };
    let host = Node::start_with("found-host", announce).await;
    let joiner = Node::start_with("found-joiner", announce).await;

    tokio::time::sleep(Duration::from_secs(3)).await;
    let before = joiner.link.discovered().unwrap();
    assert!(
        before.iter().all(|found| found.node_id != host.id()),
        "a device that is not pairing is not announced"
    );
    let offer = host.link.begin_pairing().unwrap();

    let found = timeout(Duration::from_secs(20), async {
        loop {
            if let Some(found) = joiner
                .link
                .discovered()
                .unwrap()
                .into_iter()
                .find(|found| found.node_id == host.id())
            {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("the host was found");

    assert_eq!(found.name.as_deref(), Some("found-host"));
    assert!(!found.paired && !found.addrs.is_empty());
    assert!(
        joiner
            .link
            .discovered()
            .unwrap()
            .iter()
            .all(|found| found.node_id != joiner.id())
    );

    joiner
        .link
        .join(
            &offer.code,
            &Target {
                node_id: host.id(),
                addrs: Vec::new(),
            },
        )
        .await
        .unwrap();

    assert!(
        joiner
            .link
            .discovered()
            .unwrap()
            .iter()
            .any(|found| found.node_id == host.id() && found.paired)
    );
}

#[tokio::test]
async fn the_host_keeps_the_address_a_device_paired_and_connected_from() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    host.pair_with(&joiner).await;
    let joiner_addr = joiner.link.this_device().unwrap().bound[0];

    assert_eq!(host.link.devices().unwrap()[0].addrs, vec![joiner_addr]);

    // The address of every authenticated link connection goes to the front again: the probe moves it before :9. It is
    // written by a thread for blocking work, so it is waited for.
    host.link
        .remember_addresses(joiner.id(), vec!["127.0.0.1:9".parse().unwrap()])
        .unwrap();
    joiner.link.status(&host.id()).await.unwrap();
    let expected = vec![joiner_addr, "127.0.0.1:9".parse().unwrap()];
    timeout(Duration::from_secs(20), async {
        while host.link.devices().unwrap()[0].addrs != expected {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the address of the connection was kept");
}
