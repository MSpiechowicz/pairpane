//! What every child process of the core is started and supervised with: the HEIC helper (`view::heic`) and `unrar`
//! (`archive::rar`).
//!
//! A child is never started through a shell: [`command`] takes a fixed program and an argument list and builds a command
//! with a cleared environment, no standard input, a piped standard output (the caller may override it), no standard error,
//! a working directory that is not the app's own and, on Windows, no console window. [`wait_for`] polls a started child
//! against a cancel flag and a deadline, and [`stop`] kills and reaps it; a child that is not waited for to the end is
//! always stopped. [`join_within`] waits for a reading thread without an unbounded wait: a thread whose pipe is held
//! open by something else is left behind.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How often a waiting parent looks at the cancel flag and at the child.
pub const POLL: Duration = Duration::from_millis(25);

/// How long a parent waits for one of the two ends of an answer (the child ending, its output ending) once the other has
/// come, and for a reading thread to end after the child is killed.
pub const GRACE: Duration = Duration::from_secs(2);

/// The variables every child is given, on top of the ones the caller names: a Windows process needs its system directory
/// and its temporary folder. Elsewhere there are none.
const SYSTEM_ENV: &[&str] = &[
    #[cfg(windows)]
    "SystemRoot",
    #[cfg(windows)]
    "WINDIR",
    #[cfg(windows)]
    "TEMP",
    #[cfg(windows)]
    "TMP",
];

/// `CREATE_NO_WINDOW`: the child of a program without a console gets none either.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The command that starts `program` with `args`: the environment is cleared but for [`SYSTEM_ENV`] and the variables in
/// `kept` that the app has, so nothing of the app's own (tokens, proxy settings, a session) reaches the child; the standard
/// input is closed and the standard error goes nowhere; the standard output is a pipe. The caller may change the streams,
/// add variables and set the working directory (the default is the root, so a relative or empty entry of a kept search path
/// does not resolve against wherever the app happens to be).
pub fn command<I, S>(program: &Path, args: I, kept: &[&str]) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    for name in SYSTEM_ENV.iter().chain(kept) {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }

    #[cfg(unix)]
    command.current_dir("/");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        command.creation_flags(CREATE_NO_WINDOW);
    }

    command
}

/// Why [`wait_for`] gave up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    /// The cancel flag was raised.
    Cancelled,
    /// The child ran for the whole deadline.
    Deadline,
    /// One of the two ends (the child, its output) came and the other did not within [`GRACE`].
    Stalled,
    /// The system could not tell whether the child had ended.
    Lost,
}

/// Waits until the child has ended and `output_done` says its output has ended too (a caller with no output passes
/// `|| true`), polling the cancel flag. Returns the child's exit status. The child is not stopped here: the caller does it
/// with [`stop`], on every path.
pub fn wait_for(
    child: &mut Child,
    cancel: &AtomicBool,
    deadline: Duration,
    mut output_done: impl FnMut() -> bool,
) -> Result<ExitStatus, Wait> {
    let started = Instant::now();
    let mut exited = None;
    let mut read = None;

    loop {
        let now = Instant::now();
        if cancel.load(Ordering::Relaxed) {
            return Err(Wait::Cancelled);
        }
        if now.duration_since(started) >= deadline {
            return Err(Wait::Deadline);
        }

        if exited.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => exited = Some((now, status)),
                Ok(None) => {}
                Err(_) => return Err(Wait::Lost),
            }
        }
        if read.is_none() && output_done() {
            read = Some(now);
        }

        match (exited, read) {
            (Some((_, status)), Some(_)) => return Ok(status),
            (Some((first, _)), None) | (None, Some(first)) if now.duration_since(first) >= GRACE => {
                return Err(Wait::Stalled);
            }
            _ => {}
        }

        thread::sleep(POLL);
    }
}

/// Kills and reaps the child; harmless for one that has ended.
pub fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The result of `thread` when it ends within `bound`; `None` when it does not (it is then left to run alone) or when it
/// panicked.
pub fn join_within<T>(thread: JoinHandle<T>, bound: Duration) -> Option<T> {
    let until = Instant::now() + bound;

    while !thread.is_finished() {
        if Instant::now() >= until {
            return None;
        }

        thread::sleep(Duration::from_millis(5));
    }

    thread.join().ok()
}

#[cfg(test)]
mod tests;
