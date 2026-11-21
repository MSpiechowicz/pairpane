//! The settings file the user edits (`settings.json` in the app's config folder): for now the external editor F4
//! starts.
//!
//! ```json
//! { "version": 1, "editor": { "program": "code", "args": ["--wait", "{file}"] } }
//! ```
//!
//! `program` is a bare name (looked up on the `PATH`) or an absolute path; a relative path with a separator is refused.
//! In `args` an element that is exactly `{file}` is replaced by the absolute path of the file, as one argument (no
//! shell is ever involved, so nothing in a name is read as a command); with no such element the path is added last. An
//! empty `program`, or no `editor`, means the built-in editor. The file is the user's: it is read fresh each time (so
//! an edit applies at once), only up to [`MAX_SETTINGS`], and one that is not valid is an error, never a silent
//! fallback. The page never writes it; [`ensure`] only creates it with the defaults when it is missing, to be opened in
//! the system's own program.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Read, Write};
use std::path::Path;

use serde::Deserialize;

use crate::view::refusal;

/// The name of the file in the config folder.
pub const SETTINGS_FILE: &str = "settings.json";

/// The biggest settings file read, in bytes.
pub const MAX_SETTINGS: u64 = 64 * 1024;

/// The element of `args` that stands for the file.
const FILE_ARG: &str = "{file}";

/// What [`ensure`] writes: no editor yet, and the way to give one.
const DEFAULT_FILE: &str =
    "{\n  \"version\": 1,\n  \"editor\": {\n    \"program\": \"\",\n    \"args\": [\"{file}\"]\n  }\n}\n";

/// The settings: what the file holds, or the defaults (no external editor) without a file.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    version: Option<u32>,
    editor: Option<ExternalEditor>,
    /// A note the user keeps in the file: the one key besides the settings that is allowed, and it is not used.
    #[serde(rename = "$comment")]
    _comment: Option<serde_json::Value>,
}

/// The external editor F4 starts.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalEditor {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl Settings {
    /// The external editor, or `None` when there is none (no `editor`, or an empty `program`): the built-in editor
    /// then opens.
    pub fn editor(&self) -> Option<&ExternalEditor> {
        self.editor.as_ref().filter(|editor| !editor.program.trim().is_empty())
    }
}

fn invalid() -> io::Error {
    refusal(io::ErrorKind::InvalidInput, "the settings file is not valid")
}

/// A settings file that is there and cannot be read (no permission and so on); the system's wording is for the log.
fn unreadable(error: &io::Error) -> io::Error {
    tracing::debug!(%error, "the settings file cannot be read");

    refusal(io::ErrorKind::InvalidData, "the settings file cannot be read")
}

/// Opens the settings file for reading, following a link (a settings file kept in a dotfiles folder). A pipe that is
/// there instead is never waited on (the open does not block) and a terminal is never taken; what it is is checked on
/// the handle.
#[cfg(unix)]
fn open_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    let flags = (rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::NOCTTY).bits();

    OpenOptions::new().read(true).custom_flags(flags as i32).open(path)
}

#[cfg(not(unix))]
fn open_file(path: &Path) -> io::Result<File> {
    File::open(path)
}

/// Whether `program` is a bare name (to be looked up on the `PATH`): one plain name, no folder part. On Windows (`windows`)
/// a `:` is refused too, because `C:evil.exe` is a name in the current folder of the drive `C`, and a name with a stream
/// (`a.exe:b`) is not a program name. It is a function of the text so that it is tested on every system.
fn is_bare_name(program: &str, windows: bool) -> bool {
    let separators: &[char] = if windows { &['/', '\\'] } else { &['/'] };

    !program.is_empty()
        && program != "."
        && program != ".."
        && !program.contains(separators)
        && !(windows && program.contains(':'))
}

/// Whether `program` is a bare name or an absolute path.
fn program_fits(program: &str) -> bool {
    !program.contains('\0') && (is_bare_name(program, cfg!(windows)) || Path::new(program).is_absolute())
}

/// Reads the settings file at `path`; a missing file is the defaults. A file of more than [`MAX_SETTINGS`], not JSON,
/// with an unknown field (but `$comment`) at the top or in `editor`, or of a version other than 1 is refused as not
/// valid (what is wrong goes to the log, never to the caller), and so is an editor program that is neither a name nor an
/// absolute path. A file that cannot be read at all (no permission) is told so.
pub fn load(path: &Path) -> io::Result<Settings> {
    load_with_revision(path).map(|(settings, _)| settings)
}

/// [`load`], and the revision of the file that was read: a number that is the same for the same bytes and (but for a
/// collision) another for others, which only compares within one run. It lets a caller remember something about the file
/// until it changes. A missing file has a revision of its own.
pub fn load_with_revision(path: &Path) -> io::Result<(Settings, u64)> {
    let file = match open_file(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((Settings::default(), 0)),
        Err(error) => return Err(unreadable(&error)),
    };

    // Asked of the handle that is read: what is not a file (a folder, a pipe) is not settings.
    if !file.metadata().map_err(|error| unreadable(&error))?.is_file() {
        return Err(invalid());
    }

    let mut bytes = Vec::new();
    file.take(MAX_SETTINGS + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| unreadable(&error))?;
    if bytes.len() as u64 > MAX_SETTINGS {
        tracing::debug!("the settings file is too big");

        return Err(invalid());
    }

    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    // Never 0, which is the revision of a missing file.
    let revision = hasher.finish() | 1;

    let settings: Settings = serde_json::from_slice(&bytes).map_err(|error| {
        tracing::debug!(%error, "the settings file cannot be read");

        invalid()
    })?;

    if settings.version.is_some_and(|version| version != 1) {
        tracing::debug!("the settings file has another version");

        return Err(invalid());
    }

    if settings.editor().is_some_and(|editor| !program_fits(&editor.program)) {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "the editor program must be a name or an absolute path",
        ));
    }

    Ok((settings, revision))
}

/// Creates the settings file with the defaults when it is missing (and its folder); one that is there is left alone.
pub fn ensure(path: &Path) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }

    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => file.write_all(DEFAULT_FILE.as_bytes()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

/// The program and the arguments to start for `file`, an absolute path: the `args` with `{file}` as the path (the
/// path last when there is none).
pub fn argv(editor: &ExternalEditor, file: &Path) -> (OsString, Vec<OsString>) {
    let mut args: Vec<OsString> = editor
        .args
        .iter()
        .map(|arg| {
            if arg == FILE_ARG {
                file.as_os_str().to_owned()
            } else {
                OsString::from(arg)
            }
        })
        .collect();

    if !editor.args.iter().any(|arg| arg == FILE_ARG) {
        args.push(file.as_os_str().to_owned());
    }

    (OsString::from(editor.program.trim()), args)
}

#[cfg(test)]
mod tests;
