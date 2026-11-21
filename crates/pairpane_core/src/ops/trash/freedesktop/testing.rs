//! What the tests of the freedesktop Trash share: a place to trash from whose home Trash is inside it too, so that
//! nothing of the user's real Trash is touched.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::folders::Context;
use super::info::parse_path;
use super::{Trashed, trash_in};
use crate::ops::items::ItemFailure;

/// The user the tests are.
pub(super) fn me() -> u32 {
    rustix::process::getuid().as_raw()
}

/// Whether the tests run as root, which no permission stops.
pub(super) fn is_root() -> bool {
    me() == 0
}

pub(super) fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
}

/// A folder `work` to trash from, and the data folder `share` of a home Trash, in a temporary folder: all one drive,
/// so the home Trash is the Trash of the items.
pub(super) struct Env {
    pub temp: TempDir,
    pub context: Context,
}

impl Env {
    pub(super) fn new() -> Self {
        let temp = TempDir::new().expect("temp dir");
        let share = temp.path().join("share");
        fs::create_dir(temp.path().join("work")).expect("work");

        Self {
            context: Context {
                uid: me(),
                data_home: Some(share),
            },
            temp,
        }
    }

    pub(super) fn work(&self) -> PathBuf {
        self.temp.path().join("work")
    }

    pub(super) fn trash(&self) -> PathBuf {
        self.temp.path().join("share/Trash")
    }

    pub(super) fn file(&self, name: &str, content: &str) {
        fs::write(self.work().join(name), content).expect("file");
    }

    pub(super) fn read(&self, name: &str) -> String {
        fs::read_to_string(self.work().join(name)).expect("file")
    }

    pub(super) fn trashed(&self, names: &[&str]) -> (Trashed, Result<(), ItemFailure>) {
        let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();

        trash_in(&self.context, &self.work(), &names)
    }

    /// The names in a folder of the Trash (`files` or `info`), sorted.
    pub(super) fn listed(&self, folder: &str) -> Vec<String> {
        let mut found: Vec<String> = fs::read_dir(self.trash().join(folder))
            .expect("Trash folder")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        found.sort();

        found
    }
}

/// Removes from the Trash folder `trash` the items whose record names a path inside `folder`, and their records, and
/// nothing else: it is for a test that moved items to the real Trash from a folder whose name is its own. The paths of
/// the records are read as the code reads them (decoded), and compared by component, so a path that is written with
/// `%XX` is found as well as a plain one.
pub(crate) fn purge_below(trash: &Path, folder: &Path) {
    let Ok(folder) = fs::canonicalize(folder) else { return };
    let Ok(records) = fs::read_dir(trash.join("info")) else {
        return;
    };

    for record in records.flatten() {
        let path = record.path();
        let Some(stem) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".trashinfo"))
        else {
            continue;
        };
        let written = fs::read_to_string(&path).ok().as_deref().and_then(parse_path);

        if !written.is_some_and(|written| written.starts_with(&folder)) {
            continue;
        }

        let item = trash.join("files").join(stem);
        let _ = if fs::symlink_metadata(&item).is_ok_and(|metadata| metadata.is_dir()) {
            fs::remove_dir_all(&item)
        } else {
            fs::remove_file(&item)
        };
        let _ = fs::remove_file(&path);
    }
}

#[cfg(test)]
mod tests;
