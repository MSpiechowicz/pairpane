//! The thumbnail helper in an AppContainer on Windows, the sandbox that Edge and Chromium give their renderers.
//!
//! # What the helper is
//!
//! The app's own executable, started with `CreateProcessW` (`spawn.rs`) with the `SECURITY_CAPABILITIES` attribute of a
//! container of a fixed name, `com.mspiechowicz.pairpane.thumbnail`, and no capabilities (`container.rs`). That is a
//! process with a low-integrity token that can use no file, folder or registry key unless its access list names the
//! container (or ALL APPLICATION PACKAGES) for it, that cannot connect to the network or to the loopback, and that has
//! none of the user's other rights. It gets two pipes as its standard input and output and the null device as its standard
//! error and no other handle of the app's (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`), an environment of a few variables of the
//! machine and of the user's own folders (see `spawn.rs`), and no window.
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
//! - **(a), chosen:** when `CreateProcessW` says access denied (and only then: a file or path that is not found stays so
//!   whatever its access list says), it adds one entry to the access list of the executable, for the SID of this
//!   container (and not for ALL APPLICATION PACKAGES, which every packaged app of the user is in): read and execute, on
//!   that one file, no inheritance, and starts again. Nothing is written when the list has the entry already. The list is
//!   read and written through a handle to the file, so nothing is propagated to other objects (a write by name to a
//!   folder would make Windows walk its whole tree, and a portable executable in Downloads or one on a drive root would
//!   change the permissions of everything below it), and the list's protection from inheritance is kept as it was. The
//!   entry lets no other program read anything: the executable is not a secret, and the container is the helper's alone.
//!   It is never removed. It needs the right to change the access list, which the user who installed the app has. An
//!   update that replaces the file drops its entry, and the next start adds it again. This is the rare path: where the
//!   file's folder already allows the container (an install under `Program Files`, a build folder) nothing is written.
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
#[cfg(test)]
mod tests;

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
use self::step::{at, is_denied, is_resource};

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

/// The memory limit of the job, for the tests of this module.
#[cfg(test)]
const MEMORY_FOR_TESTS: usize = super::MEMORY;

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

/// [`create`], and if the container is denied the executable (and only then), once more after the container has been given
/// the right to read and run the file (see the module). Where that does not help the thumbnails are refused for the rest
/// of the run. The start with no grant is the common one: an executable under `Program Files`, which has ALL APPLICATION
/// PACKAGES in its access list by inheritance, and a build folder that allows it.
fn create_allowed(program: &Path, args: &[&str], container: &Sid) -> io::Result<Created> {
    let failure = match create(program, args, container) {
        Ok(created) => return Ok(created),
        Err(error) => error,
    };

    let failure = if is_denied(&failure) {
        tracing::debug!("the container is denied the helper's file: it is given the right to");

        if let Err(error) = allow_read_and_execute(program, container) {
            note(
                "the container cannot be given the helper's file",
                &error,
                !is_resource(&error),
            );

            return Err(unavailable());
        }
        match create(program, args, container) {
            Ok(created) => return Ok(created),
            Err(error) => error,
        }
    } else {
        failure
    };

    // What the container is refused for good is one thing; what the system could not do just now is another.
    let lasting = is_denied(&failure);
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
