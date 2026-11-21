//! The external editors the user has allowed in this run of the app.
//!
//! `settings.json` is a file the page may be able to plant (a link or a copy made in the config folder through another
//! command, a hard link saved through), and what it names is run as the user. So the first start of a program with an
//! argument template that was not allowed yet is put to the user in a native dialog made by the shell, which the page
//! cannot call (no dialog permission is granted to it). The answer is kept for the run, for that exact pair: a yes for as
//! long as the app runs, a no until `settings.json` changes (its revision, see `pairpane_core::settings`), so that a
//! planted file does not put the same question to the user again and again, and an edit of the file by the user is
//! asked about anew.

use std::collections::HashSet;
use std::sync::{Mutex, TryLockError};

use pairpane_core::settings::ExternalEditor;

/// The longest question put to the user, in characters: one that is longer is not shown (a part of a command line that
/// is cut off could hide what runs), and the editor is not allowed.
const MAX_QUESTION: usize = 2000;

/// A program and its argument template, as they stand in the settings (`{file}` included).
type Pair = (String, Vec<String>);

/// What the user's say is on an external editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    Allowed,
    /// Not allowed: the user said no (now or before, for this settings file), or the question is too long to show.
    Refused,
    /// Another question is open; this one is not put.
    Busy,
}

#[derive(Default)]
struct Memory {
    /// The revision of the settings file the refusals are for.
    revision: u64,
    allowed: HashSet<Pair>,
    refused: HashSet<Pair>,
}

/// The (program, arguments) pairs the user allowed, and those refused for the settings file as it is.
#[derive(Default)]
pub(super) struct Approved {
    memory: Mutex<Memory>,
}

impl Approved {
    /// Whether `editor`, as it stands in the settings file of `revision`, may be started. It may when this exact program
    /// and argument template were allowed before, and may not when they were refused for this revision (nobody is
    /// asked); otherwise `ask` (given the question, which shows what would run) decides and its answer is kept. The lock
    /// is held while the user is asked, so a second question is never put at once: it is [`Verdict::Busy`] instead of
    /// waiting behind the first one.
    pub(super) fn allows(&self, editor: &ExternalEditor, revision: u64, ask: impl FnOnce(String) -> bool) -> Verdict {
        let key = (editor.program.trim().to_owned(), editor.args.clone());
        let mut memory = match self.memory.try_lock() {
            Ok(memory) => memory,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return Verdict::Busy,
        };

        if memory.revision != revision {
            memory.revision = revision;
            memory.refused.clear();
        }

        if memory.allowed.contains(&key) {
            return Verdict::Allowed;
        }

        if memory.refused.contains(&key) {
            return Verdict::Refused;
        }

        let Some(question) = question(&key.0, &key.1) else {
            return Verdict::Refused;
        };

        if ask(question) {
            memory.allowed.insert(key);

            Verdict::Allowed
        } else {
            memory.refused.insert(key);

            Verdict::Refused
        }
    }
}

/// What the user is asked: the program and each argument quoted (so that a space, a quote or a line break in one shows as
/// what it is). `None` when it is too long to show whole.
fn question(program: &str, args: &[String]) -> Option<String> {
    let mut line = format!("{program:?}");
    for arg in args {
        line.push_str(&format!(" {arg:?}"));
    }

    if line.chars().count() > MAX_QUESTION {
        return None;
    }

    Some(format!(
        "Open files with this program?\n\n{line}\n\nIt is run with your permissions. {{file}} is the file's path."
    ))
}

/// The text to give the dialog so that it shows `text` as it is. The dialog of the GTK backend (Linux and the BSDs) takes
/// its description as a `printf` format (rfd 0.17.2, `src/backend/gtk3/message_dialog.rs:112`,
/// `gtk_message_dialog_format_secondary_text`), and what the question holds comes from the settings file: a `%s` or a
/// `%n` in an argument would be read as a directive. Every `%` is doubled. Whoever updates rfd, or turns on its portal
/// backend, must check this line again before removing or keeping the doubling.
#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn dialog_text(text: &str) -> String {
    text.replace('%', "%%")
}

/// The text to give the dialog: as it is, the other backends show it as it is.
#[cfg(not(all(unix, not(target_os = "macos"))))]
pub(super) fn dialog_text(text: &str) -> String {
    text.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(program: &str, args: &[&str]) -> ExternalEditor {
        let args = args.iter().map(ToString::to_string).collect();

        ExternalEditor {
            program: program.into(),
            args,
        }
    }

    fn yes(_question: String) -> bool {
        true
    }

    fn no(_question: String) -> bool {
        false
    }

    fn nobody(_question: String) -> bool {
        panic!("nobody is asked")
    }

    #[test]
    fn a_pair_is_asked_once_and_remembered_when_allowed_whatever_the_file() {
        let approved = Approved::default();
        let code = editor("code", &["--wait", "{file}"]);

        assert_eq!(approved.allows(&code, 1, yes), Verdict::Allowed);

        assert_eq!(approved.allows(&code, 1, nobody), Verdict::Allowed);
        assert_eq!(
            approved.allows(&code, 2, nobody),
            Verdict::Allowed,
            "the same pair in an edited file"
        );
    }

    #[test]
    fn a_refusal_holds_until_the_settings_file_changes() {
        let approved = Approved::default();
        let code = editor("code", &["{file}"]);

        assert_eq!(approved.allows(&code, 1, no), Verdict::Refused);

        assert_eq!(approved.allows(&code, 1, nobody), Verdict::Refused, "not asked again");
        assert_eq!(approved.allows(&code, 1, nobody), Verdict::Refused);

        // The file changed: the user is asked again, and may say yes.
        assert_eq!(approved.allows(&code, 2, yes), Verdict::Allowed);
    }

    #[test]
    fn a_refusal_is_forgotten_when_the_file_changes_even_if_the_pair_is_the_same() {
        let approved = Approved::default();
        let code = editor("code", &["{file}"]);
        assert_eq!(approved.allows(&code, 1, no), Verdict::Refused);

        let mut asked = false;
        let verdict = approved.allows(&code, 2, |_| {
            asked = true;

            false
        });

        assert_eq!(verdict, Verdict::Refused);
        assert!(asked, "asked anew");
        // And that refusal holds for the new revision.
        assert_eq!(approved.allows(&code, 2, nobody), Verdict::Refused);
    }

    #[test]
    fn another_program_or_other_arguments_are_another_pair() {
        let approved = Approved::default();
        assert_eq!(approved.allows(&editor("code", &["{file}"]), 1, yes), Verdict::Allowed);

        assert_eq!(
            approved.allows(&editor("code", &["--other", "{file}"]), 1, no),
            Verdict::Refused
        );
        assert_eq!(approved.allows(&editor("evil", &["{file}"]), 1, no), Verdict::Refused);
        assert_eq!(approved.allows(&editor("code", &[]), 1, no), Verdict::Refused);
    }

    #[test]
    fn the_program_is_taken_as_it_will_run_without_its_outer_spaces() {
        let approved = Approved::default();
        assert_eq!(
            approved.allows(&editor(" code ", &["{file}"]), 1, yes),
            Verdict::Allowed
        );

        assert_eq!(
            approved.allows(&editor("code", &["{file}"]), 1, nobody),
            Verdict::Allowed
        );
    }

    #[test]
    fn a_second_question_while_one_is_open_is_busy_and_does_not_wait() {
        let approved = Approved::default();

        let outer = approved.allows(&editor("code", &["{file}"]), 1, |_| {
            let inner = approved.allows(&editor("other", &["{file}"]), 1, nobody);
            assert_eq!(inner, Verdict::Busy);

            true
        });

        assert_eq!(outer, Verdict::Allowed);
        assert_eq!(approved.allows(&editor("other", &["{file}"]), 1, yes), Verdict::Allowed);
    }

    #[test]
    fn the_question_names_the_program_and_every_argument_quoted() {
        let text = question(
            "my editor",
            &["--wait".to_owned(), "a\nb".to_owned(), "{file}".to_owned()],
        )
        .expect("shown");

        assert!(
            text.contains(r#""my editor" "--wait" "a\nb" "{file}""#),
            "a line break in an argument is escaped, not shown as a line: {text}"
        );
        assert!(text.starts_with("Open files with this program?"));
    }

    #[test]
    fn a_command_too_long_to_show_whole_is_not_allowed_and_nobody_is_asked() {
        let approved = Approved::default();
        let long = editor("code", &[&"x".repeat(MAX_QUESTION)]);

        assert_eq!(approved.allows(&long, 1, nobody), Verdict::Refused);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn every_percent_sign_is_doubled_for_the_dialog_that_reads_its_text_as_a_format() {
        let text = question("code", &["%s%s%n".to_owned(), "100%".to_owned(), "%%".to_owned()]).expect("shown");

        let shown = dialog_text(&text);

        assert!(shown.contains(r#""%%s%%s%%n" "100%%" "%%%%""#), "{shown}");
        // Undoing the doubling, as the dialog does, gives the question back.
        assert_eq!(shown.replace("%%", "%"), text);
        assert!(!shown.replace("%%", "").contains('%'), "a lone % is left: {shown}");
    }

    #[cfg(not(all(unix, not(target_os = "macos"))))]
    #[test]
    fn the_text_is_left_as_it_is_for_a_dialog_that_does_not_read_a_format() {
        let text = question("code", &["%s%s%n".to_owned()]).expect("shown");

        assert_eq!(dialog_text(&text), text);
    }

    #[test]
    fn a_text_without_a_percent_sign_is_never_changed() {
        let text = question("code", &["--wait".to_owned(), "{file}".to_owned()]).expect("shown");

        assert_eq!(dialog_text(&text), text);
    }
}
