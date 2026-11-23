//! Finding the repository a folder is in, before gix is let near it. gix discovers a repository by walking up and reading
//! what it finds (`HEAD`, `commondir`, `gitdir`, the target of a `.git` file); here the same walk is done with only `lstat`
//! and small, non-blocking reads, so that what a repository names is looked at first: a pointer to a network place or with a
//! link on its way, and a file that is no regular file (or is a link), make the folder "not in a repository" (see
//! [`super::locality`], [`super::links`] and [`super::files`]). The repository found is the one gix would find: the first
//! folder going up that has a `.git` (a folder, or a file that points to one, or a link to a folder) with a valid `HEAD`, an
//! `objects` and a `refs` folder, a filesystem boundary (Unix) ends the search, and so does the `ceiling`, the user's home
//! folder, which is the last folder tried.
//!
//! What is read is read the way gix reads it, so that what is looked at is what gix opens: a `.git` file is parsed with gix's
//! own `gitdir:` parser (`gix::discover::parse::gitdir`: one space after the colon, and the text up to the end of the file
//! without its trailing white space), a `HEAD` with gix's parser of a loose reference ([`is_head`]), a `commondir` is the file
//! without its trailing white space, and a `.git` folder that has both a `commondir` and a `gitdir` is the folder of a linked
//! work tree, whose work tree gix takes from the content of the `gitdir` file (so that is refused unless it names this very
//! `.git`). The [`Layout`] that this finds is what the repository must be once gix has opened it (see
//! [`Layout::is_opened`](super::layout::Layout::is_opened)) and what the helper that reads it may read.

use std::fs;
use std::path::{Path, PathBuf};

use gix::bstr::ByteSlice;

use super::GitError;
use super::files::{self, Seen};
use super::inspect::{self, Refusal};
use super::layout::Layout;
use super::links::{self, is_local, normalise};
use super::locality::pointer_target;
use crate::listing::platform;

/// The most a `HEAD`, a `commondir`, a `gitdir` or a `.git` file may hold.
pub(super) const POINTER_BYTES: u64 = 4 * 1024;

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

fn found(mut layout: Layout) -> Result<Option<Layout>, GitError> {
    match inspect::files(&layout) {
        Ok(sources) => {
            layout.alternates = sources.alternates;
            layout.info_rules = sources.info_rules;

            Ok(Some(layout))
        }
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

/// The repository the `.git` of `dir` is: a folder, a file that points to a folder, or a link to a folder. A link is followed
/// hop by hop to a local place, and the folder at the end of it is the git folder (a path that has no link on it, so that it is
/// the one every place below it is looked at and opened by); a link to anything else is not read.
fn dot_git_folder(dir: &Path) -> Result<Layout, Skip> {
    let dot_git = dir.join(".git");
    let metadata = match fs::symlink_metadata(&dot_git) {
        Ok(metadata) => metadata,
        Err(error) if links::leads_nowhere(&error) => return Err(Skip::NotHere),
        Err(_) => return Err(Skip::Refused),
    };

    if metadata.file_type().is_symlink() {
        let resolved = match links::resolve(&dot_git) {
            Ok(resolved) => resolved,
            Err(error) if links::leads_nowhere(&error) => return Err(Skip::NotHere),
            Err(_) => return Err(Skip::Refused),
        };
        return match fs::symlink_metadata(&resolved) {
            Ok(target) if target.is_dir() => own_folder(dir, dot_git, resolved),
            Ok(_) => Err(Skip::Refused),
            Err(_) => Err(Skip::NotHere),
        };
    }

    if metadata.is_dir() {
        let git_dir = dot_git.clone();

        own_folder(dir, dot_git, git_dir)
    } else if metadata.is_file() {
        pointed_to(dir, dot_git)
    } else {
        Err(Skip::Refused)
    }
}

/// The repository whose own folder is `git_dir` (the `.git` folder `dot_git` is, or what it links to).
fn own_folder(dir: &Path, dot_git: PathBuf, git_dir: PathBuf) -> Result<Layout, Skip> {
    let folders = self::git_dir(&git_dir)?;
    names_this_folder(&git_dir, &folders)?;

    Ok(Layout {
        git_dir,
        common_dir: folders.common_dir,
        workdir: dir.to_path_buf(),
        dot_git,
        alternates: Vec::new(),
        info_rules: 0,
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

/// The repository a `.git` file points to with its `gitdir: <path>` line.
fn pointed_to(dir: &Path, dot_git: PathBuf) -> Result<Layout, Skip> {
    let bytes = match files::read(&dot_git, POINTER_BYTES) {
        Seen::Plain(bytes) => bytes,
        Seen::Missing => return Err(Skip::NotHere),
        Seen::Odd | Seen::Large => return Err(Skip::Refused),
    };
    // gix's own parser, so that the path is the one gix reads: it has no trimming at the start and it takes the whole file.
    let Ok(pointer) = gix::discover::parse::gitdir(&bytes) else {
        return Err(Skip::NotHere);
    };

    // Not a local place, or one that has a link on its way: the folder is in no repository, and nothing at the other end is
    // touched.
    let target = normalise(&pointer_target(dir, &pointer).ok_or(Skip::Refused)?);
    match fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.is_dir() => {}
        _ => return Err(Skip::NotHere),
    }

    let folders = git_dir(&target)?;

    Ok(Layout {
        git_dir: target,
        common_dir: folders.common_dir,
        workdir: dir.to_path_buf(),
        dot_git,
        alternates: Vec::new(),
        info_rules: 0,
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

    // No file of the repository is a link, `HEAD` included (git's old way of writing a symbolic ref): such a repository is not
    // read.
    match files::read(&git_dir.join("HEAD"), POINTER_BYTES) {
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

/// Whether the content of a `HEAD` is one a git folder has: what gix's own parser of a loose reference takes (a symbolic ref
/// with a valid name, or an object id), which is where git itself goes on to the parent folder otherwise. An object id of a
/// SHA-256 repository (64 hex digits) is one too: it is a git folder of a format that is not read, and gix says so when it
/// opens it (`repo: false`, and the search does not go on to a parent that holds another repository).
pub(super) fn is_head(bytes: &[u8]) -> bool {
    let Ok(name) = gix::refs::FullName::try_from("HEAD") else {
        return false;
    };
    if gix::refs::file::loose::Reference::try_from_path(name, bytes, gix::hash::Kind::Sha1).is_ok() {
        return true;
    }

    let id = bytes.trim_end_with(|c| c == '\n' || c == '\r');

    id.len() == 64 && id.iter().all(u8::is_ascii_hexdigit)
}

/// The folder a `commondir` names: its text without the white space at its end, joined to the git folder as gix joins it, and
/// not otherwise changed. `None` when it is empty, is no text, or leads to a place that is not local.
fn common_dir(git_dir: &Path, bytes: &[u8]) -> Option<PathBuf> {
    let named = plain_path(bytes)?;

    pointer_target(git_dir, &named).map(|target| normalise(&target))
}
