//! The preview of a multi-rename: `rename_preview` computes in Rust what a rule would do to the items of a folder (the new
//! names and what is wrong with them), as the page asks for it while the user types.
//!
//! The rule, a regular expression of it included, is only ever compiled here, within the limits of a search of files (see
//! `pairpane_core::rename`), and the page shows what comes back. The preview reads the folder (two reads of metadata per
//! item), so it runs on a blocking thread, at most [`PREVIEW_THREADS`] at once, because a read on a hung mount never returns
//! and must not use up the thread pool (see `limit.rs`). Starting the job is `ops_task` (see `ops/tasks.rs`).

use std::collections::HashSet;

use pairpane_core::listing::platform;
use pairpane_core::model::{IpcErrorCode, RenamePreview, RenameRule};
use pairpane_core::ops::{MAX_NAMES, MAX_NAMES_BYTES, name_fits};
use pairpane_core::rename::preview_in;
use tauri::State;
use tauri::async_runtime;

use super::limit::ThreadCap;
use super::ops::failed;
use super::panes::{folder_error, resolve};
use crate::error::{CommandError, CommandResult};

/// Preview threads the app may have at once: a stuck one is stepped around by the other, and two stuck ones refuse
/// previews as busy without ever taking more of the thread pool.
const PREVIEW_THREADS: usize = 2;

/// The threads of the previews that run.
#[derive(Default)]
pub struct Renames {
    threads: ThreadCap,
}

impl Renames {
    async fn preview(&self, path: String, names: Vec<String>, rule: RenameRule) -> CommandResult<RenamePreview> {
        let folder = resolve(Some(path), None)?;
        check_names(&names)?;
        let permit = self.threads.acquire(PREVIEW_THREADS).ok_or_else(|| {
            CommandError::new(
                IpcErrorCode::Busy,
                "a folder is not responding; try the preview again shortly",
            )
        })?;

        let previewed = async_runtime::spawn_blocking(move || {
            // The place is held until the preview ends, however it ends.
            let _permit = permit;

            // First of all: a link on the path may lead to a network or device path, which Windows would connect to.
            let actual = platform::local_path(&folder)
                .map_err(|error| failed(folder_error(&error, "read this folder", "Cannot read this folder")))?;

            // The same answer on every system for a folder that is gone (a read of it is not what tells on Linux).
            let is_folder = std::fs::metadata(&actual)
                .map(|metadata| metadata.is_dir())
                .map_err(|error| failed(folder_error(&error, "read this folder", "Cannot read this folder")))?;
            if !is_folder {
                return Err(CommandError::new(IpcErrorCode::InvalidInput, "This is not a folder."));
            }

            Ok(preview_in(&actual, &names, &rule))
        })
        .await;

        previewed.unwrap_or_else(|_| Err(CommandError::new(IpcErrorCode::Internal, "the preview was interrupted")))
    }
}

/// Refuses no names, more than [`MAX_NAMES`], one twice, one that is not a plain name or does not fit, and names that
/// together take more than [`MAX_NAMES_BYTES`]: the size of a name bounds the work on it and the total the work on the list.
fn check_names(names: &[String]) -> CommandResult<()> {
    let refused = |message: &str| CommandError::new(IpcErrorCode::InvalidInput, message);

    if names.is_empty() || names.len() > MAX_NAMES {
        return Err(refused(&format!("choose between 1 and {MAX_NAMES} different items")));
    }

    let mut seen = HashSet::with_capacity(names.len());
    let mut bytes = 0_usize;
    for name in names {
        bytes = bytes.saturating_add(name.len());
        if !name_fits(name) || bytes > MAX_NAMES_BYTES {
            return Err(refused("an item name, or the names together, are too long"));
        }
        if !platform::is_plain_name(name) || !seen.insert(name.as_str()) {
            return Err(refused("an item name must be a single plain name, and chosen once"));
        }
    }

    Ok(())
}

/// What a rename rule would do to the items `names` of the folder `path`: a row for each, in the order given (the counter
/// counts by that position), with the new name and its problem if it has one; or the fault of the rule, with the field
/// it is in. Reads the folder; a rename is started with `ops_task`.
#[tauri::command]
pub async fn rename_preview(
    renames: State<'_, Renames>,
    path: String,
    names: Vec<String>,
    rule: RenameRule,
) -> CommandResult<RenamePreview> {
    renames.preview(path, names, rule).await
}

#[cfg(test)]
mod tests {
    use pairpane_core::model::{CaseChange, CasePart, RenameCounter, RenameProblem};
    use tauri::async_runtime::block_on;

    use super::*;
    use crate::commands::testing::{folder_with, names, root};

    fn rule() -> RenameRule {
        RenameRule {
            name: "[N]-[C]".to_owned(),
            ext: "[E]".to_owned(),
            counter: RenameCounter {
                start: 1,
                step: 1,
                digits: 2,
            },
            search: String::new(),
            replace: String::new(),
            regex: false,
            match_case: true,
            first_only: false,
            case: CaseChange::Keep,
            case_part: CasePart::Both,
        }
    }

    fn preview(path: String, list: &[&str], rule: RenameRule) -> CommandResult<RenamePreview> {
        block_on(Renames::default().preview(path, names(list), rule))
    }

    fn refusal(path: String, list: Vec<String>) -> IpcErrorCode {
        block_on(Renames::default().preview(path, list, rule()))
            .expect_err("refused")
            .code()
    }

    #[test]
    fn the_preview_is_the_cores_for_the_folder_that_was_given() {
        let (_folder, path) = folder_with(&[("a.txt", b"a"), ("b.txt", b"b"), ("a-01.txt", b"x")]);

        let answer = preview(path, &["a.txt", "b.txt"], rule()).expect("a preview");

        let RenamePreview::Rows {
            rows,
            changed,
            problems,
        } = answer
        else {
            panic!("rows");
        };
        assert_eq!(
            rows.iter()
                .map(|row| (row.to.as_str(), row.problem))
                .collect::<Vec<_>>(),
            [("a-01.txt", Some(RenameProblem::Exists)), ("b-02.txt", None)]
        );
        assert_eq!((changed, problems), (1, 1));
    }

    #[test]
    fn a_rule_that_cannot_run_is_answered_with_its_field() {
        let (_folder, path) = folder_with(&[("a.txt", b"a")]);
        let broken = RenameRule {
            name: "[Q]".to_owned(),
            ..rule()
        };

        let RenamePreview::Rule { error } = preview(path, &["a.txt"], broken).expect("an answer") else {
            panic!("a rule fault");
        };

        assert_eq!(error.field, pairpane_core::model::RuleField::Name);
    }

    #[test]
    fn names_and_folders_that_are_not_usable_are_refused() {
        let too_long = "n".repeat(pairpane_core::ops::MAX_NAME_UNITS + 1);

        for bad in [
            vec![],
            vec!["../a".to_owned()],
            vec!["a/b".to_owned()],
            vec![String::new()],
            vec![too_long],
        ] {
            assert_eq!(refusal(root(), bad), IpcErrorCode::InvalidInput);
        }
        assert_eq!(
            refusal(root(), names(&["a", "a"])),
            IpcErrorCode::InvalidInput,
            "chosen once"
        );
        assert_eq!(
            refusal("relative".to_owned(), names(&["a"])),
            IpcErrorCode::InvalidInput
        );
    }

    #[test]
    fn too_many_names_are_refused_before_anything_is_read() {
        let many: Vec<String> = (0..=MAX_NAMES).map(|n| n.to_string()).collect();

        assert_eq!(refusal(root(), many), IpcErrorCode::InvalidInput);
    }

    #[test]
    fn a_folder_that_is_not_there_is_not_found() {
        let (folder, path) = folder_with(&[]);
        drop(folder);

        let refused = preview(path, &["a"], rule()).expect_err("not found");

        assert_eq!(refused.code(), IpcErrorCode::NotFound);
    }

    #[test]
    fn a_third_preview_while_two_hold_their_places_is_refused_as_busy() {
        let renames = Renames::default();
        let _first = renames.threads.acquire(PREVIEW_THREADS).expect("first");
        let _second = renames.threads.acquire(PREVIEW_THREADS).expect("second");

        let refused = block_on(renames.preview(root(), names(&["a"]), rule())).expect_err("busy");

        assert_eq!(refused.code(), IpcErrorCode::Busy);
    }
}
