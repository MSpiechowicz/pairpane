//! The confinement of the helper on macOS (see `child` for what the helper does): the steps of the thumbnail helper
//! (`view/thumb/macos.rs`: the inherited descriptors closed, the limits, `sandbox_init`), with a profile that allows what the
//! status reads. The profile is `(deny default)` and then the read of the folders of the layout and nothing else: no write, no
//! network, no program, no process, no signal; a status is pure computation over the files of the repository.
//!
//! The paths are real paths (the parent looked at them), written into the profile as strings with `\` and `"` escaped; a path
//! with a control character in it is no path the profile is built for, and the helper does not go on. To tell where a path is,
//! the system also looks at the folders above it, so their own entries (not their content) may be looked at: `file-read-metadata`
//! of each of them (`literal`, not `subpath`).
//!
//! Not tried here: the profile is built by a function that the tests of every system run ([`profile`]), but entering it is
//! `sandbox_init`, which only macOS has, and a build for macOS is what shows whether gix needs more than this (the CI step "Git
//! status helper, real executable" runs it there).

use std::ffi::CString;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use std::io;

#[cfg(target_os = "macos")]
use super::child::MEMORY;
#[cfg(target_os = "macos")]
use super::layout::Grant;
#[cfg(target_os = "macos")]
use super::request::Request;

/// The profile that lets a process read the files below `folders` and nothing else, or `None` when a path cannot be written
/// into it (it holds a control character, or is not text).
pub(super) fn profile(folders: &[PathBuf]) -> Option<CString> {
    let mut readable = String::new();
    let mut above: Vec<PathBuf> = Vec::new();

    for folder in folders {
        readable.push_str(&format!(" (subpath {})", quoted(folder)?));

        for ancestor in folder.ancestors().skip(1) {
            if !above.iter().any(|known| known == ancestor) {
                above.push(ancestor.to_path_buf());
            }
        }
    }
    let mut looked_at = String::new();
    for ancestor in &above {
        looked_at.push_str(&format!(" (literal {})", quoted(ancestor)?));
    }

    let text = format!("(version 1)(deny default)(allow file-read*{readable})(allow file-read-metadata{looked_at})");

    CString::new(text).ok()
}

/// `path` as a string of the profile.
fn quoted(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    if text.chars().any(char::is_control) {
        return None;
    }

    Some(format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"")))
}

/// The steps (see the module), in order, before gix opens anything; the descriptors are closed by the caller.
#[cfg(target_os = "macos")]
pub(super) fn confine(request: &Request, cpu_seconds: u64) -> io::Result<()> {
    use crate::view::thumb::macos;

    macos::limit_to(cpu_seconds, super::child::MAX_FILES, MEMORY)?;

    // Each folder must still be the one that was looked at: the profile names paths, and a path that now leads elsewhere would
    // allow the read of something else. (The profile matches paths when a file is opened, so a swap after this check is the
    // residual that Landlock's descriptors do not have.)
    if !request.grants.iter().all(Grant::is_unchanged) {
        return Err(io::Error::other(
            "a folder of the repository is not the one that was looked at",
        ));
    }
    let folders: Vec<PathBuf> = request.grants.iter().map(|grant| grant.path.clone()).collect();
    let profile = profile(&folders)
        .ok_or_else(|| io::Error::other("a folder of the repository cannot be written into the profile"))?;

    macos::enter(&profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_denies_everything_and_reads_the_folders_and_looks_at_the_ones_above_them() {
        let folders = [PathBuf::from("/Users/a/repo"), PathBuf::from("/Users/a/repo/.git")];

        let text = profile(&folders).expect("a profile").into_string().expect("text");

        assert_eq!(
            text,
            "(version 1)(deny default)(allow file-read* (subpath \"/Users/a/repo\") (subpath \"/Users/a/repo/.git\"))\
             (allow file-read-metadata (literal \"/Users/a\") (literal \"/Users\") (literal \"/\") (literal \"/Users/a/repo\"))"
        );
        assert!(!text.contains("file-write") && !text.contains("network") && !text.contains("process"));
    }

    #[test]
    fn a_quote_or_a_backslash_in_a_name_is_escaped_and_a_control_character_is_refused() {
        let text = profile(&[PathBuf::from(r#"/work/my "repo"\x"#)])
            .expect("a profile")
            .into_string()
            .expect("text");

        assert!(text.contains(r#"(subpath "/work/my \"repo\"\\x")"#), "{text}");
        assert!(profile(&[PathBuf::from("/work/new\nline")]).is_none());
    }
}
