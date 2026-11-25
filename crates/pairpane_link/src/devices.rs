//! The paired devices: the other devices of this user that this one trusts. A record is the device's node id (its
//! public key, the only thing that is checked when it connects), a name for the user to read and the time it was paired.
//!
//! The list is one JSON file in the link's folder, written all or nothing. It is read with a size limit and with no
//! field it does not know, so a file that was edited by something else, or damaged, is refused instead of half-read.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::error::{Error, Result};
use crate::fsutil;

/// The most characters of a device name.
pub const MAX_NAME_CHARS: usize = 64;
/// The most devices that can be paired.
pub const MAX_DEVICES: usize = 256;
/// The file name of the list in the link's folder.
pub const DEVICES_FILE_NAME: &str = "link-devices.json";

/// The most addresses kept for a device.
pub const MAX_ADDRS: usize = 8;

/// The largest list file that is read. A full list of [`MAX_DEVICES`] records is about 60 KiB.
const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// The version of the file's layout.
const FILE_VERSION: u32 = 1;

/// One paired device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    /// The device's public key; it is written as 64 hexadecimal digits.
    pub node_id: EndpointId,
    /// The name the user sees, 1 to [`MAX_NAME_CHARS`] characters.
    pub name: String,
    /// When it was paired, in milliseconds since the Unix epoch.
    pub paired_at: u64,
    /// Where the device was last reached, newest first, at most [`MAX_ADDRS`]. A list file from before addresses were
    /// kept has none.
    #[serde(default)]
    pub addrs: Vec<SocketAddr>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    version: u32,
    devices: Vec<Device>,
}

/// A device name as it is kept: trimmed, in Unicode normal form C, 1 to [`MAX_NAME_CHARS`] characters, none of them a
/// control character, a format character or a line or paragraph separator. Those can make a name look like another
/// (a right-to-left override reverses what follows it) or hide characters (a zero-width space), which matters where the
/// user decides which device to trust by its name.
pub fn validate_name(name: &str) -> Result<String> {
    let name: String = name.trim().nfc().collect();

    let length = name.chars().count();
    if length == 0 || length > MAX_NAME_CHARS || name.chars().any(|c| c.is_control() || is_invisible(c)) {
        return Err(Error::InvalidName);
    }

    Ok(name)
}

/// Whether `c` is in Unicode's general category Cf (format), Zl (line separator) or Zp (paragraph separator). There is no
/// crate for the categories in the dependency tree, so the characters of the categories are listed, with the
/// unassigned ones of the same blocks (a character that is assigned later is refused already).
fn is_invisible(c: char) -> bool {
    matches!(c,
        // Soft hyphen, Arabic number signs and the Arabic letter mark, the Syriac abbreviation mark, the Arabic
        // disputed end of ayah and small high marks.
        '\u{00AD}' | '\u{0600}'..='\u{0605}' | '\u{061C}' | '\u{06DD}' | '\u{070F}' | '\u{0890}'..='\u{0891}' | '\u{08E2}'
        // Mongolian vowel separator.
        | '\u{180E}'
        // Zero-width characters, the direction marks and the embeddings and overrides, the line and paragraph
        // separators, the word joiner and invisible operators, the isolates, and the deprecated format characters.
        | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{206F}'
        // The byte order mark and the interlinear annotation marks.
        | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}'
        // Brahmi, Kaithi and Egyptian hieroglyph format controls, shorthand and musical format controls.
        | '\u{110BD}' | '\u{110CD}' | '\u{13430}'..='\u{1343F}' | '\u{1BCA0}'..='\u{1BCA3}' | '\u{1D173}'..='\u{1D17A}'
        // The tag characters.
        | '\u{E0000}'..='\u{E007F}')
}

/// Moves a list file that cannot be read aside so the service can start with an empty list, and returns where it went:
/// `link-devices.json.bad-<milliseconds since the Unix epoch>` in the same folder, with `-<n>` added when a file of that
/// name is there already (an earlier reset in the same millisecond), so a file that was set aside is never overwritten.
/// The file is never deleted, so the records can still be recovered by hand. Call it only after
/// [`Error::CorruptDevices`]; `None` means there was no file. It takes the lock on the folder while it works, so a
/// service that runs on the folder makes it fail with [`Error::AlreadyRunning`].
pub fn reset_devices(dir: &Path) -> Result<Option<PathBuf>> {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());

    move_aside(dir, millis)
}

fn move_aside(dir: &Path, millis: u128) -> Result<Option<PathBuf>> {
    // A folder that is not there has no list; it is not made just to find that out.
    if !dir.is_dir() {
        return Ok(None);
    }
    let _lock = fsutil::lock_dir(dir, fsutil::LOCK_FILE_NAME)?;

    let file = dir.join(DEVICES_FILE_NAME);
    if std::fs::symlink_metadata(&file).is_err() {
        return Ok(None);
    }

    // The lock keeps every other writer of the folder out, so a name that is free now stays free until the move.
    let mut aside = dir.join(format!("{DEVICES_FILE_NAME}.bad-{millis}"));
    for attempt in 1.. {
        if std::fs::symlink_metadata(&aside).is_err() {
            break;
        }
        aside = dir.join(format!("{DEVICES_FILE_NAME}.bad-{millis}-{attempt}"));
    }
    std::fs::rename(&file, &aside)?;

    Ok(Some(aside))
}

/// The list of paired devices and the file it is kept in. Every change is written to the file before it is kept in
/// memory, so a failed write changes nothing.
#[derive(Debug)]
pub struct DeviceStore {
    dir: PathBuf,
    devices: Vec<Device>,
}

impl DeviceStore {
    /// Opens the list in `dir`; a folder or a file that does not exist yet is an empty list.
    pub fn open(dir: &Path) -> Result<Self> {
        let devices = match fsutil::read_capped(&dir.join(DEVICES_FILE_NAME), MAX_FILE_BYTES) {
            Ok(Some(bytes)) => parse(&bytes)?,
            Ok(None) => Vec::new(),
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => return Err(Error::CorruptDevices),
            Err(error) => return Err(error.into()),
        };

        Ok(Self {
            dir: dir.to_owned(),
            devices,
        })
    }

    /// The paired devices, in the order they were first paired.
    pub fn list(&self) -> &[Device] {
        &self.devices
    }

    /// The device with this node id, if it is paired.
    pub fn get(&self, node_id: &EndpointId) -> Option<&Device> {
        self.devices.iter().find(|device| &device.node_id == node_id)
    }

    /// Whether the device with this node id is paired.
    pub fn is_paired(&self, node_id: &EndpointId) -> bool {
        self.get(node_id).is_some()
    }

    /// Pairs a device that has no known address. See [`DeviceStore::add_with_addrs`].
    pub fn add(&mut self, node_id: EndpointId, name: &str, paired_at: u64) -> Result<Device> {
        self.add_with_addrs(node_id, name, paired_at, &[])
    }

    /// Pairs a device that was reached at `addrs`. A device that is paired already keeps its place in the list and gets
    /// the new name and time, and the new addresses in front of the ones it had.
    pub fn add_with_addrs(
        &mut self,
        node_id: EndpointId,
        name: &str,
        paired_at: u64,
        addrs: &[SocketAddr],
    ) -> Result<Device> {
        let mut device = Device {
            node_id,
            name: validate_name(name)?,
            paired_at,
            addrs: Vec::new(),
        };

        let mut next = self.devices.clone();
        if let Some(known) = next.iter_mut().find(|known| known.node_id == node_id) {
            device.addrs = merged(&known.addrs, addrs).unwrap_or_else(|| known.addrs.clone());
            *known = device.clone();
        } else if next.len() >= MAX_DEVICES {
            return Err(Error::TooManyDevices);
        } else {
            device.addrs = merged(&[], addrs).unwrap_or_default();
            next.push(device.clone());
        }

        self.commit(next)?;
        Ok(device)
    }

    /// Notes that a paired device was reached at `addrs`: they go in front of the ones kept, without repeats, and only
    /// [`MAX_ADDRS`] are kept. The file is written only when this changes the list; it does not when the addresses are
    /// the newest ones already. A device that is not paired is ignored. Returns whether the list changed.
    pub fn remember_addrs(&mut self, node_id: &EndpointId, addrs: &[SocketAddr]) -> Result<bool> {
        let Some(known) = self.get(node_id) else {
            return Ok(false);
        };
        let Some(merged) = merged(&known.addrs, addrs) else {
            return Ok(false);
        };

        let mut next = self.devices.clone();
        if let Some(device) = next.iter_mut().find(|device| &device.node_id == node_id) {
            device.addrs = merged;
        }

        self.commit(next)?;
        Ok(true)
    }

    /// Gives a paired device a new name.
    pub fn rename(&mut self, node_id: &EndpointId, name: &str) -> Result<()> {
        let name = validate_name(name)?;

        let mut next = self.devices.clone();
        next.iter_mut()
            .find(|device| &device.node_id == node_id)
            .ok_or(Error::UnknownDevice)?
            .name = name;

        self.commit(next)
    }

    /// Unpairs a device.
    pub fn remove(&mut self, node_id: &EndpointId) -> Result<()> {
        let mut next = self.devices.clone();
        let before = next.len();
        next.retain(|device| &device.node_id != node_id);
        if next.len() == before {
            return Err(Error::UnknownDevice);
        }

        self.commit(next)
    }

    fn commit(&mut self, next: Vec<Device>) -> Result<()> {
        let body = Body {
            version: FILE_VERSION,
            devices: next,
        };
        let bytes = serde_json::to_vec_pretty(&body).map_err(|_| Error::CorruptDevices)?;

        fsutil::write_atomic(&self.dir, DEVICES_FILE_NAME, &bytes)?;

        self.devices = body.devices;
        Ok(())
    }
}

/// The addresses kept after `fresh` were seen: `fresh` first, then the older ones that are not repeated, cut to
/// [`MAX_ADDRS`]. `None` when that changes nothing: the fresh ones already are the newest.
pub(crate) fn merged(kept: &[SocketAddr], fresh: &[SocketAddr]) -> Option<Vec<SocketAddr>> {
    let fresh = deduplicated(fresh);

    let newest_already = kept.len() >= fresh.len() && kept.iter().take(fresh.len()).all(|addr| fresh.contains(addr));
    if newest_already {
        return None;
    }

    Some(deduplicated(&[fresh.as_slice(), kept].concat()))
}

/// The first [`MAX_ADDRS`] different addresses, in order.
fn deduplicated(addrs: &[SocketAddr]) -> Vec<SocketAddr> {
    let mut kept: Vec<SocketAddr> = Vec::new();
    for addr in addrs {
        if kept.len() < MAX_ADDRS && !kept.contains(addr) {
            kept.push(*addr);
        }
    }

    kept
}

/// The records of a list file, checked: the known version, valid names, no node id twice and not too many.
fn parse(bytes: &[u8]) -> Result<Vec<Device>> {
    let body: Body = serde_json::from_slice(bytes).map_err(|_| Error::CorruptDevices)?;

    if body.version != FILE_VERSION || body.devices.len() > MAX_DEVICES {
        return Err(Error::CorruptDevices);
    }

    for (index, device) in body.devices.iter().enumerate() {
        let name_is_valid = validate_name(&device.name).is_ok_and(|name| name == device.name);
        let is_repeated = body.devices[..index]
            .iter()
            .any(|earlier| earlier.node_id == device.node_id);

        if !name_is_valid || is_repeated {
            return Err(Error::CorruptDevices);
        }
    }

    // A file with more addresses than are kept, or repeats, is not damaged; the first ones are kept.
    let mut devices = body.devices;
    for device in &mut devices {
        device.addrs = deduplicated(&device.addrs);
    }

    Ok(devices)
}

#[cfg(test)]
#[path = "devices_tests.rs"]
mod tests;
