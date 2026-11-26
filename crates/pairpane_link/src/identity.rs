//! The device's identity: one Ed25519 secret key, made once and kept where only this user can read it. Its public half
//! is the device's node id, the name other devices pin when they pair with it.
//!
//! A [`KeyStore`] keeps the 32 key bytes. [`KeychainStore`] uses the OS keychain, [`FileStore`] (Unix only) a file that
//! only its owner may read, for a Linux desktop without a Secret Service, and [`MemoryStore`] is for tests.
//! [`default_stores`] names the first two for the app, and [`KeyStore::kind`] says which one is in use, so the app can
//! tell the user where the key is kept.
//!
//! Which store holds the key is decided once and remembered in a marker file (see [`open_identity`]): a device whose
//! keychain is locked for a moment must not make a new key in a file and so become a different device.

mod open;

use std::fmt;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::Mutex;

use iroh::SecretKey;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{Error, Result};

pub use open::open_identity;

#[cfg(unix)]
use crate::fsutil;

/// The keychain service name, the same as the app's bundle identifier.
pub const KEYCHAIN_SERVICE: &str = "com.mspiechowicz.pairpane";
/// The keychain account of the device key.
pub const KEYCHAIN_ACCOUNT: &str = "link-device-key";
/// The file name of the key in a [`FileStore`] folder.
pub const KEY_FILE_NAME: &str = "link-device.key";

/// The length of a secret key in bytes.
const KEY_LEN: usize = 32;

/// Where the device key is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StoreKind {
    /// The OS keychain (Keychain Services, Credential Manager, Secret Service).
    Keychain,
    /// A file that only its owner can read.
    File,
    /// The memory of the process; the key is gone when it ends.
    Memory,
}

/// A place the device key can be read from and written to.
pub trait KeyStore: Send + Sync {
    /// Where this store keeps the key.
    fn kind(&self) -> StoreKind;

    /// The stored key, or `None` when nothing has been stored yet.
    fn load(&self) -> Result<Option<SecretKey>>;

    /// Stores `key`, replacing any key that is there.
    fn save(&self, key: &SecretKey) -> Result<()>;
}

/// The device key of `store`, made and stored first if the store has none. The key is made once; a later call, also
/// after a restart, returns the same key.
pub fn load_or_create(store: &dyn KeyStore) -> Result<SecretKey> {
    if let Some(key) = store.load()? {
        return Ok(key);
    }

    let key = SecretKey::generate();
    store.save(&key)?;

    Ok(key)
}

/// A key from the bytes of a store, which must be exactly 32.
fn key_from(bytes: &[u8]) -> Result<SecretKey> {
    let bytes: &[u8; KEY_LEN] = bytes.try_into().map_err(|_| Error::CorruptKey)?;

    Ok(SecretKey::from_bytes(bytes))
}

/// The bytes of a key, which are wiped when dropped.
fn key_bytes(key: &SecretKey) -> Zeroizing<[u8; KEY_LEN]> {
    Zeroizing::new(key.to_bytes())
}

/// The stores a device may keep its key in: the one it prefers and, when that is missing from the system altogether, a
/// second one.
pub struct KeyStores {
    /// The store a new key goes in first.
    pub primary: Box<dyn KeyStore>,
    /// The store used instead when the primary does not exist on this system (and the device has no pairing yet).
    pub fallback: Option<Box<dyn KeyStore>>,
}

impl KeyStores {
    /// One store and no fallback.
    pub fn single(store: impl KeyStore + 'static) -> Self {
        Self {
            primary: Box::new(store),
            fallback: None,
        }
    }

    fn of_kind(&self, kind: StoreKind) -> Option<&dyn KeyStore> {
        let fallback = self.fallback.as_deref();

        std::iter::once(self.primary.as_ref())
            .chain(fallback)
            .find(|store| store.kind() == kind)
    }
}

/// The key in the OS keychain, under [`KEYCHAIN_SERVICE`] and [`KEYCHAIN_ACCOUNT`].
#[derive(Debug, Default)]
pub struct KeychainStore;

impl KeychainStore {
    fn entry() -> Result<keyring::Entry> {
        keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).map_err(keychain_error)
    }
}

impl KeyStore for KeychainStore {
    fn kind(&self) -> StoreKind {
        StoreKind::Keychain
    }

    fn load(&self) -> Result<Option<SecretKey>> {
        match Self::entry()?.get_secret() {
            Ok(bytes) => key_from(&Zeroizing::new(bytes)).map(Some),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keychain_error(error)),
        }
    }

    fn save(&self, key: &SecretKey) -> Result<()> {
        Self::entry()?.set_secret(&*key_bytes(key)).map_err(keychain_error)
    }
}

/// A keychain failure. Only the lack of a keychain on the system (`NoDefaultStore`: no Secret Service to talk to) is
/// told apart: a keychain that is there but locked, denied or slow is a failure that may pass, and a key must not be
/// moved to a file for it.
fn keychain_error(error: keyring::Error) -> Error {
    match error {
        keyring::Error::NoDefaultStore => Error::KeychainUnavailable(error.to_string()),
        other => Error::Keychain(other.to_string()),
    }
}

/// The key in a file of its own in a folder (Unix only). The file is written through a temporary file that is renamed
/// over it, with mode 0600, and a file that group or other can access is refused instead of read: someone else may have
/// read or planted it.
#[cfg(unix)]
#[derive(Debug)]
pub struct FileStore {
    path: PathBuf,
}

#[cfg(unix)]
impl FileStore {
    /// A store for the file [`KEY_FILE_NAME`] in `dir`. The folder is created when the key is first saved.
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join(KEY_FILE_NAME),
        }
    }

    /// Whether the key file is there.
    pub fn exists(&self) -> bool {
        std::fs::symlink_metadata(&self.path).is_ok()
    }
}

#[cfg(unix)]
impl KeyStore for FileStore {
    fn kind(&self) -> StoreKind {
        StoreKind::File
    }

    fn load(&self) -> Result<Option<SecretKey>> {
        use std::os::unix::fs::OpenOptionsExt;

        match std::fs::symlink_metadata(&self.path) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => return Err(Error::InsecureKeyFile),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }

        // A link swapped in after the check above is refused by the system, not followed, and a pipe swapped in is
        // opened without waiting for a writer (which would block the start for good); it is refused below.
        let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK;
        let opened = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(flags.bits() as i32)
            .open(&self.path);
        let file = match opened {
            Ok(file) => file,
            Err(error) if error.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error()) => {
                return Err(Error::InsecureKeyFile);
            }
            Err(error) => return Err(error.into()),
        };

        read_key_file(file).map(Some)
    }

    fn save(&self, key: &SecretKey) -> Result<()> {
        let dir = self.path.parent().unwrap_or(Path::new("."));

        fsutil::write_atomic(dir, KEY_FILE_NAME, &*key_bytes(key))?;
        Ok(())
    }
}

/// The key in an open key file, which must be a regular file of this user that nobody else can access. All of it is
/// checked on the open descriptor, not on a name that could point elsewhere by now.
#[cfg(unix)]
fn read_key_file(file: std::fs::File) -> Result<SecretKey> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let meta = file.metadata()?;
    let owned_by_us = meta.uid() == rustix::process::geteuid().as_raw();
    if !meta.is_file() || meta.permissions().mode() & 0o077 != 0 || !owned_by_us {
        return Err(Error::InsecureKeyFile);
    }

    let mut bytes = Zeroizing::new(Vec::with_capacity(KEY_LEN + 1));
    file.take(KEY_LEN as u64 + 1).read_to_end(&mut bytes)?;

    key_from(&bytes)
}

/// A store that is shared, so the app can keep a handle to the one it gave the service.
impl<T: KeyStore + ?Sized> KeyStore for std::sync::Arc<T> {
    fn kind(&self) -> StoreKind {
        (**self).kind()
    }

    fn load(&self) -> Result<Option<SecretKey>> {
        (**self).load()
    }

    fn save(&self, key: &SecretKey) -> Result<()> {
        (**self).save(key)
    }
}

/// The key in the memory of the process, for tests. It never touches the keychain or the disk.
#[derive(Default)]
pub struct MemoryStore {
    key: Mutex<Option<SecretKey>>,
}

impl MemoryStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl fmt::Debug for MemoryStore {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("MemoryStore")
    }
}

impl KeyStore for MemoryStore {
    fn kind(&self) -> StoreKind {
        StoreKind::Memory
    }

    fn load(&self) -> Result<Option<SecretKey>> {
        Ok(self.key.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone())
    }

    fn save(&self, key: &SecretKey) -> Result<()> {
        *self.key.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(key.clone());
        Ok(())
    }
}

/// The stores the app uses: the OS keychain and, on Linux, a [`FileStore`] in `fallback_dir` for a desktop that has no
/// Secret Service at all. Nothing is read or written here; [`open_identity`] decides, once, which one holds the key.
pub fn default_stores(fallback_dir: &Path) -> KeyStores {
    #[cfg(target_os = "linux")]
    {
        KeyStores {
            primary: Box::new(KeychainStore),
            fallback: Some(Box::new(FileStore::new(fallback_dir))),
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = fallback_dir;
        KeyStores::single(KeychainStore)
    }
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
