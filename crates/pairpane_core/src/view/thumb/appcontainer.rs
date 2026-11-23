//! The thumbnail helper in an AppContainer on Windows, the sandbox that Edge and Chromium give their renderers.
//!
//! # What the helper is
//!
//! The app's own executable, started with `CreateProcessW` (`spawn.rs`) with the `SECURITY_CAPABILITIES` attribute of a
//! container of a fixed name, `com.mspiechowicz.pairpane.thumbnail`, and no capabilities (`container.rs`). That is a
//! process with a low-integrity token that can use no file, folder or registry key unless its access list names the
//! container (or ALL APPLICATION PACKAGES) for it, that cannot connect to the network or to the loopback, and that has
//! none of the user's other rights. It gets two pipes as its standard input and output and no other handle of the app's
//! (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`), an environment of two variables, and no window.
//!
//! The process is made suspended, put in a Job Object that bounds its memory and allows it no other process
//! (`heic/job.rs`, 1.5 GiB), and only then resumed, so the job holds it from its first instruction. The framing of the
//! answer, the deadline, the cancel flag and the reaping are the HEIC helper's (`heic/process.rs`).
//!
//! # The executable must be readable by the container
//!
//! The container may run the helper only if the access list of the executable lets it. An executable under `Program
//! Files` has ALL APPLICATION PACKAGES in its list through inheritance; one installed for the user alone
//! (`%LOCALAPPDATA%\Programs\...`, the default of the installer) or run from a build folder has not, and then
//! `CreateProcessW` fails with "access denied". This module takes the first of two ways out of that:
//!
//! - **(a), chosen:** when the container cannot reach the file (`CreateProcessW` says access denied, or file or path not
//!   found, depending on where the check fails), it adds one entry to the access list of the executable, for the SID of
//!   this container (and not for ALL APPLICATION PACKAGES, which every packaged app of the user is in): read and
//!   execute, on that one file, no inheritance, and starts again; if that is not enough it does the same for the folder
//!   that holds the file (list and enter, that folder only), and starts again. The entries let no other program read
//!   anything: the executable is not a secret, and the container is the helper's alone. They are never removed. They
//!   need the right to change the access list, which the user who installed the app has; the protection of the list from
//!   inheritance is kept as it was. An update that replaces the file drops its entry, and the next start adds it again.
//!   Where the entry cannot be added, or the start is denied still, the thumbnails of this run are refused as
//!   `Unsupported` ("thumbnails are not available on this system", which is the state of the system and so not kept for
//!   the file by the cache): the page shows icons ("fail closed"). A failure of the moment (no memory, no quota) refuses
//!   that one request in the same way and does not end the thumbnails of the run.
//! - (b) is that failure: this module never starts the helper as a normal process, whatever goes wrong. What failed, with
//!   the call and the system's error code, is kept for the log and for [`last_failure`](super::last_failure).
//!
//! Not done: a "less privileged" container (LPAC), which also takes away ALL APPLICATION PACKAGES, and which would need
//! entries on more files than the executable. A DLL next to the executable would need an entry as well; the app has
//! none.
#![allow(unsafe_code, reason = "process handles; see each `unsafe` block")]
#![deny(clippy::undocumented_unsafe_blocks)]

mod container;
mod spawn;
mod step;

use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::os::windows::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, TerminateProcess, WaitForSingleObject};

use crate::process::Started;
use crate::process::job::Job;
use crate::view::heic::job::limited;
use crate::view::heic::process::run_started;
use crate::view::{cancelled, passing_refusal};

use self::container::{Sid, allow_read_and_execute, container_sid};
use self::spawn::{Created, create, resume};
use self::step::{at, is_resource, is_unreachable};

/// Set when the container cannot be used on this system for good (no profile can be made, or the container is refused the
/// executable still after it has been given the right to run it): the rest of the run refuses at once, and does not try
/// the entries on the access list again for every picture. Never set by a failure of the moment (no memory, no quota).
static UNAVAILABLE: AtomicBool = AtomicBool::new(false);

/// What failed last, in words, for the tests and the log (never for the page).
static LAST_FAILURE: Mutex<Option<String>> = Mutex::new(None);

/// Keeps `error` as the last failure and logs it. `lasting` says whether it ends the thumbnails of this run.
fn note(what: &str, error: &io::Error, lasting: bool) {
    let text = format!("{what}: {error:?}");
    tracing::warn!(lasting, "{text}");

    *LAST_FAILURE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(text);
    if lasting {
        UNAVAILABLE.store(true, Ordering::Relaxed);
    }
}

/// What failed last in the container (the call, the system's error and its code), if anything did.
pub(super) fn last_failure() -> Option<String> {
    LAST_FAILURE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// How long a stopped helper is waited for.
const STOPPED: Duration = Duration::from_secs(10);

/// A helper in its container: its process and the job that holds it. Dropping it closes both; the job ends what is
/// still in it.
pub(super) struct Contained {
    process: OwnedHandle,
    _job: Job,
}

impl Contained {
    /// Ends the process, whatever its state.
    fn terminate(&self) -> io::Result<()> {
        // SAFETY: the handle is the live process handle this value owns, with the right to end it (the one `CreateProcessW`
        // returns has it); the call only ends the process.
        let ended = unsafe { TerminateProcess(self.process.as_raw_handle(), 1) };
        if ended == 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }
}

impl Started for Contained {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let handle = self.process.as_raw_handle();

        // SAFETY: the handle is the live process handle this value owns; a wait of 0 milliseconds only asks.
        let waited = unsafe { WaitForSingleObject(handle, 0) };
        if waited == WAIT_TIMEOUT {
            return Ok(None);
        }
        if waited != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }

        let mut code = 0_u32;

        // SAFETY: the handle is live and has the right to be asked (a process handle `CreateProcessW` made); `code` is a
        // place for the exit code.
        let asked = unsafe { GetExitCodeProcess(handle, &raw mut code) };
        if asked == 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Some(ExitStatus::from_raw(code)))
    }

    fn kill(&mut self) -> io::Result<()> {
        // A process that has ended already is not an error (as for the standard library's child). A failure to tell is not
        // a reason to leave the process running: it is ended all the same.
        if matches!(self.try_wait(), Ok(Some(_))) {
            return Ok(());
        }

        self.terminate()
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        // SAFETY: the handle is the live process handle this value owns; the wait is bounded.
        let waited = unsafe { WaitForSingleObject(self.process.as_raw_handle(), STOPPED.as_millis() as u32) };
        if waited != WAIT_OBJECT_0 {
            return Err(io::Error::other("the helper did not end"));
        }

        self.try_wait()?
            .ok_or_else(|| io::Error::other("the helper did not end"))
    }
}

/// The error for a system where the container cannot be used now: the thumbnails are refused, and the page shows icons. It
/// is the state of the system and not a verdict on the file, so a cache does not keep it.
fn unavailable() -> io::Error {
    passing_refusal(
        io::ErrorKind::Unsupported,
        "thumbnails are not available on this system",
    )
}

/// The helper `program` with `args` in its container and in a job with `memory` bytes, running, and the ends of its pipes
/// that the app keeps (`stdin`, `stdout`). Never a helper that is not in its container.
pub(super) fn start(program: &Path, args: &[&str], memory: usize) -> io::Result<(Contained, File, File)> {
    if UNAVAILABLE.load(Ordering::Relaxed) {
        return Err(unavailable());
    }

    let job = limited(memory).map_err(|error| {
        note("the job cannot be made", &error, false);

        io::Error::other(at("job", error))
    })?;
    let container = container_sid().map_err(|error| {
        // Only a profile that cannot be made is for good; a machine out of memory may make it the next time.
        note("the container profile cannot be made", &error, !is_resource(&error));

        unavailable()
    })?;
    let Created {
        process,
        thread,
        stdin,
        stdout,
    } = create_allowed(program, args, &container)?;

    // The process is in the job before it runs a byte. If it cannot be put there it is ended, and never resumed.
    let contained = Contained { process, _job: job };
    let contained_in_job = contained._job.assign_handle(contained.process.as_raw_handle());
    if let Err(error) = contained_in_job
        .map_err(|error| at("AssignProcessToJobObject", error))
        .and_then(|()| resume(&thread))
    {
        note("the helper cannot be contained", &error, false);
        let _ = contained.terminate();

        return Err(io::Error::other(error));
    }

    Ok((contained, stdin, stdout))
}

/// [`create`], and if the container cannot reach the executable, again after the container has been given the right to
/// read and run it (see the module): first for the file, and then for the folder that holds it (some failures of the
/// container's start name the folder). The thumbnails are refused for the rest of the run where that does not help.
fn create_allowed(program: &Path, args: &[&str], container: &Sid) -> io::Result<Created> {
    let mut failure = match create(program, args, container) {
        Ok(created) => return Ok(created),
        Err(error) => error,
    };

    for target in [Some(program), program.parent()].into_iter().flatten() {
        if !is_unreachable(&failure) {
            break;
        }
        tracing::debug!("the container cannot reach the helper: it is given the right to");

        if let Err(error) = allow_read_and_execute(target, container) {
            note(
                "the container cannot be given the helper's file",
                &error,
                !is_resource(&error),
            );

            return Err(unavailable());
        }
        match create(program, args, container) {
            Ok(created) => return Ok(created),
            Err(error) => failure = error,
        }
    }

    // What the container is refused for good is one thing; what the system could not do just now is another.
    let lasting = is_unreachable(&failure);
    note("the thumbnail helper did not start in its container", &failure, lasting);
    if lasting || is_resource(&failure) {
        return Err(unavailable());
    }

    Err(io::Error::other(failure))
}

/// The thumbnail's picture from the helper `program` with `args`, which is started in its container (see [`start`]) and is
/// given `bytes`; the answer is read as the HEIC helper's is, with no side over `max_side`.
pub(super) fn run(
    program: &Path,
    args: &[&str],
    bytes: Vec<u8>,
    cancel: &AtomicBool,
    deadline: Duration,
    max_side: u32,
    memory: usize,
) -> io::Result<Vec<u8>> {
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }

    let (contained, stdin, stdout) = start(program, args, memory)?;

    super::helper_answer(run_started(contained, stdin, stdout, bytes, cancel, deadline, max_side))
}
