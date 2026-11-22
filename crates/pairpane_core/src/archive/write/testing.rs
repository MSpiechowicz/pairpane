//! Helpers of the writers' tests: a sample tree, packing it, reading an archive back through the readers of the crate and
//! through `bsdtar`.

use std::io::{self, Cursor, Read};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use crate::archive::testing::{MOMENT, idle, noise};
use crate::archive::{EntryType, Flow, Input, Password, build_index, open_source};
use crate::model::{ArchiveFormat, PackFormat, PackLevel};

use super::{Entry, Options, Packer};

/// Every format the writers make, with the format the readers know it as.
pub(crate) const FORMATS: [(PackFormat, ArchiveFormat); 3] = [
    (PackFormat::Zip, ArchiveFormat::Zip),
    (PackFormat::TarZst, ArchiveFormat::TarZst),
    (PackFormat::SevenZ, ArchiveFormat::SevenZ),
];

pub(crate) const LEVELS: [PackLevel; 4] = [PackLevel::Store, PackLevel::Fast, PackLevel::Normal, PackLevel::Maximum];

/// One item to pack: a folder (no content) or a file.
pub(crate) struct Item {
    pub name: String,
    pub mode: u32,
    pub modified: i64,
    pub content: Option<Vec<u8>>,
}

impl Item {
    pub(crate) fn file(name: &str, mode: u32, content: &[u8]) -> Self {
        Self {
            name: name.to_owned(),
            mode,
            modified: MOMENT,
            content: Some(content.to_vec()),
        }
    }

    pub(crate) fn folder(name: &str, mode: u32) -> Self {
        Self {
            name: name.to_owned(),
            mode,
            modified: MOMENT,
            content: None,
        }
    }

    pub(crate) fn entry(&self) -> Entry<'_> {
        match &self.content {
            None => Entry::folder(&self.name, self.mode, Some(self.modified)),
            Some(content) => Entry::file(
                &self.name,
                self.mode,
                Some(self.modified),
                content.len() as u64,
                content.as_slice(),
            ),
        }
    }
}

/// A small project with a nested folder, an empty folder and file, text that compresses and noise that does not, and a
/// name of every kind of character.
pub(crate) fn sample() -> Vec<Item> {
    let text = "the quick brown fox jumps over the lazy dog\n".repeat(2000);
    let long = format!("{}/{}.txt", "a-long-folder-name".repeat(6), "b".repeat(120));

    vec![
        Item::folder("src", 0o755),
        Item::file("src/main.rs", 0o644, b"fn main() {}\n"),
        Item::file("src/lib/util.rs", 0o600, b"pub fn util() {}\n"),
        Item::folder("empty", 0o700),
        Item::file("empty.txt", 0o644, b""),
        Item::file("run.bin", 0o755, b"\x7fELF executable\n"),
        Item::file("text.txt", 0o644, text.as_bytes()),
        Item::file("noise.bin", 0o644, &noise(70_000)),
        Item::file("żółć/ü ä.txt", 0o644, "zażółć gęślą jaźń".as_bytes()),
        Item::file(&long, 0o644, b"long"),
    ]
}

pub(crate) fn options(level: PackLevel, password: Option<&str>) -> Options {
    Options {
        level,
        password: password.map(|text| Password::new(text.to_owned())),
        encrypt_names: true,
    }
}

/// `items` packed into an archive of `format`.
pub(crate) fn pack(format: PackFormat, options: &Options, items: &[Item]) -> io::Result<Vec<u8>> {
    let cancel = idle();
    let mut packer = Packer::new(format, Cursor::new(Vec::new()), options, &cancel)?;

    for item in items {
        packer.add(item.entry())?;
    }

    Ok(packer.finish()?.into_inner())
}

/// A packer of `format` into memory, at the fast level, without a password.
pub(crate) fn packer(format: PackFormat, cancel: &AtomicBool) -> Packer<'_, Cursor<Vec<u8>>> {
    Packer::new(format, Cursor::new(Vec::new()), &options(PackLevel::Fast, None), cancel).expect("a packer")
}

/// What the readers say of an entry.
#[derive(Debug, PartialEq)]
pub(crate) struct Seen {
    pub path: String,
    pub dir: bool,
    pub mode: Option<u32>,
    pub modified: Option<i64>,
    pub content: Vec<u8>,
}

/// Every entry of `bytes`, listed and extracted through [`build_index`] and [`open_source`].
pub(crate) fn read_back(bytes: &Vec<u8>, format: ArchiveFormat, password: Option<&Password>) -> io::Result<Vec<Seen>> {
    let cancel = idle();
    let input = Input {
        file: bytes,
        path: None,
    };
    let index = build_index(format, &input, &cancel, password)?;
    let wanted: Vec<u32> = (0..index.entries().len() as u32).collect();
    let mut content = Vec::new();

    open_source(&index, &input, &cancel, password)?.extract(&wanted, &mut |_, data| {
        let mut bytes = Vec::new();
        data.read_to_end(&mut bytes)?;
        content.push(bytes);

        Ok(Flow::Continue)
    })?;

    Ok(index
        .entries()
        .iter()
        .zip(content)
        .map(|(entry, content)| Seen {
            path: entry.path.clone(),
            dir: entry.kind == EntryType::Dir,
            mode: entry.mode,
            modified: entry.modified,
            content,
        })
        .collect())
}

/// What `items` should read back as from an archive of `format`: the same paths in the same order with the same content
/// and times, and the modes where the format stores them (not a zip or a 7z made on Windows).
pub(crate) fn expected(format: PackFormat, items: &[Item]) -> Vec<Seen> {
    items
        .iter()
        .map(|item| Seen {
            path: item.name.clone(),
            dir: item.content.is_none(),
            mode: (format == PackFormat::TarZst || cfg!(unix)).then_some(item.mode & 0o777),
            modified: Some(item.modified),
            content: item.content.clone().unwrap_or_default(),
        })
        .collect()
}

/// `bsdtar` with `arguments`, or `None` when it is not installed. Its output is the standard output (lossy text for a
/// listing, the bytes of an entry for `-xOf`).
pub(crate) fn bsdtar(arguments: &[&str]) -> Option<(bool, Vec<u8>)> {
    let output = Command::new("bsdtar").args(arguments).output().ok()?;

    Some((output.status.success(), output.stdout))
}

/// Whether `bsdtar` is there and its version line names one of `libraries` (it lists what it was built with: a zip with AES
/// needs a crypto library, a tar.zst `libzstd`; the macOS one has neither).
pub(crate) fn bsdtar_has(libraries: &[&str]) -> bool {
    let Some((true, version)) = bsdtar(&["--version"]) else {
        return false;
    };
    let version = String::from_utf8_lossy(&version).to_lowercase();

    libraries.iter().any(|library| version.contains(library))
}

/// Whether `bsdtar` can open a zip with AES.
pub(crate) fn bsdtar_decrypts() -> bool {
    bsdtar_has(&["openssl", "mbedtls", "nettle", "cng"])
}

/// Checks that `bsdtar` lists the archive at `path` with `items` in it and extracts their content (with `passphrase`
/// for an encrypted one). Only the names that are ASCII are looked for by name: what a locale makes of the others is not
/// the writer's doing.
pub(crate) fn assert_bsdtar_reads(path: &str, items: &[Item], passphrase: Option<&str>) {
    let (listed, listing) = bsdtar(&["-tf", path]).expect("bsdtar");
    let listing = String::from_utf8_lossy(&listing).into_owned();

    assert!(listed, "{path}");
    assert_eq!(listing.lines().count(), items.len(), "{path}");

    for item in items.iter().filter(|item| item.name.is_ascii()) {
        assert!(
            listing.lines().any(|line| line.trim_end_matches('/') == item.name),
            "{path}: {}",
            item.name
        );

        let Some(content) = &item.content else {
            continue;
        };
        let mut arguments = Vec::new();
        if let Some(passphrase) = passphrase {
            arguments.extend(["--passphrase", passphrase]);
        }
        arguments.extend(["-xOf", path, &item.name]);
        let (read, bytes) = bsdtar(&arguments).expect("bsdtar");

        assert!(read, "{path}: {}", item.name);
        assert_eq!(&bytes, content, "{path}: {}", item.name);
    }
}

/// Writes `bytes` to `name` in `folder` and gives the path.
pub(crate) fn store(folder: &Path, name: &str, bytes: &[u8]) -> String {
    let path = folder.join(name);
    std::fs::write(&path, bytes).expect("write the archive");

    path.to_string_lossy().into_owned()
}

/// A reader that gives `bytes` and raises `cancel` with its first chunk, as a user cancelling in the middle of a file.
pub(crate) struct Raising<'a> {
    pub bytes: &'a [u8],
    pub cancel: &'a AtomicBool,
}

impl Read for Raising<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.bytes.len().min(buffer.len());
        buffer[..count].copy_from_slice(&self.bytes[..count]);
        self.bytes = &self.bytes[count..];
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);

        Ok(count)
    }
}
