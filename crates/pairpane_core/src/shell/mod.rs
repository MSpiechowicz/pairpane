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

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Instant;

use crate::model::ShellEvent;
use crate::process::{GRACE, POLL};

mod clean;
#[cfg(test)]
mod tests;

pub use clean::Cleaner;

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

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;

            command.arg("-c").arg(line).process_group(0);
        }

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;

            // `/S` takes the line between the outer quotes as it is; the quotes are the only change made to it.
            command
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

/// What a reading thread sends: a chunk read, or the end of its pipe.
enum Piece {
    Bytes(Vec<u8>),
    End,
}

/// Reads `pipe` to its end on a thread of its own, sending what it reads to `sender`.
fn read_on_thread(mut pipe: impl Read + Send + 'static, sender: Sender<Piece>, name: &str) -> io::Result<()> {
    thread::Builder::new().name(name.to_owned()).spawn(move || {
        let mut buffer = vec![0; CHUNK];
        loop {
            match pipe.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    if sender.send(Piece::Bytes(buffer[..read].to_vec())).is_err() {
                        return;
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
/// io error returned) when it cannot be started. Stops the command when `stop` is raised, or when `emit` says the
/// output is no longer wanted (it returns `false`).
pub fn run(
    shell: &Shell,
    line: &str,
    folder: &Path,
    stop: &AtomicBool,
    emit: &mut dyn FnMut(ShellEvent) -> bool,
) -> io::Result<()> {
    check_line(line)?;
    let (mut running, stdout, stderr) = Running::start(shell, line, folder)?;

    let (sender, receiver) = mpsc::channel();
    let readers = read_on_thread(stdout, sender.clone(), "pairpane-shell-out")
        .and_then(|()| read_on_thread(stderr, sender, "pairpane-shell-err"));
    if let Err(error) = readers {
        running.kill();
        running.child.wait().ok();
        return Err(error);
    }

    let mut supervisor = Supervisor {
        running,
        receiver,
        cleaner: Cleaner::default(),
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
    cleaner: Cleaner,
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
    fn drive(&mut self, stop: &AtomicBool, emit: &mut dyn FnMut(ShellEvent) -> bool) -> Option<ExitStatus> {
        let mut status = None;
        let mut ended_at: Option<Instant> = None;

        loop {
            if self.stopped_at.is_none() && stop.load(Ordering::Relaxed) {
                self.stop();
            }
            if let Some(asked) = self.stopped_at
                && !self.killed
                && status.is_none()
                && asked.elapsed() >= GRACE
            {
                self.running.kill();
                self.killed = true;
            }

            if status.is_none() {
                match self.running.child.try_wait() {
                    Ok(Some(exit)) => {
                        status = Some(exit);
                        ended_at = Some(Instant::now());
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(%error, "cannot wait for the command");
                        self.running.kill();
                        self.running.child.wait().ok();
                        return None;
                    }
                }
            }

            if let Some(ended) = ended_at
                && (self.open_pipes == 0 || ended.elapsed() >= GRACE)
            {
                break;
            }

            match self.receiver.recv_timeout(POLL) {
                Ok(Piece::Bytes(bytes)) => {
                    if !self.forward(&bytes, emit) {
                        self.stop();
                    }
                }
                Ok(Piece::End) => self.open_pipes = self.open_pipes.saturating_sub(1),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => self.open_pipes = 0,
            }
        }

        // What is still queued was read before the end.
        while let Ok(piece) = self.receiver.try_recv() {
            if let Piece::Bytes(bytes) = piece {
                self.forward(&bytes, emit);
            }
        }
        let rest = self.cleaner.finish();
        self.send(rest, emit);

        status
    }

    fn stop(&mut self) {
        if self.stopped_at.is_none() {
            self.stopped_at = Some(Instant::now());
            self.running.terminate();
        }
    }

    /// Sends what `bytes` say as text, up to [`MAX_OUTPUT`] in all. `false` when the page no longer wants it.
    fn forward(&mut self, bytes: &[u8], emit: &mut dyn FnMut(ShellEvent) -> bool) -> bool {
        let text = self.cleaner.push(bytes);
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

/// The program a terminal is opened with in a folder, with its arguments (the folder is also its working directory).
/// `None` when none is found.
pub fn terminal(folder: &Path) -> Option<(OsString, Vec<OsString>)> {
    terminal_from(folder, std::env::var_os("TERMINAL"), &|name| on_path(name))
}

/// The terminals tried in order on Linux and the BSDs, when `$TERMINAL` names none.
#[cfg(all(unix, not(target_os = "macos")))]
const TERMINALS: &[&str] = &[
    "x-terminal-emulator",
    "gnome-terminal",
    "konsole",
    "xfce4-terminal",
    "kitty",
    "alacritty",
    "wezterm",
    "foot",
    "xterm",
];

/// [`terminal`], from what `$TERMINAL` holds and a lookup of a name on the `PATH`, so that it can be tested.
#[cfg(all(unix, not(target_os = "macos")))]
fn terminal_from(
    _folder: &Path,
    variable: Option<OsString>,
    find: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<(OsString, Vec<OsString>)> {
    // `$TERMINAL` is the user's own choice: a name on the PATH or an absolute path, with no arguments.
    let chosen = variable.and_then(|value| {
        let value = value.into_string().ok()?;
        let path = PathBuf::from(&value);
        if path.is_absolute() {
            return path.is_file().then_some(path);
        }
        (!value.is_empty() && !value.contains('/'))
            .then(|| find(&value))
            .flatten()
    });

    chosen
        .or_else(|| TERMINALS.iter().find_map(|name| find(name)))
        .map(|program| (program.into_os_string(), Vec::new()))
}

/// [`terminal`] on macOS: Terminal opened on the folder.
#[cfg(target_os = "macos")]
fn terminal_from(
    folder: &Path,
    _variable: Option<OsString>,
    _find: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<(OsString, Vec<OsString>)> {
    Some((
        OsString::from("/usr/bin/open"),
        vec![
            OsString::from("-a"),
            OsString::from("Terminal"),
            folder.as_os_str().to_owned(),
        ],
    ))
}

/// [`terminal`] on Windows: the shell in a console of its own (its working directory is the folder).
#[cfg(windows)]
fn terminal_from(
    _folder: &Path,
    _variable: Option<OsString>,
    _find: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<(OsString, Vec<OsString>)> {
    Some((Shell::of_user().program.into_os_string(), vec![OsString::from("/K")]))
}

/// The file `name` stands for on the `PATH`: the first folder that has it as a file.
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;

    std::env::split_paths(&path)
        .filter(|folder| folder.is_absolute())
        .map(|folder| folder.join(name))
        .find(|candidate| candidate.is_file())
}
