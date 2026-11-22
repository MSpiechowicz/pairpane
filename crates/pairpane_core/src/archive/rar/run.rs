//! Running `unrar`: the two commands this code uses, started and supervised through [`crate::process`].
//!
//! | what | command |
//! | ---- | ------- |
//! | list | `unrar vt -p- -cfg- -@ -- <archive>` (the technical listing, read by [`parse`](super::parse)) |
//! | unpack | `unrar x -p- -o+ -ol- -idq -cfg- -@ -- <archive> [<name>...] ./` (run in the staging folder) |
//!
//! `--` ends the switches before any path or name; `-p-` never asks for a password (and never uses one); `-cfg-` ignores
//! the configuration files and the `RAR` variable; `-@` reads no list files (a name that starts with `@` is a name);
//! `-ol-` skips every symbolic link instead of creating it; `-o+` overwrites (the staging folder is empty, and a name that
//! comes twice in an archive is wanted as its last one). No shell is involved. The environment is cleared but for
//! `LC_ALL` (a UTF-8 locale, so names are written as UTF-8) and `TZ=UTC` (so the times it prints are UTC). The standard
//! input is closed, the standard error and, when unpacking, the standard output go nowhere. The destination is relative
//! (`./`) to the working folder, which is the staging folder: when that folder is removed (the app exits while `unrar`
//! runs) `unrar` fails instead of making the folder again at an absolute path. The listing's output is read
//! with caps ([`parse`](super::parse)) and `unrar` is killed when the cancel flag is raised or the deadline passes.

use std::ffi::OsString;
use std::io;
use std::path::{MAIN_SEPARATOR_STR, Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::atomic::AtomicBool;
use std::thread;
use std::time::{Duration, Instant};

use crate::archive::MIN_FREE_RESERVE;
use crate::listing::platform;
use crate::process::{self, Exit, GRACE, Wait};
use crate::view::refusal;

use super::parse::{Listing, parse};
use super::{find, full, missing, protected, stage, version};
use crate::archive::error::{cancelled, check, unreadable, unsupported};

/// How long a listing may take.
const LIST_DEADLINE: Duration = Duration::from_secs(5 * 60);

/// How long an unpacking may take: a big archive on a slow disk takes hours, a hung `unrar` should not stay forever.
const EXTRACT_DEADLINE: Duration = Duration::from_secs(6 * 60 * 60);

/// How often a running unpacking touches its staging folder (see [`stage::touch`]).
const TOUCH_EVERY: Duration = Duration::from_secs(5);

/// A UTF-8 locale of this system; `unrar` writes names in the locale's charset.
#[cfg(target_os = "macos")]
pub(super) const LOCALE: &str = "en_US.UTF-8";
#[cfg(not(target_os = "macos"))]
pub(super) const LOCALE: &str = "C.UTF-8";

// The exit codes of `unrar` that are told apart.
const CODE_OK: i32 = 0;
const CODE_WARNING: i32 = 1;
const CODE_CRC: i32 = 3;
const CODE_WRITE: i32 = 5;
const CODE_CREATE: i32 = 9;
const CODE_NO_FILES: i32 = 10;
const CODE_PASSWORD: i32 = 11;

/// The `unrar` program that is started.
#[derive(Debug, Clone)]
pub struct Unrar {
    pub(super) program: PathBuf,
}

impl Unrar {
    /// The `unrar` of this system (see [`find`]), or the error [`missing`] makes, or [`super::too_old`] when it is older than 7.
    pub fn locate() -> io::Result<Self> {
        let program = find::find()?;

        version::require(&program)?;

        Ok(Self { program })
    }

    /// The program at `program`, an absolute path.
    pub fn at(program: PathBuf) -> Self {
        Self { program }
    }

    /// The entries `unrar` lists for `archive`. `InvalidData` when it is not an archive `unrar` reads, a password refusal
    /// ([`protected`]) when its headers are encrypted, `Interrupted` when `cancel` is raised.
    pub fn list(&self, archive: &Path, cancel: &AtomicBool) -> io::Result<Listing> {
        check(cancel)?;
        plain(archive)?;

        let mut child = self.start(list_args(archive), None, Stdio::piped())?;
        let Some(stdout) = child.stdout.take() else {
            process::stop(&mut child);

            return Err(unreadable());
        };
        let reader = thread::Builder::new()
            .name("unrar-list".to_owned())
            .spawn(move || parse(stdout))
            .map_err(|_| unreadable())?;

        let waited = process::wait_for(&mut child, cancel, LIST_DEADLINE, || reader.is_finished());

        // Stopped in every case; its output closes with it, which ends the reader (or it is left behind).
        process::stop(&mut child);
        let parsed = process::join_within(reader, GRACE);

        match waited {
            Ok(status) => listed(status, parsed),
            Err(Wait::Cancelled) => Err(cancelled()),
            Err(Wait::Deadline) => Err(too_slow()),
            // The reader stopped at a cap and `unrar` went on writing.
            Err(Wait::Stalled) => match parsed {
                Some(Ok(listing)) if listing.cut => Ok(listing),
                _ => Err(unreadable()),
            },
            Err(Wait::Lost) => Err(unreadable()),
        }
    }

    /// Unpacks `names` (everything when `None`) of `archive` into the existing, empty folder `staging`, which `unrar` gets
    /// as its working folder; its destination is that folder as `./` (never the absolute path). A file that cannot be unpacked is not there afterwards (a damaged one
    /// is removed by `unrar`); that is not an error here. While it runs the free space of the volume of `staging` is
    /// looked at, and `unrar` is killed with a "storage full" refusal when less than [`MIN_FREE_RESERVE`] is left.
    pub fn extract(
        &self,
        archive: &Path,
        names: Option<&[&str]>,
        staging: &Path,
        cancel: &AtomicBool,
    ) -> io::Result<()> {
        check(cancel)?;
        plain(archive)?;
        plain(staging)?;

        let mut child = self.start(extract_args(archive, names), Some(staging), Stdio::null())?;
        let mut touched = Instant::now();
        let waited = process::wait_for_exit(&mut child, cancel, EXTRACT_DEADLINE, || {
            // The folder's age is the time since its last sign of life, for the sweep of another instance.
            if touched.elapsed() >= TOUCH_EVERY {
                touched = Instant::now();
                stage::touch(staging);
            }

            keep_reserve(staging, MIN_FREE_RESERVE)
        });
        process::stop(&mut child);

        match waited {
            Ok(status) => unpacked(status),
            Err(Exit::Watch(error)) => Err(error),
            Err(Exit::Gave(Wait::Cancelled)) => Err(cancelled()),
            Err(Exit::Gave(Wait::Deadline)) => Err(too_slow()),
            Err(Exit::Gave(Wait::Stalled | Wait::Lost)) => Err(unreadable()),
        }
    }

    fn start(&self, args: Vec<OsString>, folder: Option<&Path>, output: Stdio) -> io::Result<Child> {
        let mut command = process::command(&self.program, args, &[]);
        command.env("LC_ALL", LOCALE).env("TZ", "UTC").stdout(output);

        if let Some(folder) = folder {
            command.current_dir(folder);
        }

        command.spawn().map_err(|error| {
            tracing::debug!(%error, "unrar did not start");

            if error.kind() == io::ErrorKind::NotFound {
                missing()
            } else {
                refusal(io::ErrorKind::Other, "unrar could not be started")
            }
        })
    }
}

/// The arguments of the listing of `archive`.
pub(super) fn list_args(archive: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["vt", "-p-", "-cfg-", "-@", "--"].map(OsString::from).into();
    args.push(archive.into());

    args
}

/// The arguments that unpack `names` of `archive` into the working folder, as `./`.
pub(super) fn extract_args(archive: &Path, names: Option<&[&str]>) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["x", "-p-", "-o+", "-ol-", "-idq", "-cfg-", "-@", "--"]
        .map(OsString::from)
        .into();
    args.push(archive.into());
    args.extend(names.unwrap_or_default().iter().map(OsString::from));

    // The destination is the last argument, and it is told from a name by its trailing separator. It is relative: the
    // working folder is the staging folder, and a folder that is gone must not be made again by `unrar`.
    let mut destination = OsString::from(".");
    destination.push(MAIN_SEPARATOR_STR);
    args.push(destination);

    args
}

/// Fails with [`full`] when the volume of `staging` has less than `reserve` left (and, where the system cannot tell,
/// never).
pub(super) fn keep_reserve(staging: &Path, reserve: u64) -> io::Result<()> {
    match platform::free_space(staging) {
        Some(free) if free < reserve => Err(full()),
        _ => Ok(()),
    }
}

/// Refuses a path that is not absolute, or that holds a wildcard: `unrar` reads the archive's name as a mask, so a
/// `*` or `?` in it could name other archives.
fn plain(path: &Path) -> io::Result<()> {
    let text = path.to_string_lossy();

    if !path.is_absolute() || text.contains(['*', '?']) {
        return Err(unsupported("unrar cannot be given this path"));
    }

    Ok(())
}

/// The result of a listing that ended with `status`, whose output read as `parsed` (`None`: the reader did not end).
fn listed(status: ExitStatus, parsed: Option<io::Result<Listing>>) -> io::Result<Listing> {
    if status.code() == Some(CODE_PASSWORD) {
        return Err(protected());
    }

    let listing = parsed.ok_or_else(unreadable)??;

    match status.code() {
        Some(CODE_OK) => Ok(listing),
        // A warning or a damaged end: what was listed is good, the archive is cut.
        Some(CODE_WARNING | CODE_CRC) => Ok(Listing { cut: true, ..listing }),
        _ => Err(unreadable()),
    }
}

/// What an unpacking that ended with `status` means. Files that were damaged, or not found, are missing from the folder
/// and each of their entries fails on its own.
fn unpacked(status: ExitStatus) -> io::Result<()> {
    match status.code() {
        Some(CODE_OK | CODE_WARNING | CODE_CRC | CODE_NO_FILES) => Ok(()),
        Some(CODE_PASSWORD) => Err(protected()),
        Some(CODE_WRITE | CODE_CREATE) => Err(refusal(
            io::ErrorKind::Other,
            "unrar could not write the unpacked files",
        )),
        _ => Err(unreadable()),
    }
}

fn too_slow() -> io::Error {
    refusal(io::ErrorKind::TimedOut, "unrar took too long")
}
