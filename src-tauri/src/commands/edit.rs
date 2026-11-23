//! The editors: the built-in one reads and saves a file whole (`edit_open`, `edit_save`), the external one is another
//! program that F4 starts (`edit_external`, from the user's `settings.json`), and `settings_open` opens that file.
//!
//! Like the viewer's `view_open`, what the page sent is checked on the command's thread and a request that fails the
//! check is refused before a thread is taken; the disk is touched only on a blocking thread that holds a place of a
//! [`ThreadCap`] until it ends, because a read on a hung mount never returns. The buffer lives in the page: nothing is
//! kept here between commands, and a save finds the file again by its folder and name.
//!
//! The external editor is started by argument vector, never through a shell, with its input and output closed; a thread
//! waits for it so that it never lingers as a zombie, and no more than [`MAX_RUNNING`] run at once. It runs in the user's
//! home folder.
//!
//! What the page is and is not trusted with. The page is not a trust boundary for the whole app: other commands (`ops_*`)
//! can write files, and a link or a copy of a settings file made elsewhere is read as settings. So the checks here
//! narrow what the editor commands add, and the last word on running a program is the user's.
//!
//! - `edit_save` writes only a file that `edit_open` gave the page as text in this run ([`opened`]; a file that was
//!   deleted since may be made again with `force`, a file never opened may not), so it is no way to write a file the user
//!   did not open in the editor; it refuses a target in the app's config folder and one that is the same file as
//!   `settings.json` or `state.json` (a hard link to either, kept anywhere, is found by device and inode), and the
//!   page never writes `settings.json` (it is the user's file): `settings_open` only creates it with the defaults when it
//!   is missing and hands it to the system's own program, and only when it is a plain `.json` file.
//! - The first start of a program with an argument template that was not allowed in this run is put to the user in a
//!   native dialog of the shell ([`approval`]), which the page cannot call (no dialog permission is granted); a pair
//!   that is refused is not started, and not asked about again until `settings.json` changes. That stops a planted
//!   settings file from running a program. The question is put on a thread of its own ([`LAUNCH_THREADS`]) so that one
//!   that is not answered never holds up opening and saving.
//!
//! No text sent to the page holds a path; details go to the log.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use pairpane_core::edit;
use pairpane_core::model::{EditOpened, EditSaveOptions, EditSaved, ExternalEdit, IpcErrorCode};
use pairpane_core::ops::name_fits;
use pairpane_core::settings::{self, SETTINGS_FILE};
use pairpane_core::view::is_refusal;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

use super::items::{begin, blocking, in_folder};
use super::limit::ThreadCap;
use super::ops::{check_name, names_too_large};
use super::panes::resolve;
use super::state::STATE_FILE;
use super::view::view_error;
use crate::error::{CommandError, CommandResult};
use crate::window::MAIN_WINDOW;

mod approval;
#[cfg(test)]
mod guard_tests;
mod opened;
#[cfg(test)]
mod tests;

use approval::{Approved, Verdict, dialog_text};
use opened::Opened;

/// Threads for opening and saving; a stuck one is stepped around by the other.
const EDIT_THREADS: usize = 2;

/// Threads for starting an external editor, apart from [`EDIT_THREADS`]: the thread that starts one may wait for the
/// user's answer for as long as the dialog is open, and must not take a place that opening and saving need.
const LAUNCH_THREADS: usize = 1;

/// External editors that may run at once; another F4 is told the app is busy until one has ended.
const MAX_RUNNING: usize = 8;

/// What the page is told when the user did not allow the external editor (a refusal of the dialog, or a command line too
/// long to show whole). It is `invalidInput`, not `forbidden`: a `forbidden` error is shown with a generic text, and the
/// user is to be told what was refused.
const NOT_ALLOWED: &str = "the external editor was not allowed";

/// The caps on the blocking threads, the external editors that were allowed and are running, and where the settings
/// file is.
pub struct Editor {
    threads: ThreadCap,
    /// The places of the threads that start an external editor (and may wait for the user's answer).
    launches: ThreadCap,
    /// The places of the external editors that run; the thread that waits for one holds its place until it ends.
    running: ThreadCap,
    approved: Arc<Approved>,
    /// The files `edit_open` gave the page as text, the only ones `edit_save` writes.
    opened: Arc<Opened>,
    /// `None` when the system has no config folder: no external editor is set, and there are no settings to open.
    settings: Option<PathBuf>,
}

impl Editor {
    /// An editor whose settings file is `settings`.
    pub fn new(settings: Option<PathBuf>) -> Self {
        Self {
            threads: ThreadCap::default(),
            launches: ThreadCap::default(),
            running: ThreadCap::default(),
            approved: Arc::default(),
            opened: Arc::default(),
            settings,
        }
    }

    /// The folder of the settings file, where a save is refused.
    fn config_dir(&self) -> Option<PathBuf> {
        self.settings.as_deref()?.parent().map(Path::to_owned)
    }

    /// Reads the file `name` of the folder `path` as text on a blocking thread, in the encoding `encoding` labels or the
    /// one found. A file that is read as text may be saved afterwards.
    async fn open(&self, path: String, name: String, encoding: Option<String>) -> CommandResult<EditOpened> {
        let folder = checked(path, &name)?;
        let permit = begin(&self.threads, EDIT_THREADS)?;
        let opened = Arc::clone(&self.opened);

        in_folder(permit, folder, move |actual| {
            let opening = edit::open(actual, &name, encoding.as_deref())?;

            if matches!(opening, EditOpened::Text { .. })
                && let Some(file) = opened::key(actual, &name)
            {
                opened.remember(file);
            }

            Ok(opening)
        })
        .await?
        .map_err(|error| view_error(&error))
    }

    /// Writes `text` over the file `name` of the folder `path` as `options` say, on a blocking thread. Only a file that
    /// was opened as text in this run is written (nothing is touched for another), and nothing in the config folder, nor
    /// any file that is the settings or the saved state under another name.
    async fn save(
        &self,
        path: String,
        name: String,
        text: String,
        options: EditSaveOptions,
    ) -> CommandResult<EditSaved> {
        let folder = checked(path, &name)?;
        let config = self.config_dir();
        let permit = begin(&self.threads, EDIT_THREADS)?;
        let opened = Arc::clone(&self.opened);

        let outcome = in_folder(permit, folder, move |actual| {
            if !opened::key(actual, &name).is_some_and(|file| opened.contains(&file)) {
                return Ok(Outcome::NotOpened);
            }

            if config.as_deref().is_some_and(|config| protected(actual, &name, config)) {
                return Ok(Outcome::Protected);
            }

            edit::save(actual, &name, &text, &options).map(Outcome::Saved)
        })
        .await?
        .map_err(|error| save_error(&error))?;

        match outcome {
            Outcome::Saved(saved) => Ok(saved),
            Outcome::NotOpened => Err(CommandError::new(
                IpcErrorCode::Forbidden,
                "only a file opened in the editor is saved: open it again",
            )),
            Outcome::Protected => Err(CommandError::new(
                IpcErrorCode::Forbidden,
                "the settings and the saved state are not saved into",
            )),
        }
    }

    /// Starts the external editor of the settings on the file `name` of the folder `path`; `NotConfigured` when the
    /// settings have none (the page then opens the built-in one). A program that was not allowed in this run is
    /// put to the user first, by `ask` (given the question; on the blocking thread, so it may wait for the answer).
    async fn external(
        &self,
        path: String,
        name: String,
        ask: impl FnOnce(String) -> bool + Send + 'static,
    ) -> CommandResult<ExternalEdit> {
        let folder = checked(path, &name)?;
        let Some(settings) = self.settings.clone() else {
            return Ok(ExternalEdit::NotConfigured);
        };
        let permit = self.launches.acquire(LAUNCH_THREADS).ok_or_else(|| {
            CommandError::new(
                IpcErrorCode::Busy,
                "an external editor is being started; try again shortly",
            )
        })?;
        let launcher = Launcher {
            approved: Arc::clone(&self.approved),
            running: self.running.clone(),
        };

        in_folder(permit, folder, move |actual| {
            launcher.launch(&settings, &actual.join(&name), ask)
        })
        .await?
        .map_err(|error| external_error(&error))
    }

    /// Makes the settings file with the defaults when it is missing and has `open` open it (with the path as text).
    async fn settings(&self, open: impl FnOnce(String) -> Result<(), String> + Send + 'static) -> CommandResult<()> {
        let Some(file) = self.settings.clone() else {
            return Err(CommandError::new(
                IpcErrorCode::Unsupported,
                "there is no place to keep settings on this system",
            ));
        };
        let permit = begin(&self.threads, EDIT_THREADS)?;

        blocking(permit, move || {
            settings::ensure(&file).map_err(|error| {
                tracing::warn!(%error, "cannot create the settings file");

                CommandError::new(IpcErrorCode::Io, "the settings file cannot be created")
            })?;

            let not_opened = || CommandError::new(IpcErrorCode::Process, "the settings file could not be opened");
            // What was checked is what is handed over: the file the path leads to, not the path again (a link swapped in
            // between could otherwise make it open something else).
            let Some(real) = plain_json(&file) else {
                tracing::warn!("the settings file is not a plain .json file; not opened");

                return Err(CommandError::new(
                    IpcErrorCode::Process,
                    "the settings file cannot be opened",
                ));
            };

            let Some(text) = real.to_str() else {
                return Err(not_opened());
            };

            open(text.to_owned()).map_err(|detail| {
                tracing::warn!(detail, "cannot open the settings file");

                not_opened()
            })
        })
        .await?
    }
}

/// What a save came to on its thread.
enum Outcome {
    Saved(EditSaved),
    /// The file was not opened as text in this run.
    NotOpened,
    /// The file is the settings or the saved state, or in their folder.
    Protected,
}

/// The file the settings path at `file` leads to (a link is followed), if it is a regular file whose name ends in
/// `.json`: the system's own program is handed nothing else, so a link planted in its place cannot make it open (and so
/// run) some other file.
fn plain_json(file: &Path) -> Option<PathBuf> {
    let real = fs::canonicalize(file).ok()?;

    let json = real
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));

    (json && fs::metadata(&real).is_ok_and(|meta| meta.is_file())).then_some(real)
}

/// The folder of a request, after the checks that need no disk: the same as the viewer's (a folder path that passes the
/// folder rules and one plain name that is not too long).
fn checked(path: String, name: &str) -> CommandResult<PathBuf> {
    let folder = resolve(Some(path), None)?;
    if !name_fits(name) {
        return Err(names_too_large());
    }

    // Also refuses a name that is not a single plain name.
    check_name(&folder, name)?;

    Ok(folder)
}

/// Whether `folder` is `config` or inside it, by where they are on disk (a link to either is followed).
fn inside(folder: &Path, config: &Path) -> bool {
    let real = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());

    real(folder).starts_with(real(config))
}

/// Whether the file `name` of `folder` is off limits to a save: it is in the config folder, or it is the very file of
/// the settings or of the saved state (by device and inode, so a hard link made elsewhere is found).
fn protected(folder: &Path, name: &str, config: &Path) -> bool {
    if inside(folder, config) {
        return true;
    }

    let target = folder.join(name);

    [SETTINGS_FILE, STATE_FILE]
        .iter()
        .any(|kept| edit::is_same_file(&target, &config.join(kept)))
}

/// What starting an external editor needs from the editor.
struct Launcher {
    approved: Arc<Approved>,
    running: ThreadCap,
}

impl Launcher {
    /// Starts the program of the settings at `settings` on `file`, an absolute path, once the user has allowed it (see
    /// [`approval`]) and fewer than [`MAX_RUNNING`] editors run. A thread waits for it to end, so it leaves no zombie,
    /// and the program is never waited on here: the editor may run for hours.
    fn launch(&self, settings: &Path, file: &Path, ask: impl FnOnce(String) -> bool) -> io::Result<ExternalEdit> {
        let (loaded, revision) = settings::load_with_revision(settings)?;
        let Some(editor) = loaded.editor() else {
            return Ok(ExternalEdit::NotConfigured);
        };

        edit::check_regular(file)?;

        let place = self.running.acquire(MAX_RUNNING).ok_or_else(|| {
            pairpane_core::view::refusal(io::ErrorKind::WouldBlock, "too many external editors are running")
        })?;
        match self.approved.allows(editor, revision, ask) {
            Verdict::Allowed => {}
            Verdict::Refused => {
                return Err(pairpane_core::view::refusal(
                    io::ErrorKind::PermissionDenied,
                    NOT_ALLOWED,
                ));
            }
            Verdict::Busy => {
                return Err(pairpane_core::view::refusal(
                    io::ErrorKind::WouldBlock,
                    "another question is open",
                ));
            }
        }

        let (program, args) = settings::argv(editor, file);
        let mut child = Command::new(program)
            .args(args)
            .current_dir(start_folder())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                tracing::warn!(%error, "cannot start the external editor");

                pairpane_core::view::refusal(io::ErrorKind::Other, "the external editor could not be started")
            })?;

        let waiter = std::thread::Builder::new()
            .name("pairpane-editor-wait".to_owned())
            .spawn(move || {
                // The place is given back when the editor ends.
                let _place = place;
                child.wait().ok();
            });
        if let Err(error) = waiter {
            tracing::warn!(%error, "cannot wait for the external editor");
        }

        Ok(ExternalEdit::Launched)
    }
}

/// The folder an external editor starts in: the user's home, or the root when there is none, never the app's own.
fn start_folder() -> PathBuf {
    std::env::home_dir()
        .filter(|home| home.is_dir())
        .unwrap_or_else(|| PathBuf::from(std::path::MAIN_SEPARATOR_STR))
}

/// What the page is told when saving fails: our own texts (they hold no path) as they are, else a fixed text per kind
/// of failure, never the system's wording. The details go to the log.
fn save_error(error: &io::Error) -> CommandError {
    if is_refusal(error) {
        let code = match error.kind() {
            io::ErrorKind::InvalidInput => IpcErrorCode::InvalidInput,
            io::ErrorKind::NotFound => IpcErrorCode::NotFound,
            _ => IpcErrorCode::Io,
        };

        return CommandError::new(code, error.to_string());
    }

    match error.kind() {
        io::ErrorKind::NotFound => CommandError::new(IpcErrorCode::NotFound, "the file no longer exists"),
        io::ErrorKind::PermissionDenied => {
            CommandError::new(IpcErrorCode::Io, "the file cannot be written: no permission")
        }
        _ => {
            tracing::debug!(%error, "editor save failed");

            CommandError::new(IpcErrorCode::Io, "the file cannot be saved")
        }
    }
}

/// What the page is told when F4 cannot start the external editor: `invalidInput` when the user did not allow it, `busy`
/// when too many run, `process` for a program that did not start, and otherwise the refusals of the settings and of the
/// file as the viewer tells them.
fn external_error(error: &io::Error) -> CommandError {
    if is_refusal(error) {
        let code = match error.kind() {
            io::ErrorKind::PermissionDenied => Some(IpcErrorCode::InvalidInput),
            io::ErrorKind::WouldBlock => Some(IpcErrorCode::Busy),
            io::ErrorKind::Other => Some(IpcErrorCode::Process),
            _ => None,
        };

        if let Some(code) = code {
            return CommandError::new(code, error.to_string());
        }
    }

    view_error(error)
}

/// Asks the user, in a native dialog of the shell, whether the external editor `question` names may run. Blocks until
/// the answer, so it is called on a blocking thread only. The dialog is modal over the main window.
pub(super) fn ask_user(app: &AppHandle, question: String) -> bool {
    let dialog = app
        .dialog()
        .message(dialog_text(&question))
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancel);

    match app.get_webview_window(MAIN_WINDOW) {
        Some(window) => dialog.parent(&window).blocking_show(),
        None => dialog.blocking_show(),
    }
}

/// Starts the external editor of the settings on the file `name` of the folder `path` (an absolute path to the file,
/// as one argument, and no shell); `notConfigured` when the settings have none, and the page opens the built-in editor.
/// The first start of each program with its argument template asks the user in a native dialog.
#[tauri::command]
pub async fn edit_external(
    app: AppHandle,
    editor: State<'_, Editor>,
    path: String,
    name: String,
) -> CommandResult<ExternalEdit> {
    editor
        .external(path, name, move |question| ask_user(&app, question))
        .await
}

/// Reads the file `name` of the folder `path` as text for the built-in editor, in the encoding `encoding` labels or
/// the one found from the file.
#[tauri::command]
pub async fn edit_open(
    editor: State<'_, Editor>,
    path: String,
    name: String,
    encoding: Option<String>,
) -> CommandResult<EditOpened> {
    editor.open(path, name, encoding).await
}

/// Saves `text` over the file `name` of the folder `path` as `options` say (see `pairpane_core::edit::save`).
#[tauri::command]
pub async fn edit_save(
    editor: State<'_, Editor>,
    path: String,
    name: String,
    text: String,
    options: EditSaveOptions,
) -> CommandResult<EditSaved> {
    editor.save(path, name, text, options).await
}

/// Opens the settings file with the system's default program, making it with the defaults first when it is missing.
#[tauri::command]
pub async fn settings_open(app: AppHandle, editor: State<'_, Editor>) -> CommandResult<()> {
    editor
        .settings(move |file| {
            app.opener()
                .open_path(file, None::<&str>)
                .map_err(|error| error.to_string())
        })
        .await
}
