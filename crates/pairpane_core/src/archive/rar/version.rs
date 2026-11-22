//! Which `unrar` this is. The program prints its banner (`UNRAR 7.30 beta 2 freeware ...`) when it is started with no
//! arguments; the version before the first dot is read once per program and kept. Versions before 7 are refused: their
//! technical listing and their switches differ from what this code reads. A banner that cannot be read is let through, so
//! a program that words it differently still works.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use crate::process::{self, GRACE};

use super::run::LOCALE;

/// The oldest major version that is used.
const MIN_MAJOR: u32 = 7;

/// How long the banner may take, and how much of it is read.
const BANNER_DEADLINE: Duration = Duration::from_secs(10);
const BANNER_BYTES: u64 = 4096;

/// The major version `program` prints, or `None` when it prints none this code reads.
fn banner_major(program: &Path) -> Option<u32> {
    let mut child = process::command(program, [""; 0], &[])
        .env("LC_ALL", LOCALE)
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let reader = thread::Builder::new()
        .name("unrar-banner".to_owned())
        .spawn(move || {
            let mut text = String::new();
            let _ = stdout.take(BANNER_BYTES).read_to_string(&mut text);

            text
        })
        .ok()?;

    // A program that prints less than a banner, or more, is stopped; what was read is what counts.
    let _ = process::wait_for(&mut child, &AtomicBool::new(false), BANNER_DEADLINE, || {
        reader.is_finished()
    });
    process::stop(&mut child);

    parse_major(&process::join_within(reader, GRACE)?)
}

/// The major version in the banner `text`: the number after the first `UNRAR` or `RAR` word, up to its dot.
pub(super) fn parse_major(text: &str) -> Option<u32> {
    text.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        words.find(|word| matches!(*word, "UNRAR" | "RAR"))?;

        let version = words.next()?;
        let (major, minor) = version.split_once('.')?;
        minor.chars().next().filter(char::is_ascii_digit)?;

        major.parse().ok()
    })
}

/// Whether `program` is a version this code works with; asked once per program.
pub(super) fn is_recent(program: &Path) -> bool {
    static KNOWN: OnceLock<Mutex<HashMap<PathBuf, bool>>> = OnceLock::new();

    let known = KNOWN.get_or_init(Mutex::default);
    if let Some(answer) = known.lock().ok().and_then(|known| known.get(program).copied()) {
        return answer;
    }

    let recent = banner_major(program).is_none_or(|major| major >= MIN_MAJOR);
    if let Ok(mut known) = known.lock() {
        known.insert(program.to_owned(), recent);
    }

    recent
}
