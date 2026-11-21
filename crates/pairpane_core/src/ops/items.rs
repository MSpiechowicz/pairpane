//! Single-item changes of a folder that are not copy jobs: new folders and files, links, and permanent delete.
//!
//! Every function here blocks on the disk. The caller has checked the folder with `check_folder` and resolved it
//! with `local_path`; the names are checked again here wherever a wrong one would do harm.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use crate::listing::platform;
use crate::model::LinkKind;

use super::report::refusal;
use super::{delete, open, rename};

/// Most parts a new-folder path may have (`a/b/c` has 3).
pub const MAX_FOLDER_DEPTH: usize = 64;

/// An item that a multi-item change stopped at, and why. The items before it were done; the ones after it were not
/// touched.
#[derive(Debug)]
pub struct ItemFailure {
    /// The plain name of the item.
    pub name: String,
    pub error: io::Error,
}

/// Splits the path of a new folder into its parts (see [`super::folder_steps`]).
pub(super) fn folder_steps(relative: &str) -> io::Result<Vec<&str>> {
    let steps: Vec<&str> = relative.split(is_separator).collect();

    if steps.len() > MAX_FOLDER_DEPTH {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "the folder path has too many levels",
        ));
    }

    for step in &steps {
        if step.is_empty() {
            return Err(refusal(
                io::ErrorKind::InvalidInput,
                "the folder path must not start or end with a separator or hold an empty part",
            ));
        }

        if !platform::is_plain_name(step) {
            return Err(refusal(
                io::ErrorKind::InvalidInput,
                "the folder path holds a name that is not valid",
            ));
        }

        if !super::name_fits(step) {
            return Err(refusal(
                io::ErrorKind::InvalidInput,
                "a folder name in the path is too long",
            ));
        }
    }

    Ok(steps)
}

/// `/` everywhere, and `\` on Windows.
fn is_separator(char: char) -> bool {
    char == '/' || (cfg!(windows) && char == '\\')
}

/// Creates the folders of `relative` inside `folder` (see [`super::create_folders`]).
pub(super) fn create_folders(folder: &Path, relative: &str) -> io::Result<String> {
    let steps = folder_steps(relative)?;
    let mut path = folder.to_path_buf();

    for (index, step) in steps.iter().enumerate() {
        path.push(step);

        match open::create_folder(&path, false) {
            Ok(()) => {}
            // A folder that is there on the way is used; the last one must be new.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && index + 1 < steps.len() => {
                if !is_real_folder(&path) {
                    return Err(refusal(
                        io::ErrorKind::InvalidInput,
                        "a name on the path is a file or a link, not a folder",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
    }

    Ok(steps[0].to_owned())
}

/// Whether `path` is a folder itself, not a link to one.
fn is_real_folder(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

/// Creates the empty file `name` in `folder` (see [`super::create_file`]).
pub(super) fn create_file(folder: &Path, name: &str) -> io::Result<PathBuf> {
    if !platform::is_plain_name(name) {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "the name is not a plain file name",
        ));
    }

    let path = folder.join(name);

    // Not `open::create_new`: that file is private to its owner until a copy sets its real permissions, and this
    // one is final. `create_new` also fails on a link at the name, even a dangling one, and never follows it.
    OpenOptions::new().write(true).create_new(true).open(&path)?;

    Ok(path)
}

/// Makes the link `name` in `folder` to `target` (see [`super::make_link`]).
pub(super) fn make_link(kind: LinkKind, target: &Path, folder: &Path, name: &str) -> io::Result<()> {
    if !platform::is_plain_name(name) {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "the link name is not a plain file name",
        ));
    }

    if !target.is_absolute() {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "the link target must be an absolute path",
        ));
    }

    let link = folder.join(name);

    match kind {
        LinkKind::Symbolic => symbolic(target, &link),
        LinkKind::Hard => hard(target, &link),
    }
}

#[cfg(unix)]
fn symbolic(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// A Windows link is made for a folder or for a file, so the kind of the target decides. A target that cannot be
/// followed (it is gone, or `follow` refuses its path) gets a file link.
#[cfg(windows)]
fn symbolic(target: &Path, link: &Path) -> io::Result<()> {
    use std::os::windows::fs::{symlink_dir, symlink_file};

    use super::report::WINDOWS_NO_LINK_PRIVILEGE;

    let made = if platform::follow(target).is_some_and(|metadata| metadata.is_dir()) {
        symlink_dir(target, link)
    } else {
        symlink_file(target, link)
    };

    made.map_err(|error| {
        if error.raw_os_error() == Some(WINDOWS_NO_LINK_PRIVILEGE) {
            return refusal(
                io::ErrorKind::Other,
                "Windows allows symbolic links only with Developer Mode on or as administrator",
            );
        }

        error
    })
}

/// A hard link to the file `target` itself: never to a folder (the system refuses it) or to a link (whether the
/// new name would be one more link or a name of the file behind it differs between systems).
fn hard(target: &Path, link: &Path) -> io::Result<()> {
    if !fs::symlink_metadata(target)?.is_file() {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "a hard link can be made only to a file",
        ));
    }

    fs::hard_link(target, link).map_err(|error| {
        if rename::crosses_devices(&error) {
            return refusal(
                io::ErrorKind::CrossesDevices,
                "a hard link cannot be made to a file on another disk",
            );
        }

        error
    })
}

/// Deletes the items `names` of `folder` for good (see [`super::delete_permanently`]).
pub(super) fn delete_permanently(folder: &Path, names: &[String]) -> Result<(), ItemFailure> {
    for name in names {
        let failure = |error| ItemFailure {
            name: name.clone(),
            error,
        };

        if !platform::is_plain_name(name) {
            return Err(failure(refusal(
                io::ErrorKind::InvalidInput,
                "the name is not a plain file name",
            )));
        }

        delete::remove(&folder.join(name)).map_err(failure)?;
    }

    Ok(())
}
