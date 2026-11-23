//! What every child process of the core is started and supervised with: the HEIC helper (`view::heic`), the thumbnail helper
//! (`view::thumb`), the git status helper (`gitstatus`) and `unrar` (`archive::rar`).
//!
//! A child is never started through a shell: [`command`] takes a fixed program and an argument list and builds a command
//! with a cleared environment, no standard input, a piped standard output (the caller may override it), no standard error,
//! a working directory that is not the app's own and, on Windows, no console window. [`wait_for`] polls a started child
//! whose output is read by a thread, [`wait_for_exit`] one whose output nobody reads, each against a cancel flag and a
//! deadline, and [`stop`] kills and reaps it; a child that is not waited for to the end is
//! always stopped. [`join_within`] waits for a reading thread without an unbounded wait: a thread whose pipe is held
//! open by something else is left behind.

use std::ffi::OsStr;
use std::io;
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
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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

/// Held while a process is started, on Windows: a handle that is inheritable while it is made (the pipes of the thumbnail
/// helper in its container, see `view::thumb::appcontainer`) is inherited by any process that is started at the same time
/// by the standard library, which inherits every handle that is inheritable. So every start of a process by this app's own
/// code goes through [`spawn`] (the HEIC helper, `unrar`, the command line, the external editor, the terminal), and none
/// runs while that helper's pipes are made inheritable. What is outside the lock: Tauri's restart (`app_relaunch`), the
/// `ShellExecute` calls of the opener and the updater, and WebView2's own processes. They can start a process in the few
/// microseconds the pipes are inheritable and inherit them; the helper's `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` keeps the
/// sandbox safe regardless, as it names the only handles the helper itself gets.
#[cfg(windows)]
pub(crate) static SPAWN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Starts `command`, under [`SPAWN_LOCK`] on Windows.
pub fn spawn(command: &mut Command) -> io::Result<Child> {
    #[cfg(windows)]
    let _held = SPAWN_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

    command.spawn()
}

/// A process this module can wait for and stop: the standard library's [`Child`], and on Windows the helper that is started
/// in an AppContainer with `CreateProcessW` (`view::thumb::windows`), which `Command` cannot do on stable Rust.
pub trait Started {
    /// The exit status when the process has ended, `None` while it runs.
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>>;

    /// Ends the process.
    fn kill(&mut self) -> io::Result<()>;

    /// Waits for the process to end (after [`Started::kill`], which makes that quick) and reaps it.
    fn wait(&mut self) -> io::Result<ExitStatus>;
}

impl Started for Child {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Child::try_wait(self)
    }

    fn kill(&mut self) -> io::Result<()> {
        Child::kill(self)
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        Child::wait(self)
    }
}

/// Sets the soft and the hard limit of `resource` for the calling process to `value`: what the helper processes do to
/// themselves before they read a byte (`view::heic::sandbox`, `view::thumb::macos`).
#[cfg(unix)]
pub(crate) fn lower_limit(resource: rustix::process::Resource, value: u64) -> io::Result<()> {
    let limit = rustix::process::Rlimit {
        current: Some(value),
        maximum: Some(value),
    };

    Ok(rustix::process::setrlimit(resource, limit)?)
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

/// How often the periodic check of [`wait_for_exit`] runs.
const WATCH_EVERY: Duration = Duration::from_millis(500);

/// Why [`wait_for_exit`] gave up.
#[derive(Debug)]
pub enum Exit {
    /// The wait itself ended: cancelled, past the deadline or lost.
    Gave(Wait),
    /// The periodic check failed; its error.
    Watch(io::Error),
}

/// Waits until the child has ended and `output_done` says its output has ended too, polling the cancel flag. Returns the
/// child's exit status. One of the two ends coming without the other within [`GRACE`] is [`Wait::Stalled`]. The child is
/// not stopped here: the caller does it with [`stop`], on every path. A child whose output nobody reads is waited for
/// with [`wait_for_exit`].
pub fn wait_for(
    child: &mut impl Started,
    cancel: &AtomicBool,
    deadline: Duration,
    mut output_done: impl FnMut() -> bool,
) -> Result<ExitStatus, Wait> {
    match drive(child, cancel, deadline, Some(&mut output_done), &mut || Ok(())) {
        Ok(status) => Ok(status),
        Err(Exit::Gave(wait)) => Err(wait),
        Err(Exit::Watch(_)) => Err(Wait::Lost),
    }
}

/// Waits until a child whose output nobody reads (it is closed or goes to a file) has ended, polling the cancel flag, and
/// runs `watch` every half second meanwhile; a check that fails gives up with its error. There is no [`GRACE`]: the child
/// may run until the deadline. The child is not stopped here either.
pub fn wait_for_exit(
    child: &mut impl Started,
    cancel: &AtomicBool,
    deadline: Duration,
    mut watch: impl FnMut() -> io::Result<()>,
) -> Result<ExitStatus, Exit> {
    drive(child, cancel, deadline, None, &mut watch)
}

/// The loop of both waits; `output` is the reader's end, when there is a reader.
fn drive(
    child: &mut impl Started,
    cancel: &AtomicBool,
    deadline: Duration,
    mut output: Option<&mut dyn FnMut() -> bool>,
    watch: &mut dyn FnMut() -> io::Result<()>,
) -> Result<ExitStatus, Exit> {
    let started = Instant::now();
    let mut watched = started;
    let mut exited = None;
    let mut read = None;

    loop {
        let now = Instant::now();
        if cancel.load(Ordering::Relaxed) {
            return Err(Exit::Gave(Wait::Cancelled));
        }
        if now.duration_since(started) >= deadline {
            return Err(Exit::Gave(Wait::Deadline));
        }

        if exited.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => exited = Some((now, status)),
                Ok(None) => {}
                Err(_) => return Err(Exit::Gave(Wait::Lost)),
            }
        }
        if read.is_none() {
            match output.as_mut() {
                Some(done) => {
                    if done() {
                        read = Some(now);
                    }
                }
                // Nothing is read, so there is nothing to wait for.
                None => read = Some(now),
            }
        }

        match (exited, read) {
            (Some((_, status)), Some(_)) => return Ok(status),
            (Some((first, _)), None) | (None, Some(first))
                if output.is_some() && now.duration_since(first) >= GRACE =>
            {
                return Err(Exit::Gave(Wait::Stalled));
            }
            _ => {}
        }

        if exited.is_none() && now.duration_since(watched) >= WATCH_EVERY {
            watched = now;
            watch().map_err(Exit::Watch)?;
        }

        thread::sleep(POLL);
    }
}

/// Kills and reaps the child; harmless for one that has ended.
pub fn stop(child: &mut impl Started) {
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

#[cfg(windows)]
pub(crate) mod job;
#[cfg(test)]
mod tests;
