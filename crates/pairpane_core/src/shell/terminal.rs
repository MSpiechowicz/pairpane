//! "Open terminal here": the terminal a folder is opened in, by system.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use super::Shell;

/// The program a terminal is opened with in a folder, with its arguments (the folder is also its working directory).
/// `None` when none is found.
pub fn terminal(folder: &Path) -> Option<(OsString, Vec<OsString>)> {
    terminal_from(folder, std::env::var_os("TERMINAL"), &on_path, &in_system)
}

/// The system's own folders of programs, where a known terminal is looked for (not the user's, where a file of the same
/// name could have been put).
#[cfg(all(unix, not(target_os = "macos")))]
const SYSTEM_FOLDERS: &[&str] = &["/usr/bin", "/bin", "/usr/local/bin", "/run/current-system/sw/bin"];

/// The terminals tried in order on Linux and the BSDs, when `$TERMINAL` names none.
#[cfg(all(unix, not(target_os = "macos")))]
const TERMINALS: &[&str] = &[
    "x-terminal-emulator",
    "gnome-terminal",
    "konsole",
    "xfce4-terminal",
    "kitty",
    "alacritty",
    "wezterm",
    "foot",
    "xterm",
];

/// [`terminal`], from what `$TERMINAL` holds, a lookup of a name on the `PATH` (for the one `$TERMINAL` names, the
/// user's own choice) and one in the system's folders (for the known ones), so that it can be tested.
#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn terminal_from(
    _folder: &Path,
    variable: Option<OsString>,
    find: &dyn Fn(&str) -> Option<PathBuf>,
    find_known: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<(OsString, Vec<OsString>)> {
    // `$TERMINAL` is the user's own choice: a name on the PATH or an absolute path, with no arguments.
    let chosen = variable.and_then(|value| {
        let value = value.into_string().ok()?;
        let path = PathBuf::from(&value);
        if path.is_absolute() {
            return path.is_file().then_some(path);
        }
        (!value.is_empty() && !value.contains('/'))
            .then(|| find(&value))
            .flatten()
    });

    chosen
        .or_else(|| TERMINALS.iter().find_map(|name| find_known(name)))
        .map(|program| (program.into_os_string(), Vec::new()))
}

/// [`terminal`] on macOS and Windows, where it is fixed: on macOS Terminal, by its bundle id (not by a name another app
/// could have), opened on the folder; on Windows the shell in a console of its own (its working directory is the folder).
#[cfg(not(all(unix, not(target_os = "macos"))))]
pub(super) fn terminal_from(
    folder: &Path,
    _variable: Option<OsString>,
    _find: &dyn Fn(&str) -> Option<PathBuf>,
    _find_known: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<(OsString, Vec<OsString>)> {
    #[cfg(target_os = "macos")]
    {
        Some((
            OsString::from("/usr/bin/open"),
            vec![
                OsString::from("-b"),
                OsString::from("com.apple.Terminal"),
                folder.as_os_str().to_owned(),
            ],
        ))
    }

    #[cfg(windows)]
    {
        let _ = folder;
        Some((Shell::of_user().program.into_os_string(), vec![OsString::from("/K")]))
    }
}

/// The file `name` stands for on the `PATH`: the first folder that has it as a file.
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;

    std::env::split_paths(&path)
        .filter(|folder| folder.is_absolute())
        .map(|folder| folder.join(name))
        .find(|candidate| candidate.is_file())
}

/// The file `name` is in the system's own folders of programs. Elsewhere than Linux and the BSDs there are none to look in.
fn in_system(name: &str) -> Option<PathBuf> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        SYSTEM_FOLDERS
            .iter()
            .map(|folder| Path::new(folder).join(name))
            .find(|candidate| candidate.is_file())
    }

    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let _ = name;
        None
    }
}
