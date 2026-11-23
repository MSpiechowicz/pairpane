//! The user's say, in a native dialog the page cannot call, on what the command line and "Open terminal here" start.
//!
//! Each question is about one thing (the command line with the user's shell, one terminal program with its arguments)
//! and its answer holds for that thing until the app ends: a yes, and a no too, so that a page cannot put the same
//! question again and again until it gets a yes. One question is put at a time; another at once is `busy`.

use std::collections::HashSet;
use std::ffi::OsString;
use std::hash::Hash;
use std::path::Path;
use std::sync::{Mutex, TryLockError};

use pairpane_core::model::IpcErrorCode;

use crate::error::{CommandError, CommandResult};

/// The longest question put to the user, in characters: one that is longer is not shown (a part of a command line that
/// is cut off could hide what runs), and the thing is not allowed.
const MAX_QUESTION: usize = 2000;

struct Answers<K> {
    allowed: HashSet<K>,
    refused: HashSet<K>,
}

/// The answers the user gave in this run, by what they are about.
pub(super) struct Consent<K> {
    answers: Mutex<Answers<K>>,
}

impl<K> Default for Consent<K> {
    fn default() -> Self {
        Self {
            answers: Mutex::new(Answers {
                allowed: HashSet::new(),
                refused: HashSet::new(),
            }),
        }
    }
}

impl<K: Eq + Hash> Consent<K> {
    /// Whether `key` may be started: as the user answered before in this run, or else as `ask` answers `question`
    /// (refused when there is none: it is too long to show). `refused` is the text of a no.
    pub(super) fn allows(
        &self,
        key: K,
        question: impl FnOnce() -> Option<String>,
        ask: impl FnOnce(String) -> bool,
        refused: &'static str,
    ) -> CommandResult<()> {
        // A question that panicked is over: its lock is taken again, not left as one open for good.
        let mut answers = match self.answers.try_lock() {
            Ok(answers) => answers,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => {
                return Err(CommandError::new(IpcErrorCode::Busy, "another question is open"));
            }
        };
        // Not `forbidden`, whose text the page never sees: this one says what happened.
        let no = || CommandError::new(IpcErrorCode::InvalidInput, refused);

        if answers.allowed.contains(&key) {
            return Ok(());
        }
        if answers.refused.contains(&key) {
            return Err(no());
        }

        let Some(question) = question() else {
            return Err(CommandError::new(
                IpcErrorCode::InvalidInput,
                "the command is too long to be shown in the question",
            ));
        };

        if ask(question) {
            answers.allowed.insert(key);
            Ok(())
        } else {
            answers.refused.insert(key);
            Err(no())
        }
    }
}

/// What the user is asked before the first line of a run: the line itself (quoted, so that a space or a quote in it
/// shows as what it is), and that a yes holds for the lines after it. `None` when it is too long to show whole.
pub(super) fn line_question(shell: &Path, line: &str) -> Option<String> {
    let question = format!(
        "Run this command in your shell?\n\n{line:?}\n\nIt runs in {:?} with your permissions, in the folder of the active \
         pane. If you allow it, the next commands you type in Pairpane's command line run without asking, until \
         Pairpane ends; if not, the command line stays off until then.",
        shell.display().to_string()
    );

    (question.chars().count() <= MAX_QUESTION).then_some(question)
}

/// What the user is asked before a terminal program is first started: the program and its arguments, quoted.
pub(super) fn terminal_question(program: &OsString, args: &[OsString]) -> Option<String> {
    let mut line = format!("{program:?}");
    for arg in args {
        line.push_str(&format!(" {arg:?}"));
    }

    let question = format!(
        "Open terminals with this program?\n\n{line}\n\nIt runs with your permissions, in the folder of the active pane. \
         The answer holds until Pairpane ends."
    );

    (question.chars().count() <= MAX_QUESTION).then_some(question)
}
