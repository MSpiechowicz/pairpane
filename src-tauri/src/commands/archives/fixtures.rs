//! Archives for the tests of the archive commands: a tar and a stored zip built at run time by hand, and a 7z with
//! encrypted headers kept as bytes.

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::de::DeserializeOwned;
use tauri::ipc::{Channel, InvokeResponseBody};

/// One entry of a fixture: its path (a trailing `/` makes a folder), its content and, for a zip, whether it is marked as
/// encrypted (only the marker: the data is not really encrypted, which is enough to be listed and refused).
pub(in crate::commands) struct Item<'a> {
    pub(in crate::commands) path: &'a str,
    pub(in crate::commands) data: &'a [u8],
    pub(in crate::commands) encrypted: bool,
    /// For a zip: the size the archive declares, when it is not the content's.
    pub(in crate::commands) declared: Option<u32>,
}

pub(in crate::commands) fn item<'a>(path: &'a str, data: &'a [u8]) -> Item<'a> {
    Item {
        path,
        data,
        encrypted: false,
        declared: None,
    }
}

/// A zip entry marked as encrypted.
pub(in crate::commands) fn encrypted<'a>(path: &'a str, data: &'a [u8]) -> Item<'a> {
    Item {
        encrypted: true,
        ..item(path, data)
    }
}

/// A zip entry that declares `size` bytes and holds none.
pub(in crate::commands) fn declaring(path: &str, size: u32) -> Item<'_> {
    Item {
        declared: Some(size),
        ..item(path, b"")
    }
}

/// A POSIX (ustar) tar of `items`, with the checksum of every header.
pub(in crate::commands) fn tar_of(items: &[Item<'_>]) -> Vec<u8> {
    let mut tar = Vec::new();

    for item in items {
        let folder = item.path.ends_with('/');
        let mut header = [0_u8; 512];
        header[..item.path.len()].copy_from_slice(item.path.as_bytes());
        header[100..108].copy_from_slice(if folder { b"0000755\0" } else { b"0000644\0" });
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        header[124..136].copy_from_slice(format!("{:011o}\0", item.data.len()).as_bytes());
        header[136..148].copy_from_slice(b"14000000000\0");
        header[148..156].copy_from_slice(b"        ");
        header[156] = if folder { b'5' } else { b'0' };
        header[257..265].copy_from_slice(b"ustar\0\x30\x30");

        let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());

        tar.extend_from_slice(&header);
        tar.extend_from_slice(item.data);
        tar.resize(tar.len().next_multiple_of(512), 0);
    }
    tar.resize(tar.len() + 1024, 0);

    tar
}

/// A zip of `items`, stored (no compression).
pub(in crate::commands) fn zip_of(items: &[Item<'_>]) -> Vec<u8> {
    let mut zip = Vec::new();
    let mut directory = Vec::new();

    for item in items {
        let name = item.path.as_bytes();
        let crc = crc32(item.data);
        let flags: u16 = u16::from(item.encrypted);
        let size = item
            .declared
            .unwrap_or_else(|| u32::try_from(item.data.len()).expect("small"));
        let at = u32::try_from(zip.len()).expect("small");

        // Local header, then the data.
        zip.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        zip.extend_from_slice(&[20, 0]);
        zip.extend_from_slice(&flags.to_le_bytes());
        zip.extend_from_slice(&[0, 0, 0, 0, 0x21, 0]);
        zip.extend_from_slice(&crc.to_le_bytes());
        zip.extend_from_slice(&size.to_le_bytes());
        zip.extend_from_slice(&size.to_le_bytes());
        zip.extend_from_slice(&u16::try_from(name.len()).expect("short").to_le_bytes());
        zip.extend_from_slice(&[0, 0]);
        zip.extend_from_slice(name);
        zip.extend_from_slice(item.data);

        // Its record in the central directory.
        directory.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        directory.extend_from_slice(&[20, 0, 20, 0]);
        directory.extend_from_slice(&flags.to_le_bytes());
        directory.extend_from_slice(&[0, 0, 0, 0, 0x21, 0]);
        directory.extend_from_slice(&crc.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&u16::try_from(name.len()).expect("short").to_le_bytes());
        directory.extend_from_slice(&[0; 12]);
        directory.extend_from_slice(&at.to_le_bytes());
        directory.extend_from_slice(name);
    }

    let at = u32::try_from(zip.len()).expect("small");
    let length = u32::try_from(directory.len()).expect("small");
    let count = u16::try_from(items.len()).expect("few");
    zip.extend_from_slice(&directory);
    zip.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    zip.extend_from_slice(&[0; 4]);
    zip.extend_from_slice(&count.to_le_bytes());
    zip.extend_from_slice(&count.to_le_bytes());
    zip.extend_from_slice(&length.to_le_bytes());
    zip.extend_from_slice(&at.to_le_bytes());
    zip.extend_from_slice(&[0, 0]);

    zip
}

/// The password of [`sevenz_with_encrypted_headers`].
pub(in crate::commands) const SEVENZ_PASSWORD: &str = "secret";

/// A 7z archive that holds one file, `a.txt` ("alpha"), whose headers are encrypted with [`SEVENZ_PASSWORD`]: its names
/// cannot be listed without the password. Written once by the core crate's 7z writer (a dev-dependency of that crate
/// only) and kept as bytes.
pub(in crate::commands) fn sevenz_with_encrypted_headers() -> Vec<u8> {
    let hex = concat!(
        "377abcaf271c0004f669e043800000000000000053000000000000009fbfb9c6",
        "55a3fde1139e93cd707474417c2129a3ffe5d94783977c0580ff4fc504d59b72",
        "377f234f9693f838bc60ff00964a2122305697450f29207155b1c88ddddbd68c",
        "c0d10760f41a74422f61cd45319e971c9690c42e72df396ed328c4733b021b3b",
        "b0304decf6abc6fdfe8cfbe0740b39692bf88df0b5c80a786fc7157187c8f4a4",
        "1706100109700a018745cd5000070b0100022406f1070122c8ff7d6b7ea93d55",
        "cf1cbeb7492f5f9babafb3ef78421b679b1beac35c14fdf5230c23030101055d",
        "0000800001000c636100080a01f5415dcd0000",
    );

    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).expect("hex"))
        .collect()
}

/// The CRC-32 of `data` (the one a zip stores).
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0_u32;

    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }

    !crc
}

/// A folder holding the archive `name` with `bytes`, and the archive's path as the page would send it.
pub(in crate::commands) fn archive_in_folder(name: &str, bytes: &[u8]) -> (tempfile::TempDir, String) {
    let folder = tempfile::tempdir().expect("folder");
    std::fs::write(folder.path().join(name), bytes).expect("archive");
    let path = folder.path().join(name);

    (folder, path_text(&path))
}

pub(in crate::commands) fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// A channel that records the events the page would get, decoded.
pub(in crate::commands) fn collecting<T: DeserializeOwned + Send + 'static>() -> (Channel<T>, Arc<Mutex<Vec<T>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);

    let channel = Channel::new(move |body| {
        let InvokeResponseBody::Json(text) = body else {
            panic!("events are sent as JSON");
        };
        let event = serde_json::from_str(&text).expect("an event");
        sink.lock().expect("not poisoned").push(event);

        Ok(())
    });

    (channel, log)
}
