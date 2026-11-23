//! Finding the repository a folder is in, before gix is let near it. gix discovers a repository by walking up and reading
//! what it finds (`HEAD`, `commondir`, `gitdir`, the target of a `.git` file); here the same walk is done with only `lstat`
//! and small, non-blocking reads, so that what a repository names is looked at first: a pointer to a network place and a
//! file that is no regular file make the folder "not in a repository" (see [`super::locality`] and [`super::files`]). The
//! repository found is the one gix would find: the first folder going up that has a `.git` (a folder, or a file that points
//! to one) with a valid `HEAD`, an `objects` and a `refs` folder, a filesystem boundary (Unix) ends the search, and so does
//! the `ceiling`, the user's home folder, which is the last folder tried.
//!
//! What is read is read the way gix reads it, so that what is looked at is what gix opens: a `.git` file is parsed with gix's
//! own `gitdir:` parser (`gix::discover::parse::gitdir`: one space after the colon, and the text up to the end of the file
//! without its trailing white space), a `commondir` is the file without its trailing white space, and a `.git` folder that
//! has both a `commondir` and a `gitdir` is the folder of a linked work tree, whose work tree gix takes from the content of the
//! `gitdir` file (so that is refused unless it names this very `.git`). The [`Layout`] that this finds is what the repository
//! must be once gix has opened it (see [`Layout::is_opened`]).

use std::fs;
use std::path::{Path, PathBuf};

use gix::bstr::ByteSlice;

use super::GitError;
use super::files::{self, Seen};
use super::inspect::{self, Refusal};
use super::links::{self, is_local, normalise};
use super::locality::pointer_target;
use crate::listing::platform;

/// The most a `HEAD`, a `commondir`, a `gitdir` or a `.git` file may hold.
pub(super) const POINTER_BYTES: u64 = 4 * 1024;

/// The folders of a repository that was found, as gix is to open them.
pub(super) struct Layout {
    /// The `.git` that is handed to gix: a folder, a file that points to one, or a link to a folder.
    pub(super) dot_git: PathBuf,
    /// The folder with `HEAD`, the `index` and the files of a work tree of its own.
    pub(super) git_dir: PathBuf,
    /// The folder with `config`, the objects and the refs: the same as `git_dir` unless this is a linked work tree.
    pub(super) common_dir: PathBuf,
    /// The work tree: the folder that holds the `.git`.
    pub(super) workdir: PathBuf,
}

impl Layout {
    /// Whether the repository gix opened is the one that was looked at: the same git folder, common folder and work tree, as
    /// paths are compared (their `.` and `..` read away). When it is not, gix took a path from somewhere that this did not
    /// look at, and the repository must not be read.
    pub(super) fn is_opened(&self, repository: &gix::Repository) -> bool {
        same(repository.git_dir(), &self.git_dir)
            && same(repository.common_dir(), &self.common_dir)
            && repository.workdir().is_some_and(|workdir| same(workdir, &self.workdir))
    }
}

/// Whether two paths are the same one once their dots are read away (and, on macOS, once they are precomposed, which gix does
/// to the paths of a repository that asks for it).
fn same(opened: &Path, looked_at: &Path) -> bool {
    let (opened, looked_at) = (normalise(opened), normalise(looked_at));

    if opened == looked_at {
        return true;
    }

    cfg!(target_os = "macos")
        && gix::utils::str::precompose_path(opened.as_path().into())
            == gix::utils::str::precompose_path(looked_at.as_path().into())
}

/// Why a folder going up the search gives no repository.
enum Skip {
    /// Not a repository; the search goes on.
    NotHere,
    /// Something that must not be read (not local, not a regular file): the folder is in no repository, and the search ends.
    Refused,
}

/// The repository that `real` (a real path) is in, or `None` when there is none, or one that must not be read. `ceiling` is a
/// real path too. Only a repository whose files all pass [`inspect`] is found.
pub(super) fn find(real: &Path, ceiling: Option<&Path>) -> Result<Option<Layout>, GitError> {
    let mut dir = real.to_path_buf();
    let device = device_of(&dir);

    loop {
        // A filesystem boundary ends the search: a mount point's parent is not the same place.
        if device_of(&dir) != device {
            return Ok(None);
        }

        match candidate(&dir) {
            Ok(layout) => return found(layout),
            Err(Skip::Refused) => return Ok(None),
            Err(Skip::NotHere) => {}
        }

        if ceiling == Some(dir.as_path()) || !dir.pop() {
            return Ok(None);
        }
    }
}

fn found(layout: Layout) -> Result<Option<Layout>, GitError> {
    match inspect::files(&layout) {
        Ok(()) => Ok(Some(layout)),
        Err(Refusal::Odd) => Ok(None),
        Err(Refusal::TooBig) => Err(GitError::TooBig),
    }
}

fn device_of(dir: &Path) -> Option<u64> {
    fs::metadata(dir).ok().and_then(|metadata| platform::device(&metadata))
}

/// What `dir` holds: its `.git`, or else itself as a repository's folder (a `dir` that is a git folder has no work tree,
/// and is read as no repository by the caller: it is found here so that the search stops where gix's would).
fn candidate(dir: &Path) -> Result<Layout, Skip> {
    match dot_git_folder(dir) {
        Err(Skip::NotHere) => {}
        other => return other,
    }

    // A git folder itself is no work tree: nothing is read of it.
    match git_dir(dir) {
        Ok(_) => Err(Skip::Refused),
        Err(skip) => Err(skip),
    }
}

/// The repository the `.git` of `dir` is: a folder, a file that points to a folder, or a link to either.
fn dot_git_folder(dir: &Path) -> Result<Layout, Skip> {
    let dot_git = dir.join(".git");
    let metadata = match fs::symlink_metadata(&dot_git) {
        Ok(metadata) => metadata,
        Err(error) if links::leads_nowhere(&error) => return Err(Skip::NotHere),
        Err(_) => return Err(Skip::Refused),
    };

    // A link is followed hop by hop to a local place, and what is at the end of it is looked at, not what the link is.
    let (metadata, at) = if metadata.file_type().is_symlink() {
        let resolved = match links::resolve(&dot_git) {
            Ok(resolved) => resolved,
            Err(error) if links::leads_nowhere(&error) => return Err(Skip::NotHere),
            Err(_) => return Err(Skip::Refused),
        };
        let metadata = fs::symlink_metadata(&resolved).map_err(|_| Skip::NotHere)?;

        (metadata, resolved)
    } else {
        (metadata, dot_git.clone())
    };

    if metadata.is_dir() {
        own_folder(dir, dot_git)
    } else if metadata.is_file() {
        pointed_to(dir, dot_git, &at)
    } else {
        Err(Skip::Refused)
    }
}

/// The repository whose own folder is `dot_git` (a `.git` folder).
fn own_folder(dir: &Path, dot_git: PathBuf) -> Result<Layout, Skip> {
    let folders = git_dir(&dot_git)?;
    names_this_folder(&dot_git, &folders)?;

    Ok(Layout {
        git_dir: dot_git.clone(),
        common_dir: folders.common_dir,
        workdir: dir.to_path_buf(),
        dot_git,
    })
}

/// A `.git` folder that has a `commondir` and a `gitdir` is a linked work tree's own folder, and gix takes the work tree from
/// the content of its `gitdir` file (the folder of the work tree, or its `.git` file). That has to be this folder's own place:
/// the work tree whose files are looked at is the one that holds this `.git`, and not another one that a hostile `gitdir` names,
/// which would carry the ignore rules and the files of that other place into the status of this one.
fn names_this_folder(dot_git: &Path, folders: &Folders) -> Result<(), Skip> {
    if !folders.has_commondir {
        return Ok(());
    }

    let bytes = match files::read(&dot_git.join("gitdir"), POINTER_BYTES) {
        Seen::Plain(bytes) => bytes,
        Seen::Missing => return Ok(()),
        Seen::Odd | Seen::Large => return Err(Skip::Refused),
    };
    let Some(named) = plain_path(&bytes) else {
        // gix reads an empty or unreadable `gitdir` as none, and so takes no work tree from it.
        return Ok(());
    };

    // gix resolves a relative path from the folder of the file.
    if normalise(&dot_git.join(named)) == normalise(dot_git) {
        Ok(())
    } else {
        Err(Skip::Refused)
    }
}

/// The path a file that holds nothing else says, with the white space at its end cut off as gix does and nothing else: `None`
/// for an empty one, and for text that is no path.
fn plain_path(bytes: &[u8]) -> Option<PathBuf> {
    let text = bytes.trim_end();
    if text.is_empty() {
        return None;
    }

    gix::path::from_bstring(text.to_vec()).ok()
}

/// The repository a `.git` file points to with its `gitdir: <path>` line. `dot_git` is where the file is, and `at` where it is
/// read from (the same, unless `.git` is a link to it).
fn pointed_to(dir: &Path, dot_git: PathBuf, at: &Path) -> Result<Layout, Skip> {
    let bytes = match files::read(at, POINTER_BYTES) {
        Seen::Plain(bytes) => bytes,
        Seen::Missing => return Err(Skip::NotHere),
        Seen::Odd | Seen::Large => return Err(Skip::Refused),
    };
    // gix's own parser, so that the path is the one gix reads: it has no trimming at the start and it takes the whole file.
    let Ok(pointer) = gix::discover::parse::gitdir(&bytes) else {
        return Err(Skip::NotHere);
    };

    // Not a local place: the folder is in no repository, and nothing at the other end is touched.
    let target = pointer_target(dir, &pointer).ok_or(Skip::Refused)?;
    let resolved = match links::resolve(&target) {
        Ok(resolved) => resolved,
        Err(error) if links::leads_nowhere(&error) => return Err(Skip::NotHere),
        Err(_) => return Err(Skip::Refused),
    };
    match fs::symlink_metadata(&resolved) {
        Ok(metadata) if metadata.is_dir() => {}
        _ => return Err(Skip::NotHere),
    }

    let folders = git_dir(&target)?;

    Ok(Layout {
        git_dir: target,
        common_dir: folders.common_dir,
        workdir: dir.to_path_buf(),
        dot_git,
    })
}

/// What a git folder holds that was looked at.
struct Folders {
    /// The folder with `config`, the objects and the refs.
    common_dir: PathBuf,
    /// There is a `commondir` file.
    has_commondir: bool,
}

/// Whether `git_dir` is a repository's folder, as gix sees one (a `HEAD` that holds a ref or an id, and `objects` and `refs` in
/// the common folder), and where its common folder is.
fn git_dir(git_dir: &Path) -> Result<Folders, Skip> {
    if !is_local(git_dir) {
        return Err(Skip::Refused);
    }

    // gix reads `HEAD` through a link (git's old way of writing a symbolic ref), so it is read as gix reads it.
    match files::read_following(&git_dir.join("HEAD"), POINTER_BYTES) {
        Seen::Missing => return Err(Skip::NotHere),
        Seen::Plain(head) if is_head(&head) => {}
        // A `HEAD` that holds neither makes a folder that is no git folder, as for git.
        Seen::Plain(_) => return Err(Skip::NotHere),
        Seen::Odd | Seen::Large => return Err(Skip::Refused),
    }

    // `commondir` makes this a linked work tree's own folder, and `gitdir` names the work tree it belongs to; gix reads both.
    if !files::seen(&git_dir.join("gitdir"), POINTER_BYTES).is_fine() {
        return Err(Skip::Refused);
    }
    let (common_dir, has_commondir) = match files::read(&git_dir.join("commondir"), POINTER_BYTES) {
        Seen::Missing => (git_dir.to_path_buf(), false),
        Seen::Plain(bytes) => (common_dir(git_dir, &bytes).ok_or(Skip::Refused)?, true),
        Seen::Odd | Seen::Large => return Err(Skip::Refused),
    };

    // The folders gix asks for next, through a link or not, have to be local ones first, and real folders, not links.
    let objects = common_dir.join("objects");
    if !is_local(&common_dir) || !is_local(&objects) {
        return Err(Skip::Refused);
    }
    for name in ["objects", "refs"] {
        match files::folder(&common_dir.join(name)) {
            Seen::Plain(()) => {}
            Seen::Missing => return Err(Skip::NotHere),
            Seen::Odd | Seen::Large => return Err(Skip::Refused),
        }
    }

    Ok(Folders {
        common_dir,
        has_commondir,
    })
}

/// Whether the content of a `HEAD` is one a git folder has: a symbolic ref to a name below `refs/`, or an object id (of 40 or
/// 64 hex digits). A `HEAD` of anything else is no git folder, which is where git itself goes on to the parent folder.
pub(super) fn is_head(bytes: &[u8]) -> bool {
    let text = bytes.trim_end();

    match text.strip_prefix(b"ref:") {
        Some(name) => name.trim_start().starts_with(b"refs/"),
        None => matches!(text.len(), 40 | 64) && text.iter().all(u8::is_ascii_hexdigit),
    }
}

/// The folder a `commondir` names: its text without the white space at its end, joined to the git folder as gix joins it, and
/// not otherwise changed. `None` when it is empty, is no text, or leads to a place that is not local.
fn common_dir(git_dir: &Path, bytes: &[u8]) -> Option<PathBuf> {
    let named = plain_path(bytes)?;

    pointer_target(git_dir, &named)
}
