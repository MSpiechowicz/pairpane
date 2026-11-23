//! Finding the repository a folder is in, before gix is let near it. gix discovers a repository by walking up and reading
//! what it finds (`HEAD`, `commondir`, `gitdir`, the target of a `.git` file); here the same walk is done with only `lstat`
//! and small, non-blocking reads, so that what a repository names is looked at first: a pointer to a network place and a
//! file that is no regular file make the folder "not in a repository" (see [`super::locality`] and [`super::files`]). The
//! repository found is the one gix would find: the first folder going up that has a `.git` (a folder, or a file that points
//! to one) with a `HEAD`, an `objects` and a `refs` folder, a filesystem boundary (Unix) ends the search, and so does the
//! `ceiling`, the user's home folder, which is the last folder tried.

use std::fs;
use std::path::{Path, PathBuf};

use super::GitError;
use super::files::{self, Seen};
use super::inspect::{self, Refusal};
use super::locality::{is_local, pointer_target};
use crate::listing::platform;

/// The most a `HEAD`, a `commondir`, a `gitdir` or a `.git` file may hold.
pub(super) const POINTER_BYTES: u64 = 4 * 1024;

/// The folders of a repository that was found.
pub(super) struct Layout {
    /// The folder with `HEAD`, the `index` and the files of a work tree of its own.
    pub(super) git_dir: PathBuf,
    /// The folder with `config`, the objects and the refs: the same as `git_dir` unless this is a linked work tree.
    pub(super) common_dir: PathBuf,
}

/// What a folder going up the search is.
enum Candidate {
    /// Not a repository; the search goes on.
    NotHere,
    /// Something that must not be read (not local, not a regular file): the folder is in no repository, and the search ends.
    Refused,
    /// A repository's own folder.
    Repository(Layout),
}

/// The `.git` (a folder, or a file that points to one) of the repository that `real` (a real path) is in, or `None` when
/// there is none, or one that must not be read. `ceiling` is a real path too. Only a repository whose files all pass
/// [`inspect`] is found.
pub(super) fn find(real: &Path, ceiling: Option<&Path>) -> Result<Option<PathBuf>, GitError> {
    let mut dir = real.to_path_buf();
    let device = device_of(&dir);

    loop {
        // A filesystem boundary ends the search: a mount point's parent is not the same place.
        if device_of(&dir) != device {
            return Ok(None);
        }

        match candidate(&dir) {
            Candidate::NotHere => {}
            Candidate::Refused => return Ok(None),
            Candidate::Repository(layout) => return found(dir, &layout),
        }

        if ceiling == Some(dir.as_path()) || !dir.pop() {
            return Ok(None);
        }
    }
}

fn found(dir: PathBuf, layout: &Layout) -> Result<Option<PathBuf>, GitError> {
    match inspect::files(layout) {
        Ok(()) => Ok(Some(dir.join(".git"))),
        Err(Refusal::Odd) => Ok(None),
        Err(Refusal::TooBig) => Err(GitError::TooBig),
    }
}

fn device_of(dir: &Path) -> Option<u64> {
    fs::metadata(dir).ok().and_then(|metadata| platform::device(&metadata))
}

/// What `dir` holds: its `.git`, or else itself as a repository's folder (a `dir` that is a git folder has no work tree,
/// and is read as no repository by the caller: it is found here so that the search stops where gix's would).
fn candidate(dir: &Path) -> Candidate {
    let dot_git = dir.join(".git");

    match dot_git_folder(dir, &dot_git) {
        Candidate::NotHere => {}
        found => return found,
    }

    match git_dir(dir) {
        // A git folder itself is no work tree: nothing is read of it.
        Candidate::Repository(_) => Candidate::Refused,
        other => other,
    }
}

/// The repository the `.git` of `dir` is: a folder, a file that points to a folder, or a link to either.
fn dot_git_folder(dir: &Path, dot_git: &Path) -> Candidate {
    let metadata = match fs::symlink_metadata(dot_git) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Candidate::NotHere,
        Err(_) => return Candidate::Refused,
    };

    let metadata = if metadata.file_type().is_symlink() {
        // What a link leads to is followed only to a local place; one that leads nowhere is no repository.
        if !is_local(dot_git) {
            return Candidate::Refused;
        }
        match fs::metadata(dot_git) {
            Ok(metadata) => metadata,
            Err(_) => return Candidate::NotHere,
        }
    } else {
        metadata
    };

    if metadata.is_dir() {
        git_dir(dot_git)
    } else if metadata.is_file() {
        pointed_to(dir, dot_git)
    } else {
        Candidate::Refused
    }
}

/// The repository a `.git` file points to with its `gitdir: <path>` line.
fn pointed_to(dir: &Path, dot_git: &Path) -> Candidate {
    let bytes = match files::read(dot_git, POINTER_BYTES) {
        Seen::Plain(bytes) => bytes,
        Seen::Missing => return Candidate::NotHere,
        Seen::Odd | Seen::Large => return Candidate::Refused,
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return Candidate::NotHere;
    };
    let Some(path) = text.lines().next().and_then(|line| line.strip_prefix("gitdir:")) else {
        return Candidate::NotHere;
    };

    // Not a local place: the folder is in no repository, and nothing at the other end is touched.
    let Some(target) = pointer_target(dir, path.trim()) else {
        return Candidate::Refused;
    };

    match fs::metadata(&target) {
        Ok(metadata) if metadata.is_dir() => git_dir(&target),
        _ => Candidate::NotHere,
    }
}

/// Whether `git_dir` is a repository's folder, as gix sees one (`HEAD`, and `objects` and `refs` in the common folder), and
/// where its common folder is.
fn git_dir(git_dir: &Path) -> Candidate {
    if !is_local(git_dir) {
        return Candidate::Refused;
    }

    match files::seen(&git_dir.join("HEAD"), POINTER_BYTES) {
        Seen::Missing => return Candidate::NotHere,
        Seen::Plain(()) => {}
        Seen::Odd | Seen::Large => return Candidate::Refused,
    }

    // `commondir` makes this a linked work tree's own folder, and `gitdir` names the work tree it belongs to; gix reads both.
    if !files::seen(&git_dir.join("gitdir"), POINTER_BYTES).is_fine() {
        return Candidate::Refused;
    }
    let common_dir = match files::read(&git_dir.join("commondir"), POINTER_BYTES) {
        Seen::Missing => git_dir.to_path_buf(),
        Seen::Plain(bytes) => match common_dir(git_dir, &bytes) {
            Some(common_dir) => common_dir,
            None => return Candidate::Refused,
        },
        Seen::Odd | Seen::Large => return Candidate::Refused,
    };

    // The folders gix asks for next, through a link or not, have to be local ones first.
    let objects = common_dir.join("objects");
    if !is_local(&common_dir) || !is_local(&objects) {
        return Candidate::Refused;
    }
    if !objects.is_dir() || !common_dir.join("refs").is_dir() {
        return Candidate::NotHere;
    }

    Candidate::Repository(Layout {
        git_dir: git_dir.to_path_buf(),
        common_dir,
    })
}

fn common_dir(git_dir: &Path, bytes: &[u8]) -> Option<PathBuf> {
    let text = std::str::from_utf8(bytes).ok()?;

    pointer_target(git_dir, text.trim())
}
