//! Tests of the link service on loopback sockets, with the key in memory.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pairpane_core::model::{DevicesEvent, IpcErrorCode, PairingEnd};
use pairpane_link::{
    DeviceStore, Discovery, KeyStores, Link, LinkConfig, LinkEvent, MemoryStore, PairingEndReason, PairingState,
};
use tauri::async_runtime::block_on;

use super::convert_tests::NODE_ID;
use super::service::{StartError, forward, left_empty};
use super::{LinkService, paired_on_disk};
use crate::commands::testing::{collecting, wait_until};

/// The same service as the app's, on loopback and with the key in memory, so a test touches neither network nor keychain.
fn loopback(dir: &Path, name: &str) -> (LinkConfig, KeyStores) {
    let mut config = LinkConfig::new(dir, name);
    (config.ports, config.loopback_only, config.discovery) = (vec![0], true, Discovery::Off);

    (config, KeyStores::single(MemoryStore::new()))
}

fn service(dir: &Path) -> LinkService {
    LinkService::with_setup(Some(dir.to_owned()), "Desk".to_owned(), loopback)
}

fn pair_on_disk(dir: &Path) {
    let mut store = DeviceStore::open(dir).expect("a list");
    store
        .add(NODE_ID.parse().expect("a node id"), "Laptop", 1)
        .expect("added");
}

/// A started service with `NODE_ID` paired and a screen listening.
fn started_with_listener() -> (tempfile::TempDir, LinkService, Arc<Link>, Arc<Mutex<Vec<DevicesEvent>>>) {
    let folder = tempfile::tempdir().expect("folder");
    pair_on_disk(folder.path());
    let service = service(folder.path());
    let link = block_on(service.ensure_started()).expect("started");
    let (channel, seen) = collecting();
    service.subscribe(channel, 1);

    (folder, service, link, seen)
}

fn received(events: &Arc<Mutex<Vec<DevicesEvent>>>) -> Vec<DevicesEvent> {
    events.lock().expect("not poisoned").clone()
}

#[test]
fn a_fresh_install_has_no_paired_device_and_the_check_creates_nothing() {
    let folder = tempfile::tempdir().expect("folder");
    let missing = folder.path().join("data");

    assert!(!paired_on_disk(&missing));
    assert!(!missing.exists());
    assert!(!paired_on_disk(folder.path()));
}

#[test]
fn a_listed_device_is_found_and_an_unreadable_list_is_not() {
    let folder = tempfile::tempdir().expect("folder");
    std::fs::write(folder.path().join("link-devices.json"), "not json").expect("file");
    assert!(!paired_on_disk(folder.path()));

    let other = tempfile::tempdir().expect("folder");
    pair_on_disk(other.path());
    assert!(paired_on_disk(other.path()));
}

#[test]
fn nothing_runs_until_the_link_is_started() {
    let folder = tempfile::tempdir().expect("folder");
    let service = service(folder.path());

    let error = service.link().err().expect("not started");

    assert_eq!(error.code(), IpcErrorCode::Internal);
    assert!(service.running().is_none());
    assert!(!service.keeps_app_alive());
}

#[test]
fn without_a_data_folder_the_link_never_starts() {
    let service = LinkService::with_setup(None, "Desk".to_owned(), loopback);

    let error = block_on(service.ensure_started()).err().expect("no folder");

    assert!(matches!(error, StartError::NoFolder));
}

#[test]
fn starting_twice_at_once_starts_one_link() {
    let folder = tempfile::tempdir().expect("folder");
    let service = service(folder.path());

    let (first, second) = block_on(async { tokio::join!(service.ensure_started(), service.ensure_started()) });

    let (first, second) = (first.expect("started"), second.expect("started"));
    assert!(Arc::ptr_eq(&first, &second));
    assert!(Arc::ptr_eq(&first, &service.running().expect("running")));
    block_on(service.shutdown());
}

#[test]
fn the_app_stays_alive_only_while_a_device_is_paired() {
    let empty = tempfile::tempdir().expect("folder");
    let paired = tempfile::tempdir().expect("folder");
    pair_on_disk(paired.path());
    let (alone, with_device) = (service(empty.path()), service(paired.path()));

    block_on(async {
        alone.ensure_started().await.expect("started");
        with_device.ensure_started().await.expect("started");
    });

    assert!(!alone.keeps_app_alive());
    assert!(with_device.keeps_app_alive());
    block_on(async {
        alone.shutdown().await;
        with_device.shutdown().await;
    });
}

#[test]
fn cancelling_the_code_ends_the_one_that_is_offered() {
    let folder = tempfile::tempdir().expect("folder");
    let service = service(folder.path());
    let link = block_on(service.ensure_started()).expect("started");

    block_on(async { link.begin_pairing().expect("a code") });
    assert!(matches!(link.pairing_state(), Ok(PairingState::Waiting { .. })));
    service.cancel_code();

    assert!(matches!(link.pairing_state(), Ok(PairingState::Idle)));
    block_on(service.shutdown());
}

#[test]
fn a_stopped_service_runs_nothing_and_does_not_start_again() {
    let folder = tempfile::tempdir().expect("folder");
    let service = service(folder.path());
    block_on(service.ensure_started()).expect("started");

    block_on(service.shutdown());

    assert!(service.running().is_none());
    assert!(matches!(block_on(service.ensure_started()), Err(StartError::Stopped)));
}

#[test]
fn what_the_link_reports_reaches_the_newest_channel_only() {
    let folder = tempfile::tempdir().expect("folder");
    pair_on_disk(folder.path());
    let service = service(folder.path());
    let link = block_on(service.ensure_started()).expect("started");
    let ((old, seen_by_old), (new, seen_by_new)) = (collecting(), collecting());
    assert!(service.subscribe(old, 1));
    assert!(service.subscribe(new, 2));

    link.unpair(&NODE_ID.parse().expect("a node id")).expect("unpaired");

    wait_until("the event", || !received(&seen_by_new).is_empty());
    assert_eq!(
        received(&seen_by_new),
        vec![DevicesEvent::Unpaired {
            node_id: NODE_ID.to_owned()
        }]
    );
    assert!(received(&seen_by_old).is_empty());
    block_on(service.shutdown());
}

#[test]
fn a_listener_that_fell_behind_gets_the_list_to_start_from() {
    let (_folder, service, link, seen) = started_with_listener();

    let (events, receiver) = tokio::sync::broadcast::channel(2);
    for _ in 0..5 {
        events
            .send(LinkEvent::PairingEnded {
                reason: PairingEndReason::Expired,
            })
            .expect("a receiver");
    }
    drop(events);
    block_on(forward(receiver, Arc::downgrade(&link), service.shared.clone()));

    let events = received(&seen);
    assert!(
        matches!(&events[0], DevicesEvent::Resync { devices } if devices.len() == 1 && devices[0].node_id == NODE_ID)
    );
    assert_eq!(events[1..].len(), 2);
    assert!(events[1..].iter().all(|event| *event
        == DevicesEvent::PairingEnded {
            reason: PairingEnd::Expired
        }));
    block_on(service.shutdown());
}

#[test]
fn shutting_down_is_prompt_when_nothing_is_paired() {
    let folder = tempfile::tempdir().expect("folder");
    let service = service(folder.path());
    block_on(service.ensure_started()).expect("started");

    let finished = block_on(async { tokio::time::timeout(Duration::from_secs(5), service.shutdown()).await });

    assert!(finished.is_ok());
}

#[test]
fn an_open_older_than_the_one_that_has_the_channel_does_not_take_it() {
    let folder = tempfile::tempdir().expect("folder");
    pair_on_disk(folder.path());
    let service = service(folder.path());
    let link = block_on(service.ensure_started()).expect("started");
    let ((newer, seen_by_newer), (older, seen_by_older)) = (collecting(), collecting());

    assert!(service.subscribe(newer, 20));
    assert!(!service.subscribe(older, 19));

    link.unpair(&NODE_ID.parse().expect("a node id")).expect("unpaired");

    wait_until("the event", || !received(&seen_by_newer).is_empty());
    assert!(received(&seen_by_older).is_empty());
    block_on(service.shutdown());
}

#[test]
fn a_newer_open_or_one_of_the_same_page_takes_the_channel() {
    let folder = tempfile::tempdir().expect("folder");
    let service = service(folder.path());

    assert!(service.subscribe(collecting().0, 5));
    assert!(service.subscribe(collecting().0, 5));
    assert!(service.subscribe(collecting().0, 6));
    assert!(!service.subscribe(collecting().0, 5));
}

#[test]
fn whether_a_device_is_paired_follows_the_list_without_reading_the_files() {
    let (_folder, service, link, seen) = started_with_listener();
    assert!(service.keeps_app_alive());

    link.unpair(&NODE_ID.parse().expect("a node id")).expect("unpaired");

    wait_until("the event", || !received(&seen).is_empty());
    assert!(!service.keeps_app_alive());
    block_on(service.shutdown());
    assert!(!service.keeps_app_alive());
}

#[test]
fn a_command_that_changes_the_list_is_seen_before_any_event() {
    let folder = tempfile::tempdir().expect("folder");
    pair_on_disk(folder.path());
    let service = service(folder.path());
    let link = block_on(service.ensure_started()).expect("started");

    link.unpair(&NODE_ID.parse().expect("a node id")).expect("unpaired");
    service.refresh_paired(&link);

    assert!(!service.keeps_app_alive());
    block_on(service.shutdown());
}

#[test]
fn only_an_unpairing_or_a_new_list_that_leaves_nothing_paired_is_the_end() {
    let unpaired = DevicesEvent::Unpaired {
        node_id: NODE_ID.to_owned(),
    };
    let resync = DevicesEvent::Resync { devices: Vec::new() };
    let ended = DevicesEvent::PairingEnded {
        reason: PairingEnd::Expired,
    };
    let table = [
        (&unpaired, Some(0), true),
        (&resync, Some(0), true),
        (&unpaired, Some(1), false),
        (&resync, Some(2), false),
        // The list could not be read.
        (&unpaired, None, false),
        (&ended, Some(0), false),
    ];

    for (event, remaining, empty) in table {
        assert_eq!(left_empty(event, remaining), empty, "{event:?} {remaining:?}");
    }
}

#[test]
fn the_hook_is_called_when_the_last_device_is_unpaired_and_not_before() {
    let folder = tempfile::tempdir().expect("folder");
    let mut store = DeviceStore::open(folder.path()).expect("a list");
    let second = format!("01{}", "00".repeat(31));
    store
        .add(NODE_ID.parse().expect("a node id"), "Laptop", 1)
        .expect("added");
    store
        .add(second.parse().expect("a node id"), "Phone", 2)
        .expect("added");
    drop(store);
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let service = LinkService::with_hook(
        Some(folder.path().to_owned()),
        "Desk".to_owned(),
        loopback,
        Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        }),
    );
    let link = block_on(service.ensure_started()).expect("started");
    let (channel, seen) = collecting();
    service.subscribe(channel, 1);

    link.unpair(&NODE_ID.parse().expect("a node id")).expect("unpaired");
    wait_until("the first event", || received(&seen).len() == 1);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(service.keeps_app_alive());

    link.unpair(&second.parse().expect("a node id")).expect("unpaired");
    wait_until("the hook", || calls.load(Ordering::SeqCst) == 1);
    assert_eq!(received(&seen).len(), 2);
    assert!(!service.keeps_app_alive());
    block_on(service.shutdown());
}

#[test]
fn a_code_that_is_cancelled_is_reported_to_the_screen() {
    let folder = tempfile::tempdir().expect("folder");
    let service = service(folder.path());
    let link = block_on(service.ensure_started()).expect("started");
    let (channel, seen) = collecting();
    service.subscribe(channel, 1);
    block_on(async { link.begin_pairing().expect("a code") });

    service.cancel_code();

    wait_until("the event", || !received(&seen).is_empty());
    assert_eq!(
        received(&seen),
        vec![DevicesEvent::PairingEnded {
            reason: PairingEnd::Cancelled
        }]
    );
    block_on(service.shutdown());
}
