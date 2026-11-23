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
//! - **(a), chosen:** when the start is denied, it adds one entry to the access list of the executable, for the SID of
//!   this container (and not for ALL APPLICATION PACKAGES, which every packaged app of the user is in): read and
//!   execute, on that one file, no inheritance, and starts again. The entry lets no other program read anything: the
//!   executable is not a secret, and the container is the helper's alone. It needs the right to change the file's access
//!   list, which the user who installed the app has. An update that replaces the file drops the entry, and the next
//!   start adds it again. Where the entry cannot be added, or the start is denied still, the thumbnails of this run are
//!   refused as `Unsupported` ("fail closed"): the page shows icons, and what is refused is remembered.
//! - (b) is that failure: this module never starts the helper as a normal process, whatever goes wrong.
//!
//! Not done: a "less privileged" container (LPAC), which also takes away ALL APPLICATION PACKAGES, and which would need
//! entries on more files than the executable. A DLL next to the executable would need an entry as well; the app has
//! none.
#![allow(unsafe_code, reason = "process handles; see each `unsafe` block")]
#![deny(clippy::undocumented_unsafe_blocks)]

mod container;
mod spawn;

use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::os::windows::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, TerminateProcess, WaitForSingleObject};

use crate::process::Started;
use crate::process::job::Job;
use crate::view::heic::job::limited;
use crate::view::heic::process::run_started;
use crate::view::{cancelled, refusal};

use self::container::{Sid, allow_read_and_execute, container_sid};
use self::spawn::{Created, create, resume};

/// Set when the container cannot be used on this system (no profile, no way to let it run the executable): the rest of
/// the run refuses at once, and does not try the entries on the access list again for every picture.
static UNAVAILABLE: AtomicBool = AtomicBool::new(false);

/// How long a stopped helper is waited for.
const STOPPED: Duration = Duration::from_secs(10);

/// A helper in its container: its process and the job that holds it. Dropping it closes both; the job ends what is
/// still in it.
pub(super) struct Contained {
    process: OwnedHandle,
    _job: Job,
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
        // A process that has ended already is not an error (as for the standard library's child).
        if self.try_wait()?.is_some() {
            return Ok(());
        }

        // SAFETY: the handle is the live process handle this value owns, with the right to end it (the one `CreateProcessW`
        // returns has it); the call only ends the process.
        let ended = unsafe { TerminateProcess(self.process.as_raw_handle(), 1) };
        if ended == 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
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

/// The error for a system where the container cannot be used: the thumbnails are refused, and the page shows icons.
fn unavailable() -> io::Error {
    refusal(
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
        tracing::debug!(%error, "the thumbnail helper's job cannot be made");

        io::Error::other("the helper cannot be contained")
    })?;
    let container = container_sid().map_err(|error| {
        tracing::warn!(%error, "the thumbnail container cannot be made: no thumbnails on this system");
        UNAVAILABLE.store(true, Ordering::Relaxed);

        unavailable()
    })?;
    let Created {
        process,
        thread,
        stdin,
        stdout,
    } = create_allowed(program, args, &container)?;

    // The process is in the job before it runs a byte. If it cannot be put there it is ended, and never resumed.
    let mut contained = Contained { process, _job: job };
    let contained_in_job = contained._job.assign_handle(contained.process.as_raw_handle());
    if let Err(error) = contained_in_job.and_then(|()| resume(&thread)) {
        tracing::debug!(%error, "the thumbnail helper cannot be contained");
        let _ = contained.kill();

        return Err(io::Error::other("the helper cannot be contained"));
    }

    Ok((contained, stdin, stdout))
}

/// [`create`], and if the container is denied the executable, once more after the executable's access list has been
/// given the container (see the module). Where that cannot be done the thumbnails are refused for the rest of the run.
fn create_allowed(program: &Path, args: &[&str], container: &Sid) -> io::Result<Created> {
    match create(program, args, container) {
        Ok(created) => return Ok(created),
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
            tracing::debug!("the container may not run the helper's file: it is given the right to");
        }
        Err(error) => {
            tracing::debug!(%error, "the thumbnail helper did not start");

            return Err(io::Error::other("the helper did not start"));
        }
    }

    let given = allow_read_and_execute(program, container).and_then(|()| create(program, args, container));

    given.map_err(|error| {
        tracing::warn!(%error, "the thumbnail container cannot run the helper: no thumbnails on this system");
        UNAVAILABLE.store(true, Ordering::Relaxed);

        unavailable()
    })
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

    run_started(contained, stdin, stdout, bytes, cancel, deadline, max_side)
}
