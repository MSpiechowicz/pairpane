//! The `known_hosts` files `ssh -G` prints. A list is its entries joined with a space, and not quoted: a path with a space
//! in it is several words, which are joined again here (a path starts at a word that is absolute, the words after it
//! that are not absolute are the rest of it). A leading `~/` (older and Windows builds leave it) and `__PROGRAMDATA__`
//! (Windows) are filled in.
//!
//! The joining is a guess, and a wrong one would read a file that `ssh` does not (`UserKnownHostsFile /a/kh rel_kh` names
//! two files, `/a/kh` and `rel_kh`, and so does `/a/my home/kh` when `/a/my` is a file). So when a word that is not
//! absolute would be added to a path that exists in any form (a file, a folder, a device: `symlink_metadata` finds it) or
//! is no file (`/dev/null`, `NUL`), the other reading is as likely as this one, and the list is refused; so is a word
//! with a `.` or `..` component. A path that was joined and is not there is not guessed at any further: it is in `Listed::joined`,
//! and a key is not saved to it unless its folder exists (see `Files::writable`).

use std::io;
use std::path::{Path, PathBuf};

use crate::error::fail;

/// The most files of one list that are read for a host; a longer list is refused, not cut.
pub(super) const MAX_FILES: usize = 8;

/// What is known about the system the paths are on.
pub(super) struct Rules<'a> {
    pub(super) home: &'a Path,
    /// Paths are Windows paths (a drive or `\\`), not Unix ones.
    pub(super) windows: bool,
    /// `%ProgramData%`, for `__PROGRAMDATA__`.
    pub(super) program_data: Option<&'a str>,
}

impl<'a> Rules<'a> {
    /// The rules of the system this runs on.
    pub(super) fn here(home: &'a Path, program_data: Option<&'a str>) -> Self {
        Self {
            home,
            windows: cfg!(windows),
            program_data,
        }
    }

    /// `word` as the start of a path (made absolute where `ssh` left a `~/` or `__PROGRAMDATA__` in it), or `None` when
    /// it is not the start of one.
    fn start_of(&self, word: &str) -> Option<String> {
        if let Some(rest) = word
            .strip_prefix("~/")
            .or_else(|| self.windows.then(|| word.strip_prefix("~\\")).flatten())
        {
            return Some(self.home.join(rest).to_string_lossy().into_owned());
        }

        if !self.windows {
            return word.starts_with('/').then(|| word.to_owned());
        }

        if let Some(rest) = word.strip_prefix("__PROGRAMDATA__") {
            return self.program_data.map(|data| format!("{data}{rest}"));
        }

        let mut chars = word.chars();
        let drive = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.next() == Some(':')
            && matches!(chars.next(), Some('/' | '\\'));

        (drive || word.starts_with("\\\\") || word.eq_ignore_ascii_case("nul")).then(|| word.to_owned())
    }

    /// Whether `path` is no file (`/dev/null`, and `NUL` on Windows).
    fn is_nothing(&self, path: &str) -> bool {
        path == "/dev/null" || (self.windows && path.eq_ignore_ascii_case("nul"))
    }

    /// Whether `word` has a `.` or `..` component (a path of `ssh` is never tidied here).
    fn steps(&self, word: &str) -> bool {
        word.split(|c| c == '/' || (self.windows && c == '\\'))
            .any(|part| part == "." || part == "..")
    }

    /// A list whose words cannot be told apart from the paths of two files.
    fn ambiguous() -> io::Error {
        fail(
            io::ErrorKind::InvalidInput,
            "ssh_config names a known_hosts file Pairpane cannot place (a relative path or a path with spaces next to another file); connect with ssh",
        )
    }

    fn refusal(&self) -> io::Error {
        if self.windows {
            fail(
                io::ErrorKind::InvalidInput,
                "the ssh found on PATH is not Windows OpenSSH; install the OpenSSH client feature",
            )
        } else {
            fail(
                io::ErrorKind::InvalidInput,
                "ssh_config names a known_hosts file that Pairpane cannot find; connect with ssh",
            )
        }
    }
}

/// The files of a list, and which of them were made of several words.
#[derive(Debug, Default)]
pub(super) struct Listed {
    pub(super) paths: Vec<PathBuf>,
    /// The paths that were joined from more than one word (a guess).
    pub(super) joined: Vec<PathBuf>,
}

/// The files of a list that `ssh -G` printed, without repeats. `none` and `/dev/null` are no file. A list that does not
/// start with an absolute path, one that has `none` among other entries, one whose words could be the paths of other
/// files (see the top of this file) and one of more than [`MAX_FILES`] files are refused (fixed messages): where a path
/// points depends on where `ssh` ran, and a file that is left out could be the one that holds the key that stops a
/// connection.
pub(super) fn files(words: &[String], rules: &Rules<'_>) -> io::Result<Listed> {
    if let [only] = words
        && only == "none"
    {
        return Ok(Listed::default());
    }

    let mut paths: Vec<(String, bool)> = Vec::new();

    for word in words {
        if word == "none" {
            return Err(rules.refusal());
        }

        // A `__PROGRAMDATA__` path always starts a new file; with no `%ProgramData%` to put in, it cannot be placed.
        if rules.windows && word.starts_with("__PROGRAMDATA__") && rules.program_data.is_none() {
            return Err(Rules::ambiguous());
        }

        if let Some(path) = rules.start_of(word) {
            paths.push((path, false));
        } else if let Some((last, joined)) = paths.last_mut() {
            if rules.is_nothing(last) || Path::new(last.as_str()).symlink_metadata().is_ok() || rules.steps(word) {
                return Err(Rules::ambiguous());
            }

            last.push(' ');
            last.push_str(word);
            *joined = true;
        } else {
            return Err(rules.refusal());
        }
    }

    let mut files: Vec<PathBuf> = Vec::new();
    let mut joined: Vec<PathBuf> = Vec::new();

    for (path, was_joined) in paths.iter().filter(|(path, _)| !rules.is_nothing(path)) {
        let path = PathBuf::from(path);

        if *was_joined && !joined.contains(&path) {
            joined.push(path.clone());
        }

        if !files.contains(&path) {
            files.push(path);
        }
    }

    if files.len() > MAX_FILES {
        return Err(fail(
            io::ErrorKind::InvalidInput,
            "ssh_config names too many known_hosts files; connect with ssh",
        ));
    }

    Ok(Listed { paths: files, joined })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split(' ').map(str::to_owned).collect()
    }

    fn unix() -> Rules<'static> {
        Rules {
            home: Path::new("/home/me"),
            windows: false,
            program_data: None,
        }
    }

    fn windows() -> Rules<'static> {
        Rules {
            home: Path::new("C:\\Users\\me"),
            windows: true,
            program_data: Some("C:\\ProgramData"),
        }
    }

    fn listed(rules: &Rules<'_>, text: &str) -> Vec<String> {
        files(&words(text), rules)
            .expect("files")
            .paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_path_with_spaces_is_joined_again_and_the_next_absolute_word_starts_a_new_path() {
        assert_eq!(
            listed(&unix(), "/tmp/my home/.ssh/kh /etc/ssh/known /a b  c/d"),
            ["/tmp/my home/.ssh/kh", "/etc/ssh/known", "/a b  c/d"]
        );
        assert_eq!(listed(&unix(), "/x /x /dev/null"), ["/x"]);
    }

    #[test]
    fn none_is_no_file_alone_and_a_refusal_among_others() {
        assert_eq!(listed(&unix(), "none"), Vec::<String>::new());
        assert_eq!(listed(&unix(), "/dev/null"), Vec::<String>::new());

        for text in ["none /x", "/x none", "relative/kh", "/x /y none", ""] {
            assert_eq!(
                files(&words(text), &unix()).expect_err(text).kind(),
                io::ErrorKind::InvalidInput,
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_leading_tilde_is_the_home_folder() {
        assert_eq!(
            listed(&unix(), "~/.ssh/known_hosts ~/my files/kh"),
            ["/home/me/.ssh/known_hosts", "/home/me/my files/kh"]
        );
    }

    #[test]
    fn windows_paths_start_at_a_drive_or_a_share_and_program_data_is_filled_in() {
        assert_eq!(
            listed(
                &windows(),
                "C:\\Users\\my name\\.ssh\\kh D:/kh \\\\server\\share\\kh __PROGRAMDATA__\\ssh\\ssh_known_hosts nul"
            ),
            [
                "C:\\Users\\my name\\.ssh\\kh",
                "D:/kh",
                "\\\\server\\share\\kh",
                "C:\\ProgramData\\ssh\\ssh_known_hosts"
            ]
        );
    }

    #[test]
    fn a_program_data_path_with_no_program_data_is_refused_rather_than_joined() {
        let rules = Rules {
            program_data: None,
            ..windows()
        };

        assert!(files(&words("C:\\missing\\kh __PROGRAMDATA__\\ssh\\extra_kh"), &rules).is_err());
    }

    #[test]
    fn a_unix_style_path_on_windows_is_a_ssh_that_is_not_windows_openssh() {
        let error = files(&words("/c/Users/me/.ssh/known_hosts"), &windows()).expect_err("refused");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            error.to_string(),
            "the ssh found on PATH is not Windows OpenSSH; install the OpenSSH client feature"
        );
        // And a Windows path where Unix is expected is not one either.
        assert!(files(&words("C:\\kh"), &unix()).is_err());
    }

    #[test]
    fn more_files_than_the_limit_are_refused_not_cut() {
        let many: Vec<String> = (0..=MAX_FILES).map(|n| format!("/kh{n}")).collect();

        assert_eq!(
            files(&many, &unix()).expect_err("too many").to_string(),
            "ssh_config names too many known_hosts files; connect with ssh"
        );
        assert_eq!(
            files(&many[..MAX_FILES], &unix()).expect("at the limit").paths.len(),
            MAX_FILES
        );
    }

    #[cfg(unix)]
    #[test]
    fn words_that_could_be_the_paths_of_two_files_are_refused_and_a_folder_with_a_space_is_not() {
        let dir = tempfile::tempdir().expect("temp dir");
        let base = dir.path().to_string_lossy().into_owned();

        std::fs::write(dir.path().join("kh"), "").expect("a file");
        std::fs::create_dir(dir.path().join("folder")).expect("a folder");

        // `<dir>/kh` is a file, `<dir>/folder` a folder and `/dev/null` a device: the word after any of them is another file
        // as likely as the rest of a name. So is a word with a step in it, where the path built so far is nothing.
        for text in [
            format!("{base}/kh rel_kh"),
            format!("{base}/kh x/y"),
            format!("{base}/folder rel"),
            "/dev/null rel".to_owned(),
            format!("{base}/missing ../x"),
            format!("{base}/missing ./x"),
            format!("{base}/missing x/../y"),
            format!("{base}/missing home/.."),
        ] {
            assert_eq!(
                files(&words(&text), &unix()).expect_err(&text).to_string(),
                "ssh_config names a known_hosts file Pairpane cannot place (a relative path or a path with spaces next to \
                 another file); connect with ssh",
                "{text}"
            );
        }

        // An absolute word is a new path, whatever comes before it.
        assert_eq!(
            listed(&unix(), &format!("{base}/kh {base}/b c")),
            [format!("{base}/kh"), format!("{base}/b c")]
        );

        // `<dir>/my` is nothing, so `<dir>/my home/kh` is one path: a joined one.
        let joined = files(&words(&format!("{base}/my home/kh")), &unix()).expect("one path");

        assert_eq!(joined.paths, [PathBuf::from(format!("{base}/my home/kh"))]);
        assert_eq!(joined.joined, joined.paths);
        assert!(
            files(&words(&format!("{base}/kh")), &unix())
                .expect("one")
                .joined
                .is_empty()
        );
    }
}
