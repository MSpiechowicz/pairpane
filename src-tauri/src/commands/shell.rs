//! The command line of the dock (`shell_run`, `shell_stop`) and "Open terminal here" (`terminal_open`).
//!
//! What the page is trusted with. `shell_run` runs a line in the user's shell: whatever sends it runs anything the user
//! can. The page is not a trust boundary for the whole app (other commands write files, see `edit.rs`), but no command
//! before this one started a program the user had not chosen. So the first line in a run of the app is put to the user
//! in a native dialog of the shell, which the page cannot call (no dialog permission is granted to it), showing the line
//! itself and that a yes holds for the next ones; the answer, a no too, holds until the app ends (see [`consent`]).
//! Nothing is remembered between runs: the state file is the page's to write.
//!
//! The line runs as the core's [`shell`] module says: in the folder of the active pane, its input closed, its output
//! streamed as text up to a cap; a stop ends it and what it started. At most [`RUNNING`] commands run at once, and all
//! are stopped as the app exits.
//!
//! `terminal_open` starts no line and no program the page names: the terminal is the one `$TERMINAL` names or a known
//! one found in the system's own folders (Terminal on macOS, the shell in a console of its own on Windows), with no
//! arguments but the folder on macOS and the folder as its working directory. Still, a program file can be planted
//! (the page can write files), so the first start of each terminal program in a run is put to the user the same way,
//! showing its path. At most [`TERMINALS`] are open at once. No text sent to the page holds a path; details go to the
//! log.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use pairpane_core::model::{IpcError, IpcErrorCode, ShellEvent, ShellId};
use pairpane_core::shell::{self, Shell, Stop};
use pairpane_core::view::is_refusal;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State, async_runtime};

use super::edit::ask_user;
use super::limit::{Permit, ThreadCap};
use super::panes::resolve;
use crate::error::{CommandError, CommandResult};

mod consent;
#[cfg(test)]
mod tests;

use consent::{Consent, line_question, terminal_question};

/// Commands that may run at once.
const RUNNING: usize = 4;

/// Terminals open at once (each holds its place until it is closed, when the thread that waits for it ends).
const TERMINALS: usize = 8;

/// How long the exit waits for the commands to be told to stop.
const EXIT_WAIT: Duration = Duration::from_millis(200);

/// The commands that run, and the user's say on the command line in this run.
#[derive(Default)]
pub struct Shells {
    runs: Mutex<HashMap<ShellId, Arc<Stop>>>,
    ids: AtomicU64,
    running: ThreadCap,
    terminals: ThreadCap,
    /// The user's say on the command line, by shell.
    lines: Consent<PathBuf>,
    /// The user's say on terminal programs, by program and arguments.
    programs: Consent<(OsString, Vec<OsString>, String)>,
    /// The app is closing: no command or terminal starts any more.
    closing: AtomicBool,
}

impl Shells {
    /// Whether the command line may run `line` with `shell`: it may once the user has allowed it in this run (and may
    /// not once they refused); otherwise `ask` (given the question, which shows the line) decides.
    fn consent(&self, shell: &Path, line: &str, ask: impl FnOnce(String) -> bool) -> CommandResult<()> {
        self.lines.allows(
            shell.to_owned(),
            || line_question(shell, line),
            ask,
            "the command line was not allowed (until Pairpane is started again)",
        )
    }

    /// Takes a place for a command and gives it an id and a stop flag; `busy` when [`RUNNING`] run.
    fn register(&self) -> CommandResult<(ShellId, Arc<Stop>, Permit)> {
        self.open()?;
        let permit = self.running.acquire(RUNNING).ok_or_else(|| {
            CommandError::new(IpcErrorCode::Busy, "four commands are running; stop one or wait for it")
        })?;

        let id = ShellId(self.ids.fetch_add(1, Ordering::Relaxed) + 1);
        let stop = Arc::new(Stop::default());
        self.lock().insert(id, Arc::clone(&stop));

        Ok((id, stop, permit))
    }

    /// Stops the command `id`; an unknown one (it has ended) is `notFound`.
    fn stop(&self, id: ShellId) -> CommandResult<()> {
        let stop = self
            .lock()
            .get(&id)
            .cloned()
            .ok_or_else(|| CommandError::new(IpcErrorCode::NotFound, "this command has ended"))?;
        stop.ask();

        Ok(())
    }

    /// Refused once the app is closing (a command that got past its question then would outlive it).
    fn open(&self) -> CommandResult<()> {
        if self.closing.load(Ordering::Acquire) {
            return Err(CommandError::new(IpcErrorCode::Busy, "Pairpane is closing"));
        }

        Ok(())
    }

    fn forget(&self, id: ShellId) {
        self.lock().remove(&id);
    }

    /// Kills every command and what it started, as the app exits (not the polite stop, which waits
    /// [`GRACE`](pairpane_core::process::GRACE) for a
    /// command that ignores it), and waits a moment for them to be killed: each sees it within a poll of its thread.
    pub fn shut_down(&self) {
        self.closing.store(true, Ordering::Release);
        for stop in self.lock().values() {
            stop.force();
        }

        let since = Instant::now();
        while !self.lock().is_empty() && since.elapsed() < EXIT_WAIT {
            std::thread::sleep(pairpane_core::process::POLL);
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<ShellId, Arc<Stop>>> {
        self.runs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Runs a command on a blocking thread that holds `permit` until it ends, and drops it from `shells` then.
fn spawn(
    app: AppHandle,
    id: ShellId,
    stop: Arc<Stop>,
    permit: Permit,
    job: (Shell, PathBuf, String),
    on_event: Channel<ShellEvent>,
) {
    async_runtime::spawn_blocking(move || {
        let _permit = permit;
        // The command is forgotten however this ends, a panic included, so that the exit does not wait for it.
        let _forget = Forget(app, id);
        let (shell, folder, line) = job;
        let mut emit = |event: ShellEvent| on_event.send(event).is_ok();

        if let Err(error) = shell::run(&shell, &line, &folder, &stop, &mut emit) {
            emit(ShellEvent::Error {
                error: start_error(&error),
            });
        }
    });
}

/// Forgets a command of `Shells` when it is dropped.
struct Forget(AppHandle, ShellId);

impl Drop for Forget {
    fn drop(&mut self) {
        self.0.state::<Shells>().forget(self.1);
    }
}

/// What the page is told when a line cannot run: our own texts as they are, else a fixed one (the details, which may
/// hold a path, go to the log).
fn start_error(error: &io::Error) -> IpcError {
    if is_refusal(error) {
        return IpcError::new(IpcErrorCode::InvalidInput, error.to_string());
    }

    tracing::warn!(%error, "cannot run the command");
    let text = match error.kind() {
        io::ErrorKind::NotFound => "the shell or the folder is not there",
        io::ErrorKind::PermissionDenied => "the folder cannot be opened: no permission",
        _ => "the command could not be started",
    };

    IpcError::new(IpcErrorCode::Process, text)
}

/// Starts the terminal in `folder` once the user has allowed its program, and lets it run: a thread reaps it when it
/// ends, holding `place` until then.
fn open_terminal(shells: &Shells, folder: &Path, place: Permit, ask: impl FnOnce(String) -> bool) -> CommandResult<()> {
    if !folder.is_dir() {
        return Err(CommandError::new(IpcErrorCode::NotFound, "the folder is not there"));
    }

    let (program, args) = shell::terminal(folder).ok_or_else(|| {
        CommandError::new(
            IpcErrorCode::Process,
            "no terminal was found: set TERMINAL to the one you use",
        )
    })?;

    // The arguments the user is asked about are the program's own: the folder (macOS) changes with every call. The
    // answer is for the program's file as it is: one that is replaced or changed since is asked about again.
    let fixed: Vec<OsString> = args
        .iter()
        .filter(|arg| !Path::new(arg).starts_with(folder))
        .cloned()
        .collect();
    shells.programs.allows(
        (program.clone(), fixed.clone(), file_identity(Path::new(&program))),
        || terminal_question(&program, &fixed),
        ask,
        "this terminal was not allowed (until Pairpane is started again)",
    )?;

    shells.open()?;
    let mut command = Command::new(program);
    command.args(args).current_dir(folder);
    // As for the command line: a program typed in the terminal is looked for on the PATH, not in the folder first.
    #[cfg(windows)]
    command.env("NoDefaultCurrentDirectoryInExePath", "1");
    shell::apply_appimage_env(&mut command);

    // A console program started by the app (which has none) gets a console of its own, and its input and output.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        command.creation_flags(CREATE_NEW_CONSOLE);
    }
    #[cfg(not(windows))]
    {
        use std::process::Stdio;

        command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    }

    let mut child = pairpane_core::process::spawn(&mut command).map_err(|error| {
        tracing::warn!(%error, "cannot start the terminal");

        CommandError::new(IpcErrorCode::Process, "the terminal could not be started")
    })?;

    let waiter = std::thread::Builder::new()
        .name("pairpane-terminal-wait".to_owned())
        .spawn(move || {
            let _place = place;
            child.wait().ok();
        });
    if let Err(error) = waiter {
        tracing::warn!(%error, "cannot wait for the terminal");
    }

    Ok(())
}

/// What a program file is, to tell when it has been replaced or changed: its device and inode (Unix), size and time of
/// change. Empty when it cannot be read (it is then not started either).
fn file_identity(program: &Path) -> String {
    let Ok(meta) = std::fs::metadata(program) else {
        return String::new();
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        format!(
            "{}:{}:{}:{}.{}:{}.{}",
            meta.dev(),
            meta.ino(),
            meta.size(),
            meta.mtime(),
            meta.mtime_nsec(),
            meta.ctime(),
            meta.ctime_nsec()
        )
    }

    #[cfg(not(unix))]
    {
        format!("{}:{:?}", meta.len(), meta.modified().ok())
    }
}

/// Runs `command` in the user's shell in the folder `path` and answers with its id at once; the events follow on
/// `on_event`: `output` as it comes, then one `done`, or one `error` when it could not be started. The first line of a
/// run asks the user in a native dialog; a refusal is `invalidInput`. A line that is empty, too long or of several lines,
/// or a folder that is not absolute, is `invalidInput`; a fifth command at once, `busy`.
#[tauri::command]
pub async fn shell_run(
    app: AppHandle,
    shells: State<'_, Shells>,
    path: String,
    command: String,
    on_event: Channel<ShellEvent>,
) -> CommandResult<ShellId> {
    shell::check_line(&command).map_err(|error| CommandError::new(IpcErrorCode::InvalidInput, error.to_string()))?;
    let folder = resolve(Some(path), None)?;
    let user_shell = Shell::of_user();

    let asker = app.clone();
    let program = user_shell.program.clone();
    let line = command.clone();
    async_runtime::spawn_blocking(move || {
        asker
            .state::<Shells>()
            .consent(&program, &line, |question| ask_user(&asker, question))
    })
    .await
    .map_err(|_| CommandError::new(IpcErrorCode::Internal, "the question could not be put"))??;

    let (id, stop, permit) = shells.register()?;
    spawn(app, id, stop, permit, (user_shell, folder, command), on_event);

    Ok(id)
}

/// Stops the command `id` and what it started (its `done` says `stopped`). An unknown id (it has ended) is `notFound`.
#[tauri::command]
pub async fn shell_stop(shells: State<'_, Shells>, id: ShellId) -> CommandResult<()> {
    shells.stop(id)
}

/// Opens the user's terminal in the folder `path`. The first start of a terminal program in a run asks the user in a
/// native dialog; a refusal is `invalidInput`. `process` when there is none or it cannot be started; `busy` when
/// [`TERMINALS`] are open.
#[tauri::command]
pub async fn terminal_open(app: AppHandle, shells: State<'_, Shells>, path: String) -> CommandResult<()> {
    shells.open()?;
    let folder = resolve(Some(path), None)?;
    let place = shells
        .terminals
        .acquire(TERMINALS)
        .ok_or_else(|| CommandError::new(IpcErrorCode::Busy, "eight terminals are open; close one first"))?;

    async_runtime::spawn_blocking(move || {
        open_terminal(&app.state::<Shells>(), &folder, place, |question| {
            ask_user(&app, question)
        })
    })
    .await
    .map_err(|_| CommandError::new(IpcErrorCode::Internal, "the terminal could not be started"))?
}
