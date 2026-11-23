//! The command line of the dock (`shell_run`, `shell_stop`) and "Open terminal here" (`terminal_open`).
//!
//! What the page is trusted with. `shell_run` runs a line in the user's shell: whatever sends it runs anything the user
//! can. The page is not a trust boundary for the whole app (other commands write files, see `edit.rs`), but no command
//! before this one started a program the user had not chosen. So the first line in a run of the app is put to the user
//! in a native dialog of the shell, which the page cannot call (no dialog permission is granted to it); a yes holds until
//! the app ends, a no refuses that line and the next one asks again (one question at a time). Nothing is remembered
//! between runs: the state file is the page's to write.
//!
//! The line runs as the core's [`shell`] module says: in the folder of the active pane, its input closed, its output
//! streamed as text up to a cap; a stop ends it and what it started. At most [`RUNNING`] commands run at once, and all
//! are stopped as the app exits.
//!
//! `terminal_open` starts no line and no program the page names: the terminal is the one `$TERMINAL` names or a known
//! one (Terminal on macOS, the shell in a console of its own on Windows), with no arguments but the folder on macOS and
//! the folder as its working directory. No text sent to the page holds a path; details go to the log.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use pairpane_core::model::{IpcError, IpcErrorCode, ShellEvent, ShellId};
use pairpane_core::shell::{self, Shell};
use pairpane_core::view::is_refusal;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State, async_runtime};

use super::edit::ask_user;
use super::limit::{Permit, ThreadCap};
use super::panes::resolve;
use crate::error::{CommandError, CommandResult};

#[cfg(test)]
mod tests;

/// Commands that may run at once.
const RUNNING: usize = 4;

/// Terminals being started at once (each only until it has started).
const STARTING: usize = 2;

/// How long the exit waits for the commands to be told to stop.
const EXIT_WAIT: Duration = Duration::from_millis(200);

/// The commands that run, and the user's say on the command line in this run.
#[derive(Default)]
pub struct Shells {
    runs: Mutex<HashMap<ShellId, Arc<AtomicBool>>>,
    ids: AtomicU64,
    running: ThreadCap,
    starting: ThreadCap,
    /// The user allowed the command line in this run.
    allowed: AtomicBool,
    /// Held while the user is asked.
    asking: Mutex<()>,
}

impl Shells {
    /// Whether the command line may run lines: it may once the user has allowed it in this run; otherwise `ask` (given
    /// the question) decides. A second question at once is `busy`, not put.
    fn consent(&self, shell: &Path, ask: impl FnOnce(String) -> bool) -> CommandResult<()> {
        if self.allowed.load(Ordering::Acquire) {
            return Ok(());
        }

        let Ok(_asking) = self.asking.try_lock() else {
            return Err(CommandError::new(IpcErrorCode::Busy, "another question is open"));
        };
        if self.allowed.load(Ordering::Acquire) {
            return Ok(());
        }

        if ask(question(shell)) {
            self.allowed.store(true, Ordering::Release);

            Ok(())
        } else {
            // Not `forbidden`, whose text the page never sees: this one says what happened.
            Err(CommandError::new(
                IpcErrorCode::InvalidInput,
                "the command line was not allowed",
            ))
        }
    }

    /// Takes a place for a command and gives it an id and a stop flag; `busy` when [`RUNNING`] run.
    fn register(&self) -> CommandResult<(ShellId, Arc<AtomicBool>, Permit)> {
        let permit = self.running.acquire(RUNNING).ok_or_else(|| {
            CommandError::new(IpcErrorCode::Busy, "four commands are running; stop one or wait for it")
        })?;

        let id = ShellId(self.ids.fetch_add(1, Ordering::Relaxed) + 1);
        let stop = Arc::new(AtomicBool::new(false));
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
        stop.store(true, Ordering::Relaxed);

        Ok(())
    }

    fn forget(&self, id: ShellId) {
        self.lock().remove(&id);
    }

    /// Stops every command, as the app exits, and waits a moment for them to be told (each is checked every few
    /// milliseconds), so that none outlives the app.
    pub fn shut_down(&self) {
        for stop in self.lock().values() {
            stop.store(true, Ordering::Relaxed);
        }

        let since = Instant::now();
        while !self.lock().is_empty() && since.elapsed() < EXIT_WAIT {
            std::thread::sleep(pairpane_core::process::POLL);
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<ShellId, Arc<AtomicBool>>> {
        self.runs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What the user is asked before the first line of a run.
fn question(shell: &Path) -> String {
    format!(
        "Run the commands you type in Pairpane's command line?\n\nThey run in your shell ({}) with your permissions, in \
         the folder of the active pane. You are asked once each time Pairpane starts.",
        shell.display()
    )
}

/// Runs a command on a blocking thread that holds `permit` until it ends, and drops it from `shells` then.
fn spawn(
    app: AppHandle,
    id: ShellId,
    stop: Arc<AtomicBool>,
    permit: Permit,
    job: (Shell, PathBuf, String),
    on_event: Channel<ShellEvent>,
) {
    async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let (shell, folder, line) = job;
        let mut emit = |event: ShellEvent| on_event.send(event).is_ok();

        if let Err(error) = shell::run(&shell, &line, &folder, &stop, &mut emit) {
            emit(ShellEvent::Error {
                error: start_error(&error),
            });
        }

        app.state::<Shells>().forget(id);
    });
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

/// Starts the terminal in `folder` and lets it run (a thread reaps it when it ends).
fn open_terminal(folder: &Path, place: Permit) -> CommandResult<()> {
    if !folder.is_dir() {
        return Err(CommandError::new(IpcErrorCode::NotFound, "the folder is not there"));
    }

    let (program, args) = shell::terminal(folder).ok_or_else(|| {
        CommandError::new(
            IpcErrorCode::Process,
            "no terminal was found: set TERMINAL to the one you use",
        )
    })?;

    let mut command = Command::new(program);
    command.args(args).current_dir(folder);

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

    let mut child = command.spawn().map_err(|error| {
        tracing::warn!(%error, "cannot start the terminal");

        CommandError::new(IpcErrorCode::Process, "the terminal could not be started")
    })?;
    drop(place);

    let waiter = std::thread::Builder::new()
        .name("pairpane-terminal-wait".to_owned())
        .spawn(move || {
            child.wait().ok();
        });
    if let Err(error) = waiter {
        tracing::warn!(%error, "cannot wait for the terminal");
    }

    Ok(())
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
    async_runtime::spawn_blocking(move || {
        asker
            .state::<Shells>()
            .consent(&program, |question| ask_user(&asker, question))
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

/// Opens the user's terminal in the folder `path`. `process` when there is none or it cannot be started.
#[tauri::command]
pub async fn terminal_open(shells: State<'_, Shells>, path: String) -> CommandResult<()> {
    let folder = resolve(Some(path), None)?;
    let place = shells
        .starting
        .acquire(STARTING)
        .ok_or_else(|| CommandError::new(IpcErrorCode::Busy, "a terminal is being started"))?;

    async_runtime::spawn_blocking(move || open_terminal(&folder, place))
        .await
        .map_err(|_| CommandError::new(IpcErrorCode::Internal, "the terminal could not be started"))?
}
