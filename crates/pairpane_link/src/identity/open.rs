//! Choosing the key store once, and holding to it.
//!
//! The first time a device starts it makes its key in a store and writes the marker `link-identity.json` next to its
//! other files: the store's kind and the node id (the public key), nothing secret. From then on the marker decides: the
//! key is read from that store only, and if that fails (a locked keyring, a Secret Service that is slow to start) the
//! start fails with [`Error::KeyUnavailable`] instead of making a new key. A new key would be a new node id, which every
//! paired device would refuse. The key read must also be the one the marker names.
//!
//! With no marker, a key file that is there wins, then the keychain is used (a key that is in it, or a new one). A file
//! is made instead only when the system has no keychain at all and no device is paired yet.

use std::path::Path;

use iroh::{EndpointId, SecretKey};
use serde::{Deserialize, Serialize};

use super::{KeyStore, KeyStores, StoreKind};
use crate::error::{Error, Result};
use crate::fsutil;

/// The marker's file name in the data folder.
pub const MARKER_FILE_NAME: &str = "link-identity.json";

/// The version of the marker's layout.
const MARKER_VERSION: u32 = 1;

/// The most bytes of a marker that are read.
const MAX_MARKER_BYTES: u64 = 4096;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    version: u32,
    store: StoreKind,
    node_id: EndpointId,
}

/// The device key and the kind of store it is in, found as the module's documentation says. `has_devices` is whether
/// any device is paired already. The stores may block (the keychain talks to a service), so call this off an async
/// runtime's own threads.
pub fn open_identity(dir: &Path, stores: &KeyStores, has_devices: bool) -> Result<(SecretKey, StoreKind)> {
    match read_marker(dir)? {
        Some(marker) => open_marked(&marker, stores),
        None => open_unmarked(dir, stores, has_devices),
    }
}

fn open_marked(marker: &Marker, stores: &KeyStores) -> Result<(SecretKey, StoreKind)> {
    let store = stores
        .of_kind(marker.store)
        .ok_or_else(|| Error::KeyUnavailable("the key is in a store that this build does not use".to_owned()))?;

    let key = match store.load() {
        Ok(Some(key)) => key,
        Ok(None) => return Err(Error::KeyUnavailable("the key is missing from its store".to_owned())),
        Err(error) => return Err(unavailable(error)),
    };

    if key.public() != marker.node_id {
        return Err(Error::KeyMismatch);
    }

    Ok((key, marker.store))
}

fn open_unmarked(dir: &Path, stores: &KeyStores, has_devices: bool) -> Result<(SecretKey, StoreKind)> {
    // A key file that is there is the device's key, whatever the keychain says now.
    if let Some(fallback) = stores.fallback.as_deref()
        && let Some(key) = fallback.load()?
    {
        return mark(dir, fallback.kind(), key);
    }

    let primary = stores.primary.as_ref();
    match load_or_make(primary) {
        Ok(key) => mark(dir, primary.kind(), key),
        Err(Error::KeychainUnavailable(reason)) => match stores.fallback.as_deref() {
            Some(fallback) if !has_devices => mark(dir, fallback.kind(), make_in(fallback)?),
            _ => Err(Error::KeyUnavailable(reason)),
        },
        Err(error) => Err(unavailable(error)),
    }
}

fn load_or_make(store: &dyn KeyStore) -> Result<SecretKey> {
    match store.load()? {
        Some(key) => Ok(key),
        None => make_in(store),
    }
}

fn make_in(store: &dyn KeyStore) -> Result<SecretKey> {
    let key = SecretKey::generate();
    store.save(&key)?;

    Ok(key)
}

/// A keychain failure that is not the lack of a keychain: the key may still be there, so it is reported as that.
fn unavailable(error: Error) -> Error {
    match error {
        Error::Keychain(reason) | Error::KeychainUnavailable(reason) => Error::KeyUnavailable(reason),
        other => other,
    }
}

fn mark(dir: &Path, store: StoreKind, key: SecretKey) -> Result<(SecretKey, StoreKind)> {
    let marker = Marker {
        version: MARKER_VERSION,
        store,
        node_id: key.public(),
    };
    let bytes = serde_json::to_vec_pretty(&marker).map_err(|_| Error::CorruptKey)?;
    fsutil::write_atomic(dir, MARKER_FILE_NAME, &bytes)?;

    Ok((key, store))
}

fn read_marker(dir: &Path) -> Result<Option<Marker>> {
    let bytes = match fsutil::read_capped(&dir.join(MARKER_FILE_NAME), MAX_MARKER_BYTES) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => return Err(damaged()),
        Err(error) => return Err(error.into()),
    };

    let marker: Marker = serde_json::from_slice(&bytes).map_err(|_| damaged())?;
    if marker.version != MARKER_VERSION {
        return Err(damaged());
    }

    Ok(Some(marker))
}

fn damaged() -> Error {
    Error::KeyUnavailable("the identity marker file is damaged".to_owned())
}

#[cfg(test)]
#[path = "open_tests.rs"]
mod tests;
