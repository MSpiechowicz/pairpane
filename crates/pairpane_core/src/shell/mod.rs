//! The command line of the dock: a line the user typed, run by the user's own shell in a folder, its output streamed.
//!
//! Unlike every other child of the core (see [`crate::process`]), this one is meant to be a shell: the line is the
//! user's, and it runs as they would run it in a terminal, with their environment. On Unix it is `$SHELL -c LINE` (the
//! shell must be an absolute path to a file, else `/bin/sh`), on Windows `%ComSpec% /D /S /C "LINE"` (`cmd.exe`
//! without its AutoRun commands, the line passed as it is). Its standard input is closed, so a command that asks for
//! input ends instead of waiting for ever; its standard output and error are read as they come and sent as text, with
//! terminal escapes and other control characters left out ([`clean`]), up to [`MAX_OUTPUT`] (the rest is read and left
//! out, so the command is never held up by a full pipe).
//!
//! A command can be stopped with everything it started: on Unix it leads a process group of its own, which is sent
//! `SIGTERM` and, [`process::GRACE`] later, `SIGKILL`; on Windows it is held in a Job Object, which is terminated. A
//! program the command leaves running in the background when it ends normally is left alone. Once the shell has ended,
//! its output is waited for at most [`process::GRACE`] (a background program may hold the pipe open), and [`run`]
//! returns.

#[cfg(windows)]
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Instant;

use crate::model::ShellEvent;
use crate::process::{GRACE, POLL};

mod clean;
mod env;
mod terminal;
#[cfg(test)]
mod tests;

pub use clean::Cleaner;
pub use env::appimage_env;
pub use terminal::terminal;
#[cfg(all(test, unix, not(target_os = "macos")))]
use terminal::terminal_from;

/// The most output sent for one command, in bytes of text.
pub const MAX_OUTPUT: usize = 1024 * 1024;

/// The longest line run, in bytes.
pub const MAX_LINE: usize = 8 * 1024;

/// The exit code of what a stop ends on Windows.
#[cfg(windows)]
const STOPPED: u32 = 1;

/// The size of one read of a pipe.
const CHUNK: usize = 8 * 1024;

/// The shell the line is given to, and how: the program and the arguments that come before the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    pub program: PathBuf,
}

impl Shell {
    /// The user's shell: `$SHELL` when it is an absolute path to a file, else `/bin/sh`.
    #[cfg(unix)]
    pub fn of_user() -> Self {
        let program = std::env::var_os("SHELL")
            .map(PathBuf::from)
            .filter(|shell| shell.is_absolute() && shell.is_file())
            .unwrap_or_else(|| PathBuf::from("/bin/sh"));

        Self { program }
    }

    /// The user's shell: `%ComSpec%` when it is an absolute path to a file, else `cmd.exe` of the system folder.
    #[cfg(windows)]
    pub fn of_user() -> Self {
        let program = std::env::var_os("ComSpec")
            .map(PathBuf::from)
            .filter(|shell| shell.is_absolute() && shell.is_file())
            .unwrap_or_else(|| {
                let root = std::env::var_os("SystemRoot").unwrap_or_else(|| OsString::from(r"C:\Windows"));
                PathBuf::from(root).join("System32").join("cmd.exe")
            });

        Self { program }
    }

    /// The command that runs `line` in `folder`, its input closed and its output piped.
    fn command(&self, line: &str, folder: &Path) -> Command {
        let mut command = Command::new(&self.program);
        command
            .current_dir(folder)
            .env("TERM", "dumb")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (name, value) in appimage_env(std::env::vars_os()) {
            match value {
                Some(value) => command.env(name, value),
                None => command.env_remove(name),
            };
        }

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;

            // A line that starts with `-` would be read as options of the shell (`bash -c -i`): a space before it is
            // the same command.
            let line = if line.starts_with('-') {
                format!(" {line}")
            } else {
                line.to_owned()
            };
            command.arg("-c").arg(line).process_group(0);
        }

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;

            // `/S` takes the line between the outer quotes as it is; the quotes are the only change made to it.
            // A program is looked for on the PATH only, not first in the folder the line runs in (which may be one
            // that was downloaded or unpacked): `git` is never a `git.bat` planted there.
            command
                .env("NoDefaultCurrentDirectoryInExePath", "1")
                .args(["/D", "/S", "/C"])
                .raw_arg(format!("\"{line}\""))
                .creation_flags(crate::process::CREATE_NO_WINDOW);
        }

        command
    }
}

/// Why a line is not run: our own texts, which hold nothing the user did not type.
pub fn check_line(line: &str) -> io::Result<()> {
    if line.trim().is_empty() {
        return Err(crate::view::refusal(
            io::ErrorKind::InvalidInput,
            "type a command first",
        ));
    }
    if line.len() > MAX_LINE {
        return Err(crate::view::refusal(
            io::ErrorKind::InvalidInput,
            "the command is too long",
        ));
    }
    if line.contains(['\0', '\n', '\r']) {
        return Err(crate::view::refusal(
            io::ErrorKind::InvalidInput,
            "the command must be one line",
        ));
    }

    Ok(())
}

/// A started command: the child and, on Windows, the job that holds it and what it starts.
struct Running {
    child: Child,
    #[cfg(windows)]
    job: Option<crate::process::job::Job>,
}

impl Running {
    fn start(shell: &Shell, line: &str, folder: &Path) -> io::Result<(Self, ChildStdout, ChildStderr)> {
        let mut child = shell.command(line, folder).spawn()?;

        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            crate::process::stop(&mut child);
            return Err(io::Error::other("the output of the command is not piped"));
        };

        // Without the job a stop ends the shell only; the command still runs, so that is logged, not refused.
        // The job has no limits: a program the command leaves running when it ends normally (`start notepad`) goes on
        // after the job is dropped; only a stop ends what is in it. The command is in the job a moment after it has
        // started (`Command` cannot start it suspended), so a program it starts in that moment is not held.
        #[cfg(windows)]
        let job = crate::process::job::Job::new()
            .and_then(|job| job.assign(&child).map(|()| job))
            .inspect_err(|error| tracing::warn!(%error, "the command is not held in a job"))
            .ok();

        Ok((
            Self {
                child,
                #[cfg(windows)]
                job,
            },
            stdout,
            stderr,
        ))
    }

    /// Whether the shell has ended. On Unix it is not reaped yet (it stays a zombie until [`Running::reap`]), so its
    /// process group id stays reserved and a later signal to the group cannot reach another one.
    fn has_ended(&mut self) -> io::Result<bool> {
        #[cfg(unix)]
        {
            use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};

            let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
            Ok(waitid(WaitId::Pid(Pid::from_child(&self.child)), options)?.is_some())
        }

        #[cfg(windows)]
        {
            Ok(self.child.try_wait()?.is_some())
        }
    }

    /// The exit status of a shell that has ended, which is reaped.
    fn reap(&mut self) -> Option<ExitStatus> {
        self.child.wait().ok()
    }

    /// Asks the command and what it started to end (`SIGTERM` to its group on Unix; on Windows the job is ended at once).
    fn terminate(&mut self) {
        #[cfg(unix)]
        self.signal(rustix::process::Signal::TERM);

        #[cfg(windows)]
        self.kill();
    }

    /// Ends the command and what it started.
    fn kill(&mut self) {
        #[cfg(unix)]
        self.signal(rustix::process::Signal::KILL);

        #[cfg(windows)]
        if let Some(job) = &self.job
            && let Err(error) = job.terminate(STOPPED)
        {
            tracing::debug!(%error, "cannot end the command's job");
        }

        // The shell itself, in case its group or its job could not be reached.
        self.child.kill().ok();
    }

    #[cfg(unix)]
    fn signal(&self, signal: rustix::process::Signal) {
        let group = rustix::process::Pid::from_child(&self.child);
        if let Err(error) = rustix::process::kill_process_group(group, signal) {
            // The group is gone: everything in it has ended.
            tracing::debug!(%error, "cannot signal the command's group");
        }
    }
}

/// How a command is asked to stop: gently ([`Stop::ask`]: `SIGTERM`, then `SIGKILL` [`GRACE`] later), or at once
/// ([`Stop::force`], as the app exits, so that nothing it started outlives it). A stop asked once the command has ended
/// does nothing.
#[derive(Debug, Default)]
pub struct Stop(AtomicU8);

impl Stop {
    const ASKED: u8 = 1;
    const FORCED: u8 = 2;

    pub fn ask(&self) {
        self.0.fetch_max(Self::ASKED, Ordering::Relaxed);
    }

    pub fn force(&self) {
        self.0.store(Self::FORCED, Ordering::Relaxed);
    }

    /// Whether a stop has been asked, gently or not.
    pub fn asked(&self) -> bool {
        self.0.load(Ordering::Relaxed) >= Self::ASKED
    }

    fn forced(&self) -> bool {
        self.0.load(Ordering::Relaxed) >= Self::FORCED
    }
}

/// What a reading thread sends: a chunk read from its stream (0: output, 1: errors), or the end of its pipe.
enum Piece {
    Bytes(usize, Vec<u8>),
    End,
}

/// Reads `pipe` to its end on a thread of its own, sending what it reads to `sender`. Once nobody takes it (the command
/// is over, and a program it left in the background still writes), it is read and dropped: the pipe stays open, so that
/// program is not ended by a broken pipe.
fn read_on_thread(
    mut pipe: impl Read + Send + 'static,
    stream: usize,
    sender: Sender<Piece>,
    name: &str,
) -> io::Result<()> {
    thread::Builder::new().name(name.to_owned()).spawn(move || {
        let mut buffer = vec![0; CHUNK];
        let mut wanted = true;
        loop {
            match pipe.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    if wanted {
                        wanted = sender.send(Piece::Bytes(stream, buffer[..read].to_vec())).is_ok();
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        sender.send(Piece::End).ok();
    })?;

    Ok(())
}

/// Runs `line` with `shell` in `folder`, sending its output to `emit` and ending with one `done`; an `error` (with the
/// io error returned) when it cannot be started. Stops the command as `stop` asks, or when `emit` says the output is
/// no longer wanted (it returns `false`).
pub fn run(
    shell: &Shell,
    line: &str,
    folder: &Path,
    stop: &Stop,
    emit: &mut dyn FnMut(ShellEvent) -> bool,
) -> io::Result<()> {
    check_line(line)?;
    let (mut running, stdout, stderr) = Running::start(shell, line, folder)?;

    let (sender, receiver) = mpsc::channel();
    let readers = read_on_thread(stdout, 0, sender.clone(), "pairpane-shell-out")
        .and_then(|()| read_on_thread(stderr, 1, sender, "pairpane-shell-err"));
    if let Err(error) = readers {
        running.kill();
        running.child.wait().ok();
        return Err(error);
    }

    let mut supervisor = Supervisor {
        running,
        receiver,
        cleaners: [Cleaner::default(), Cleaner::default()],
        sent: 0,
        truncated: false,
        open_pipes: 2,
        stopped_at: None,
        killed: false,
    };
    let status = supervisor.drive(stop, emit);
    let code = status.and_then(|status| status.code());

    emit(ShellEvent::Done {
        code,
        stopped: supervisor.stopped_at.is_some(),
        truncated: supervisor.truncated,
    });

    Ok(())
}

/// Watches a started command: forwards its output, stops it when asked, and sees it end.
struct Supervisor {
    running: Running,
    receiver: Receiver<Piece>,
    /// One for each stream, so that neither's escape or cut character runs into the other's text.
    cleaners: [Cleaner; 2],
    /// Bytes of text sent so far.
    sent: usize,
    truncated: bool,
    open_pipes: u8,
    /// When the command was asked to stop.
    stopped_at: Option<Instant>,
    /// It was killed after not ending in time.
    killed: bool,
}

impl Supervisor {
    /// Runs until the command has ended and its output has (or [`GRACE`] after that, has not); its exit status.
    /// A stop asked while it runs is `SIGTERM` to its group and, [`GRACE`] later, `SIGKILL` to whatever of the group is
    /// left, even when the shell itself has ended by then (what it started may ignore the polite one); a forced stop
    /// is `SIGKILL` at once.
    fn drive(&mut self, stop: &Stop, emit: &mut dyn FnMut(ShellEvent) -> bool) -> Option<ExitStatus> {
        let mut ended_at: Option<Instant> = None;

        loop {
            // A stop counts only while the command runs: one that comes after it ended leaves alone what it left
            // running in the background, and does not say it was stopped.
            if ended_at.is_none() && stop.asked() {
                self.stop();
            }
            let overdue = self.stopped_at.is_some_and(|asked| asked.elapsed() >= GRACE);
            if !self.killed && (overdue || (stop.forced() && (ended_at.is_none() || self.stopped_at.is_some()))) {
                self.running.kill();
                self.killed = true;
            }

            if ended_at.is_none() {
                match self.running.has_ended() {
                    Ok(true) => ended_at = Some(Instant::now()),
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(%error, "cannot wait for the command");
                        self.running.kill();
                        return self.running.reap();
                    }
                }
            }

            // A stopped command is over once its group has been killed.
            let settled = self.stopped_at.is_none() || self.killed;
            if let Some(ended) = ended_at
                && settled
                && (self.open_pipes == 0 || ended.elapsed() >= GRACE)
            {
                break;
            }

            match self.receiver.recv_timeout(POLL) {
                Ok(Piece::Bytes(stream, bytes)) => {
                    if !self.forward(stream, &bytes, emit) && ended_at.is_none() {
                        self.stop();
                    }
                }
                Ok(Piece::End) => self.open_pipes = self.open_pipes.saturating_sub(1),
                Err(RecvTimeoutError::Timeout) => {}
                // Both pipes have closed while the command still runs (it closed them, or `exec`ed a program that did):
                // the channel answers at once from now on, so the wait is a sleep.
                Err(RecvTimeoutError::Disconnected) => {
                    self.open_pipes = 0;
                    thread::sleep(POLL);
                }
            }
        }

        // What is still queued was read before the end.
        while let Ok(piece) = self.receiver.try_recv() {
            if let Piece::Bytes(stream, bytes) = piece {
                self.forward(stream, &bytes, emit);
            }
        }
        for stream in 0..self.cleaners.len() {
            let rest = self.cleaners[stream].finish();
            self.send(rest, emit);
        }

        self.running.reap()
    }

    fn stop(&mut self) {
        if self.stopped_at.is_none() {
            self.stopped_at = Some(Instant::now());
            self.running.terminate();
        }
    }

    /// Sends what `bytes` say as text, up to [`MAX_OUTPUT`] in all. `false` when the page no longer wants it.
    fn forward(&mut self, stream: usize, bytes: &[u8], emit: &mut dyn FnMut(ShellEvent) -> bool) -> bool {
        let text = self.cleaners[stream].push(bytes);
        self.send(text, emit)
    }

    fn send(&mut self, mut text: String, emit: &mut dyn FnMut(ShellEvent) -> bool) -> bool {
        if self.truncated || text.is_empty() {
            return true;
        }

        let room = MAX_OUTPUT - self.sent;
        if text.len() > room {
            let mut cut = room;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            self.truncated = true;
        }
        self.sent += text.len();

        text.is_empty() || emit(ShellEvent::Output { text })
    }
}
