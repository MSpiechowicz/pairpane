//! The environment of what the command line and "Open terminal here" start.

use std::ffi::{OsStr, OsString};
use std::process::Command;

/// The changes to the environment a program started from an AppImage needs: the variables the AppImage runtime set
/// (`APPDIR`, `APPIMAGE`, `ARGV0`, `OWD`) removed, and the folders inside the AppImage's mount taken out of every
/// variable that names them (a list such as `PATH` keeps its other entries; a variable left empty, or one that is not a
/// list, is removed). The mount goes away with the app, and its path in the shared `/tmp` could then be made again by
/// someone else. Nothing changes outside an AppImage. `None` as a value: remove the variable.
pub fn appimage_env(vars: impl Iterator<Item = (OsString, OsString)>) -> Vec<(OsString, Option<OsString>)> {
    let vars: Vec<(OsString, OsString)> = vars.collect();
    let Some(appdir) = vars
        .iter()
        .find(|(name, _)| name == "APPDIR")
        .and_then(|(_, value)| value.to_str())
        .filter(|value| value.len() > 1)
        .map(|value| value.trim_end_matches('/').to_owned())
    else {
        return Vec::new();
    };
    let inside = |entry: &str| entry == appdir || entry.starts_with(&format!("{appdir}/"));

    vars.into_iter()
        .filter_map(|(name, value)| {
            if ["APPDIR", "APPIMAGE", "ARGV0", "OWD"].iter().any(|own| name == *own) {
                return Some((name, None));
            }
            let text = value.to_str()?;
            if !text.contains(appdir.as_str()) {
                return None;
            }

            let kept: Vec<&str> = text
                .split(':')
                .filter(|entry| !entry.is_empty() && !inside(entry))
                .collect();
            let list = text.contains(':');
            Some(if list && !kept.is_empty() {
                (name, Some(OsString::from(kept.join(":"))))
            } else {
                (name, None)
            })
        })
        .collect()
}

/// Applies [`appimage_env`] of the app's own environment to `command`, for every variable it changes.
pub fn apply_appimage_env(command: &mut Command) {
    for (name, value) in appimage_env(std::env::vars_os()) {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
}

/// `vars` as a child should see them from an AppImage: [`appimage_env`] applied, so the variables of the runtime are gone
/// and the folders of its mount are out of the others.
pub fn cleaned_env(vars: impl Iterator<Item = (OsString, OsString)>) -> Vec<(OsString, OsString)> {
    let vars: Vec<(OsString, OsString)> = vars.collect();
    let changes = appimage_env(vars.clone().into_iter());

    vars.into_iter()
        .filter_map(
            |(name, value)| match changes.iter().find(|(changed, _)| *changed == name) {
                Some((_, replacement)) => replacement.clone().map(|replacement| (name, replacement)),
                None => Some((name, value)),
            },
        )
        .collect()
}

/// Gives `command` the whole environment of the app, cleaned as [`cleaned_env`] says, but for the variables `skip` names:
/// for a program that has to see what the user's own `ssh` would (`ssh -G` runs `Match exec` commands and expands
/// `${VAR}`), with the variables that change how a program is loaded left out. Variables that `command` already has are
/// replaced.
pub fn inherit_cleaned_env(command: &mut Command, skip: impl Fn(&OsStr) -> bool) {
    for (name, value) in cleaned_env(std::env::vars_os()) {
        if !skip(&name) {
            command.env(name, value);
        }
    }
}
