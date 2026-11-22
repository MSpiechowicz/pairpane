//! The file tools as jobs: `ops_task` checks what the page sent, like `ops_start`, and queues the job in the same
//! engine, so it shows in the Transfers panel and can be paused and cancelled like a copy.
//!
//! Nothing is read from the disk here. A job that works in place (a checksum file, the check of one, permissions and
//! times, renames) has its own folder as the target, so it waits in the queue of that folder's volume; a split or a combine
//! queues behind the volume of the folder it writes to.

use pairpane_core::model::{AttributeChange, IpcErrorCode, JobId, JobTask, ModeChange};
use pairpane_core::ops::{MIN_PART_SIZE, RenameLog, Request, Task, name_fits};
use tauri::State;

use super::{Ops, checked_names, names_too_large};
use crate::commands::panes::resolve;
use crate::error::{CommandError, CommandResult};

/// The most bits of a permission mask: the nine of owner, group and others, and setuid, setgid and sticky.
const MODE_MASK: u32 = 0o7777;

impl Ops {
    /// Validates the request, then queues it. Touches no disk.
    pub(super) fn task(&self, from: String, names: Vec<String>, task: JobTask) -> CommandResult<JobId> {
        let request = task_request(from, names, task)?;

        self.rename_undo.submit(&self.engine, request)
    }
}

/// Runs a file tool on the `names` of the folder `from` as a job and returns its id at once: a checksum file, the check
/// of one, splitting a file, joining its parts, changing permissions and times, or renaming items to new names (see `JobTask`). Nothing is read
/// from the disk here; a failure ends the job with a failed `ended` event.
#[tauri::command]
pub async fn ops_task(ops: State<'_, Ops>, from: String, names: Vec<String>, task: JobTask) -> CommandResult<JobId> {
    ops.task(from, names, task)
}

/// Checks a request from the webview and builds the engine's: the folders pass `check_folder`, the names pass
/// [`checked_names`], and what the task says is what the system can do.
pub(super) fn task_request(from: String, names: Vec<String>, task: JobTask) -> CommandResult<Request> {
    let from = resolve(Some(from), None)?;
    let sent = names.len();
    let names = checked_names(&from, names)?;

    let (to, task) = match task {
        JobTask::Checksum { algorithm, output } => {
            if !pairpane_core::listing::platform::is_plain_name(&output) {
                return Err(invalid("the name of the checksum file must be a single plain name"));
            }
            if !name_fits(&output) {
                return Err(names_too_large());
            }

            (from.clone(), Task::Checksum { algorithm, output })
        }
        JobTask::VerifyChecksum => {
            exactly_one(&names)?;

            (from.clone(), Task::VerifyChecksum)
        }
        JobTask::Split { to, part_size } => {
            exactly_one(&names)?;
            if part_size < MIN_PART_SIZE {
                return Err(invalid("choose parts of at least 1 KB"));
            }

            (resolve(Some(to), None)?, Task::Split { part_size })
        }
        JobTask::Combine { to } => {
            exactly_one(&names)?;

            (resolve(Some(to), None)?, Task::Combine)
        }
        JobTask::Attributes { change } => {
            check_attributes(&change)?;

            (from.clone(), Task::Attributes(change))
        }
        JobTask::Rename { targets } => {
            // `checked_names` dropped the names that came twice: the new names would no longer line up with the items.
            if targets.len() != sent || names.len() != sent {
                return Err(invalid("choose a new name for each item, and each item once"));
            }

            let task = Task::Rename {
                targets,
                undo: false,
                log: RenameLog::default(),
            };

            (from.clone(), task)
        }
    };

    Ok(Request::task(from, names, to, task))
}

fn exactly_one(names: &[String]) -> CommandResult<()> {
    if names.len() == 1 {
        return Ok(());
    }

    Err(invalid("choose exactly one item"))
}

/// Refuses an attribute change that changes nothing, that sets permissions on Windows or the Windows attributes
/// (read-only, hidden, system, archive) elsewhere (a system has the one kind or the other), or that holds a permission
/// mask with bits beyond `0o7777`.
fn check_attributes(change: &AttributeChange) -> CommandResult<()> {
    if change.changes_nothing() {
        return Err(invalid("choose what to change"));
    }

    if cfg!(windows) && change.mode.is_some() {
        return Err(unsupported("this system has no Unix permissions to set"));
    }
    if !cfg!(windows) && change.sets_windows_attributes() {
        return Err(unsupported(
            "the read-only, hidden, system and archive attributes exist on Windows only",
        ));
    }

    let masks = match change.mode {
        None => [0, 0],
        Some(ModeChange::Set { mode }) => [mode, 0],
        Some(ModeChange::Adjust { add, remove }) => [add, remove],
    };
    if masks.iter().any(|mask| *mask > MODE_MASK) {
        return Err(invalid("a permission mask has bits that do not exist"));
    }

    Ok(())
}

fn invalid(message: &'static str) -> CommandError {
    CommandError::new(IpcErrorCode::InvalidInput, message)
}

fn unsupported(message: &'static str) -> CommandError {
    CommandError::new(IpcErrorCode::Unsupported, message)
}
