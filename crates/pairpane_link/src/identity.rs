//! The device's identity: one Ed25519 secret key, made once and kept where only this user can read it. Its public half
//! is the device's node id, the name other devices pin when they pair with it.
//!
//! A [`KeyStore`] keeps the 32 key bytes. [`KeychainStore`] uses the OS keychain, [`FileStore`] (Unix only) a file that
//! only its owner may read, for a Linux desktop without a Secret Service, and [`MemoryStore`] is for tests.
//! [`default_store`] picks between the first two, and [`KeyStore::kind`] says which one it is, so the app can tell the
//! user where the key is kept.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use iroh::SecretKey;

use crate::error::{Error, Result};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            Ok(bytes) => key_from(&bytes).map(Some),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keychain_error(error)),
        }
    }

    fn save(&self, key: &SecretKey) -> Result<()> {
        Self::entry()?.set_secret(&key.to_bytes()).map_err(keychain_error)
    }
}

fn keychain_error(error: keyring::Error) -> Error {
    Error::Keychain(error.to_string())
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
        use std::io::Read;
        use std::os::unix::fs::PermissionsExt;

        match std::fs::symlink_metadata(&self.path) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => return Err(Error::InsecureKeyFile),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }

        let file = std::fs::File::open(&self.path)?;
        // The mode and the size of the file that is open, not of a name that could point elsewhere by now.
        let meta = file.metadata()?;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(Error::InsecureKeyFile);
        }

        let mut bytes = Vec::new();
        file.take(KEY_LEN as u64 + 1).read_to_end(&mut bytes)?;

        key_from(&bytes).map(Some)
    }

    fn save(&self, key: &SecretKey) -> Result<()> {
        let dir = self.path.parent().unwrap_or(Path::new("."));

        fsutil::write_atomic(dir, KEY_FILE_NAME, &key.to_bytes())?;
        Ok(())
    }
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

/// The store the app uses: the OS keychain, or on Linux a [`FileStore`] in `fallback_dir` when there is no usable
/// keychain (no Secret Service running, a locked or refused session). A key file that already exists is kept in use, so
/// a keychain that shows up later does not give the device a new identity. Elsewhere a keychain that fails is an error
/// when the key is used, not a silent file.
///
/// The keychain is probed with a read, which may talk to the Secret Service; call this off the async runtime's threads.
pub fn default_store(fallback_dir: &Path) -> Box<dyn KeyStore> {
    #[cfg(target_os = "linux")]
    {
        let file = FileStore::new(fallback_dir);
        if file.exists() || matches!(KeychainStore.load(), Err(Error::Keychain(_))) {
            return Box::new(file);
        }
    }

    #[cfg(not(target_os = "linux"))]
    let _ = fallback_dir;

    Box::new(KeychainStore)
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
