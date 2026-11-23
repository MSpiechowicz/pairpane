//! The helper's own end: the child process that reads the status of a repository nobody has vouched for (the parent's end is
//! `helper`). The child is the app's own executable started with [`CHILD_ARG`]; the shell hands it to [`child_main`] before
//! anything else starts: no window, no plugin, nothing but the answer is written to the standard output.
//!
//! What it does, in order:
//!
//! 0. Starts a session of its own (`setsid`, Unix), so that it has no controlling terminal: nothing can be pushed into one with
//!    `TIOCSTI` and it cannot be signalled through one.
//! 1. Closes every descriptor it inherited that is not one of its three streams (Linux, macOS).
//! 2. Reads the request from its standard input, at most [`MAX_REQUEST`] bytes: the folder, the limits and the folders of the
//!    repository the parent looked at (`Layout`). It reads nothing else from anywhere but those folders from here on.
//! 3. Confines itself before gix opens anything (see below). A process that cannot is a process that exits without an
//!    answer: it never reads a repository unconfined where the system has a way to confine it. That includes a Landlock that the
//!    kernel has and that is not enforced, and a folder that is not the one that the parent looked at (its device and inode, in
//!    the request): the HEIC and thumbnail helpers, which hold no repository's files, go on without Landlock as before.
//! 4. Runs the status in-process (`scan::read`: gix opens exactly the `Layout`, and re-checks it with `Layout::is_opened`), and
//!    writes the answer (`request::encode_answer`): the marks, or only the kind of the error.
//!
//! # The confinement
//!
//! - **Linux** (`view/heic/sandbox.rs`): it dies with the app (`PR_SET_PDEATHSIG`), can gain no privileges
//!   (`PR_SET_NO_NEW_PRIVS`), has no core dump, a limit of [`MAX_FILES`] open files, of the processor time the status may take
//!   and a second (its hard limit a second more, so that `SIGXCPU` is delivered and read as a time out), and of [`MEMORY`] of data (the heap and the private writable mappings: not the address space, which would
//!   refuse to map a pack as big as the repository is). Landlock lets it **read only** below the folders of the layout (the work
//!   tree, the git folder, the common folder and the object folders of the alternates), with every network right and the
//!   abstract-socket and signal scopes handled and none allowed, so `/proc` (magic links, `pagemap`) and the rest of the file
//!   system are out of its reach. A seccomp filter, on every thread, is required: it refuses sockets, io_uring, a program and a
//!   process (threads stay), `ptrace` and the like, and every call that writes or changes a file and every open that would
//!   (`Filter::ReadOnly`), where Landlock is not there too. A helper with more than one thread before the confinement does
//!   not go on, as Landlock holds the threads it is set on.
//! - **macOS** (`seatbelt.rs`): the descriptors and limits as above (the data segment is a best effort), and `sandbox_init`
//!   with `(deny default)` and the read of the layout's folders only, with the network denied; the helper does not die with its
//!   parent (macOS has no such signal): a parent that has gone closes its standard input and the helper ends when it writes.
//! - **Windows**: the column is not available there (see `super`): no AppContainer (it would need a change of the access lists of
//!   the user's own repositories, which this must not do) and so nothing that bounds what the helper reads or reaches. The
//!   parent would put the child in a Job Object (`view/heic/job.rs`) that bounds its memory, allows one process and ends it when
//!   the app does (kill-on-close), and nothing starts it: `columns_git` answers `unsupported`. This function is built there for
//!   the tests of what needs no confinement.

use std::io::{self, Read, Write};
use std::sync::atomic::AtomicBool;

use super::request::{MAX_REQUEST, Request, encode_answer};
use super::scan;

/// The first argument with which the app's executable is started as the helper that reads a git status. The shell checks for it
/// before anything else starts and passes the process to [`child_main`].
pub const CHILD_ARG: &str = "--pairpane-git-status";

/// The open files the helper may have: gix keeps an index and a pack open at a time, and a few more for a repository of many.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) const MAX_FILES: u64 = 256;

/// The memory the helper may have above what it starts with, in bytes: what gix holds for one status (the rules, which are
/// counted in a budget of 4 MiB, the index, the trees of `HEAD` walked with a cap of 64 MiB, objects of up to 16 MiB) with
/// room to spare.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) const MEMORY: u64 = 1024 * 1024 * 1024;

/// The entry of the child: reads the request from the standard input, confines itself (on Linux and macOS) and writes the
/// answer to the standard output. Returns the exit code of the process (0 when an answer was written, which an error of the
/// status is too).
pub fn child_main() -> i32 {
    // Before anything else: the descriptors of the app that this process inherited are no use to it and no risk to us.
    #[cfg(target_os = "linux")]
    crate::view::heic::sandbox::close_inherited_fds();
    #[cfg(target_os = "macos")]
    crate::view::thumb::macos::close_inherited_fds();

    // A session of its own, before the repository is read: the helper has no controlling terminal, and so no terminal's input
    // to push characters into (`TIOCSTI`) or to be signalled through.
    #[cfg(unix)]
    if let Err(error) = rustix::process::setsid() {
        tracing::debug!(%error, "the git status helper cannot start a session of its own");

        return 1;
    }

    let Ok(request) = read_request(io::stdin().lock()) else {
        return 1;
    };

    if let Err(error) = confine(&request) {
        tracing::debug!(%error, "the git status helper cannot confine itself");

        return 1;
    }

    answer(&request, io::stdout().lock())
}

/// The request on `input`: at most [`MAX_REQUEST`] bytes, and exactly a request.
pub(super) fn read_request(input: impl Read) -> io::Result<Request> {
    let mut bytes = Vec::new();
    input.take(MAX_REQUEST + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_REQUEST {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the request is too long"));
    }

    Request::decode(&bytes)
}

/// The status of `request` written to `output` as an answer. The exit code of the process: 0 when it was written.
pub(super) fn answer(request: &Request, mut output: impl Write) -> i32 {
    // The parent kills the child at its deadline and ends the wait when the app exits: nothing here is raised.
    let result = scan::read(request, &AtomicBool::new(false));

    match output.write_all(&encode_answer(&result)).and_then(|()| output.flush()) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// The steps of the confinement (see the module), before gix opens anything.
#[cfg(target_os = "linux")]
fn confine(request: &Request) -> io::Result<()> {
    crate::view::heic::sandbox::single_threaded()?;

    apply(request)
}

/// The steps of [`confine`] that come after the check that the process has one thread (the tests, which are run on a thread of
/// a harness, take them without it).
#[cfg(target_os = "linux")]
pub(super) fn apply(request: &Request) -> io::Result<()> {
    use crate::view::heic::sandbox::{self, Filter, Landlock, Memory, Readable};

    sandbox::limit_to(cpu_seconds(request), MAX_FILES)?;

    let readable: Vec<Readable> = request
        .grants
        .iter()
        .map(|grant| Readable {
            path: &grant.path,
            identity: Some((grant.device, grant.inode)),
        })
        .collect();

    // Landlock is required where the kernel has it: a helper that it does not confine does not read the repository.
    sandbox::confine_with(
        Memory::Data(MEMORY),
        Filter::ReadOnly,
        Some(&readable),
        Landlock::Required,
    )
}

#[cfg(target_os = "macos")]
fn confine(request: &Request) -> io::Result<()> {
    super::seatbelt::confine(request, cpu_seconds(request))
}

/// Windows: the parent has put the child in a Job Object (see the module).
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn confine(_request: &Request) -> io::Result<()> {
    Ok(())
}

/// The processor time the helper may take: the time limit of the status, rounded up, and a second: the parent's deadline is
/// earlier, and this is what ends a child that the parent could not.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cpu_seconds(request: &Request) -> u64 {
    let time = request.limits.time;

    time.as_secs() + u64::from(time.subsec_nanos() > 0) + 1
}
