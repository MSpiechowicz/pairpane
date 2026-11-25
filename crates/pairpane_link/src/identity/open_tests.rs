//! Which store holds the key: the marker decides, and a store that fails never leads to a new key. The stores are fakes;
//! nothing here touches the OS keychain.

use std::fs;
use std::sync::Mutex;

use iroh::SecretKey;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::{MARKER_FILE_NAME, open_identity};
use crate::error::{Error, Result};
use crate::identity::{KeyStore, KeyStores, StoreKind};

/// A store that holds a key in memory and can be told to fail.
struct Fake {
    kind: StoreKind,
    key: Mutex<Option<SecretKey>>,
    fails: Mutex<Option<fn() -> Error>>,
    saves: Mutex<u32>,
}

impl Fake {
    fn new(kind: StoreKind) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            kind,
            key: Mutex::new(None),
            fails: Mutex::new(None),
            saves: Mutex::new(0),
        })
    }

    fn fail_with(&self, error: Option<fn() -> Error>) {
        *self.fails.lock().unwrap() = error;
    }

    fn holds(&self) -> Option<SecretKey> {
        self.key.lock().unwrap().clone()
    }

    fn saves(&self) -> u32 {
        *self.saves.lock().unwrap()
    }
}

impl KeyStore for std::sync::Arc<Fake> {
    fn kind(&self) -> StoreKind {
        self.kind
    }

    fn load(&self) -> Result<Option<SecretKey>> {
        if let Some(error) = *self.fails.lock().unwrap() {
            return Err(error());
        }
        Ok(self.holds())
    }

    fn save(&self, key: &SecretKey) -> Result<()> {
        *self.saves.lock().unwrap() += 1;
        *self.key.lock().unwrap() = Some(key.clone());
        Ok(())
    }
}

fn locked() -> Error {
    Error::Keychain("the collection is locked".to_owned())
}

fn no_service() -> Error {
    Error::KeychainUnavailable("no secret service".to_owned())
}

struct Rig {
    dir: TempDir,
    keychain: std::sync::Arc<Fake>,
    file: std::sync::Arc<Fake>,
}

impl Rig {
    fn new() -> Self {
        Self {
            dir: TempDir::new().unwrap(),
            keychain: Fake::new(StoreKind::Keychain),
            file: Fake::new(StoreKind::File),
        }
    }

    fn open(&self, has_devices: bool) -> Result<(SecretKey, StoreKind)> {
        let stores = KeyStores {
            primary: Box::new(self.keychain.clone()),
            fallback: Some(Box::new(self.file.clone())),
        };

        open_identity(self.dir.path(), &stores, has_devices)
    }

    fn marker(&self) -> String {
        fs::read_to_string(self.dir.path().join(MARKER_FILE_NAME)).unwrap()
    }
}

#[test]
fn the_first_start_makes_a_key_in_the_keychain_and_writes_a_marker_without_a_secret() {
    let rig = Rig::new();

    let (key, kind) = rig.open(false).unwrap();

    assert_eq!(kind, StoreKind::Keychain);
    assert_eq!(rig.keychain.holds().unwrap().to_bytes(), key.to_bytes());
    assert!(rig.file.holds().is_none());
    let marker: serde_json::Value = serde_json::from_str(&rig.marker()).unwrap();
    assert_eq!(marker["store"], "keychain");
    assert_eq!(marker["node_id"], key.public().to_string());
    assert!(!rig.marker().contains(&format!("{:?}", key.to_bytes())));
}

#[test]
fn a_later_start_gets_the_same_key_without_making_one() {
    let rig = Rig::new();
    let (first, _) = rig.open(false).unwrap();

    let (second, kind) = rig.open(true).unwrap();

    assert_eq!(first.to_bytes(), second.to_bytes());
    assert_eq!(kind, StoreKind::Keychain);
    assert_eq!(rig.keychain.saves(), 1);
}

#[test]
fn a_locked_keychain_after_the_first_start_is_an_error_and_never_a_new_key() {
    let rig = Rig::new();
    rig.open(false).unwrap();
    let marker = rig.marker();

    rig.keychain.fail_with(Some(locked));
    let result = rig.open(true);

    assert!(matches!(result, Err(Error::KeyUnavailable(_))), "{result:?}");
    assert!(rig.file.holds().is_none());
    assert_eq!(rig.file.saves(), 0);
    assert_eq!(rig.keychain.saves(), 1);
    assert_eq!(rig.marker(), marker);

    // Even a keychain that reports it does not exist at all does not move a device that was set up with it.
    rig.keychain.fail_with(Some(no_service));
    assert!(matches!(rig.open(true), Err(Error::KeyUnavailable(_))));
    assert!(rig.file.holds().is_none());
}

#[test]
fn a_key_that_is_gone_from_its_store_is_an_error_and_never_a_new_key() {
    let rig = Rig::new();
    rig.open(false).unwrap();
    *rig.keychain.key.lock().unwrap() = None;

    assert!(matches!(rig.open(true), Err(Error::KeyUnavailable(_))));
    assert_eq!(rig.keychain.saves(), 1);
    assert!(rig.keychain.holds().is_none());
}

#[test]
fn a_key_that_is_not_the_one_of_the_marker_is_refused() {
    let rig = Rig::new();
    rig.open(false).unwrap();
    *rig.keychain.key.lock().unwrap() = Some(SecretKey::generate());

    assert!(matches!(rig.open(true), Err(Error::KeyMismatch)));
}

#[test]
fn a_damaged_or_unknown_marker_is_an_error() {
    for text in [
        "not json",
        "{\"version\":2,\"store\":\"file\",\"node_id\":\"00\"}",
        "{}",
        "",
    ] {
        let rig = Rig::new();
        fs::write(rig.dir.path().join(MARKER_FILE_NAME), text).unwrap();

        assert!(matches!(rig.open(false), Err(Error::CorruptIdentityMarker)), "{text:?}");
        assert!(rig.keychain.holds().is_none() && rig.file.holds().is_none());
    }
}

#[test]
fn without_a_marker_a_key_file_that_exists_wins_and_is_marked() {
    let rig = Rig::new();
    let existing = SecretKey::generate();
    rig.file.save(&existing).unwrap();

    let (key, kind) = rig.open(true).unwrap();

    assert_eq!((key.to_bytes(), kind), (existing.to_bytes(), StoreKind::File));
    assert_eq!(rig.keychain.saves(), 0);
    assert!(rig.marker().contains("\"file\""));
}

#[test]
fn without_a_marker_a_missing_keychain_falls_back_to_a_file_for_a_new_device() {
    let rig = Rig::new();
    rig.keychain.fail_with(Some(no_service));

    let (key, kind) = rig.open(false).unwrap();

    assert_eq!(kind, StoreKind::File);
    assert_eq!(rig.file.holds().unwrap().to_bytes(), key.to_bytes());
    assert!(rig.marker().contains("\"file\""));

    // The keychain coming back later does not change the identity.
    rig.keychain.fail_with(None);
    assert_eq!(rig.open(true).unwrap().0.to_bytes(), key.to_bytes());
    assert_eq!(rig.keychain.saves(), 0);
}

#[test]
fn without_a_marker_a_keychain_that_is_there_but_refuses_never_gets_a_file() {
    let rig = Rig::new();
    rig.keychain.fail_with(Some(locked));

    let result = rig.open(false);

    assert!(matches!(result, Err(Error::KeyUnavailable(_))), "{result:?}");
    assert_eq!(rig.file.saves(), 0);
    assert!(!rig.dir.path().join(MARKER_FILE_NAME).exists());
}

#[test]
fn without_a_marker_no_file_is_made_for_a_device_that_has_paired_devices() {
    let rig = Rig::new();
    rig.keychain.fail_with(Some(no_service));

    let result = rig.open(true);

    assert!(matches!(result, Err(Error::KeyUnavailable(_))), "{result:?}");
    assert_eq!(rig.file.saves(), 0);
}

#[test]
fn without_a_marker_a_key_that_the_keychain_has_is_kept() {
    let rig = Rig::new();
    let existing = SecretKey::generate();
    rig.keychain.save(&existing).unwrap();

    let (key, kind) = rig.open(true).unwrap();

    assert_eq!((key.to_bytes(), kind), (existing.to_bytes(), StoreKind::Keychain));
    assert_eq!(rig.keychain.saves(), 1);
}
