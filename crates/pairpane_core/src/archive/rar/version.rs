//! Which `unrar` this is. The program prints its banner (`UNRAR 7.30 beta 2 freeware ...`) when it is started with no
//! arguments; the version at its start is read once per program and kept (only a version that was read: a program that
//! printed nothing this code reads is asked again next time). Versions before 7 are refused ([`too_old`], which names
//! the version): their technical listing and their switches differ from what this code reads. A banner that cannot be
//! read is let through, so a program that words it differently still works. The banner is read as bytes and decoded
//! lossily, so a character cut by the read's limit does not hide the version before it.

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use crate::process::{self, GRACE};

use super::run::LOCALE;
use super::too_old;

/// The oldest major version that is used.
const MIN_MAJOR: u32 = 7;

/// How long the banner may take, and how much of it is read.
const BANNER_DEADLINE: Duration = Duration::from_secs(10);
const BANNER_BYTES: u64 = 4096;

/// The most digits of the part after the dot that are taken for a version (`7.30`).
const MAX_MINOR_DIGITS: usize = 4;

/// A version of `unrar` as its banner prints it: the number before the first dot, and the digits after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Version {
    pub(super) major: u32,
    minor: String,
}

impl fmt::Display for Version {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{}.{}", self.major, self.minor)
    }
}

/// The version `program` prints, or `None` when it prints none this code reads.
fn banner_version(program: &Path) -> Option<Version> {
    let mut child = process::command(program, [""; 0], &[])
        .env("LC_ALL", LOCALE)
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let reader = thread::Builder::new()
        .name("unrar-banner".to_owned())
        .spawn(move || banner_text(stdout))
        .ok()?;

    // A program that prints less than a banner, or more, is stopped; what was read is what counts.
    let _ = process::wait_for(&mut child, &AtomicBool::new(false), BANNER_DEADLINE, || {
        reader.is_finished()
    });
    process::stop(&mut child);

    parse_version(&process::join_within(reader, GRACE)?)
}

/// The first [`BANNER_BYTES`] of what `source` yields, as text: bytes that are not UTF-8 (a character the limit cut in two
/// among them) become replacement characters instead of failing the whole read.
pub(super) fn banner_text(source: impl Read) -> String {
    let mut bytes = Vec::new();
    let _ = source.take(BANNER_BYTES).read_to_end(&mut bytes);

    String::from_utf8_lossy(&bytes).into_owned()
}

/// The version in the banner `text`: the number after the first `UNRAR` or `RAR` word, up to its dot, and the digits that
/// follow the dot.
pub(super) fn parse_version(text: &str) -> Option<Version> {
    text.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        words.find(|word| matches!(*word, "UNRAR" | "RAR"))?;

        let (major, rest) = words.next()?.split_once('.')?;
        let minor: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if minor.is_empty() || minor.len() > MAX_MINOR_DIGITS {
            return None;
        }

        Some(Version {
            major: major.parse().ok()?,
            minor,
        })
    })
}

/// Fails with [`too_old`] when `program` is a version this code does not work with; asked once per program.
pub(super) fn require(program: &Path) -> io::Result<()> {
    require_with(program, banner_version)
}

/// [`require`] with the way a version is read given. Only a version that was read is kept.
pub(super) fn require_with(program: &Path, read: impl FnOnce(&Path) -> Option<Version>) -> io::Result<()> {
    static KNOWN: OnceLock<Mutex<HashMap<PathBuf, Version>>> = OnceLock::new();

    let known = KNOWN.get_or_init(Mutex::default);
    let kept = known.lock().ok().and_then(|known| known.get(program).cloned());

    let version = match kept {
        Some(version) => Some(version),
        None => {
            let read = read(program);

            if let (Some(version), Ok(mut known)) = (&read, known.lock()) {
                known.insert(program.to_owned(), version.clone());
            }

            read
        }
    };

    match version {
        Some(version) if version.major < MIN_MAJOR => Err(too_old(Some(&version.to_string()))),
        _ => Ok(()),
    }
}
