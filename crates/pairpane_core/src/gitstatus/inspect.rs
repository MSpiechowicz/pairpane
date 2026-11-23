//! The files of a repository that gix is about to read, looked at first (see [`super::files`]): each has to be a regular
//! file within its cap (by the size of the file that is opened, not by what `lstat` says), so that a named pipe, a device or a
//! link to `/dev/zero` in a place where a file is expected never reaches an open that waits for ever or a read that does not
//! end. None of them may be a link: `HEAD`, `info/exclude`, `info/attributes`, `config`, `config.worktree`, `commondir`, the
//! `index`, `packed-refs` and the files of the refs are what they are or the repository is not read (a deliberate trade: git
//! itself follows a link there, and a `HEAD` that is a link to a branch, or an `info/exclude` that is shared between
//! repositories by a link, is a repository without a status here). The folders gix goes through to those files are looked at
//! too: `refs`, `info`, `objects`, `objects/info`, `objects/pack` and each `objects/xx` of every object folder must be real
//! folders and no link (or reparse point), as a link there would lead to files that were not looked at, and so must every
//! entry of the folders of loose objects, of `objects/pack` and of the commit graph folders be a regular file (see
//! [`super::objects`]). Everything that is not here (the content of a loose object, the blobs of the index) is read by gix inside
//! the process that the status runs in, which is held to a memory and a time limit, and may read nothing but the folders of the
//! repository.

use std::fs;
use std::path::{Path, PathBuf};

use super::budget::SOURCE_BYTES;
use super::files::{self, Seen};
use super::gate::{Gate, Halt};
use super::layout::Layout;
use super::locate::POINTER_BYTES;
use super::objects;

/// The most a repository's `config` may hold.
const CONFIG_BYTES: u64 = 1024 * 1024;

/// The most an `index` (or a shared index) may hold: a bigger index is too big to read here.
pub(super) const INDEX_BYTES: u64 = 64 * 1024 * 1024;

/// The most a `packed-refs` may hold.
const PACKED_REFS_BYTES: u64 = 64 * 1024 * 1024;

/// The most a ref may lead through with symbolic refs (gix's own limit is the same).
const REF_HOPS: usize = 5;

/// The most entries of the folders that are listed to be looked at, over a whole repository: the loose objects, the packs, the
/// incremental multi-pack index and the commit graphs of every object folder (its own and every alternate's), and the git folder.
/// A repository with more is too big to read here (auto-gc keeps an ordinary one far below it). Each entry costs a look, and the
/// alternates may name as many folders as they like, so this is what bounds the parent's work, whatever the repository says.
pub(super) const ENTRIES: usize = 65_536;

/// Why a repository must not be read.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// A file is not a plain one, a pointer leads to a place that is not local, or a file is over its cap.
    Odd,
    /// A folder the repository names is too broad to give the helper to read (see [`super::layout::is_too_broad`]).
    Broad,
    /// A folder the repository names (an alternate object store) belongs to another user.
    Owner,
    /// The index is over its cap, or the repository has more entries than [`ENTRIES`].
    TooBig,
    /// The look was stopped: the app is exiting, or the time limit is up.
    Stopped(Halt),
}

impl From<Halt> for Refusal {
    fn from(halt: Halt) -> Self {
        Self::Stopped(halt)
    }
}

/// What the look at one repository is held to: its gate (the exit flag and the time limit) and the entries it may still list
/// ([`ENTRIES`]), the same for every folder of the repository.
pub(super) struct Looking<'a> {
    gate: Gate<'a>,
    left: usize,
}

impl<'a> Looking<'a> {
    pub(super) fn new(gate: Gate<'a>) -> Self {
        Self::with_entries(gate, ENTRIES)
    }

    /// A look that may list `left` entries.
    pub(super) fn with_entries(gate: Gate<'a>, left: usize) -> Self {
        Self { gate, left }
    }

    /// The gate: whether the look may go on.
    pub(super) fn gate(&self) -> Gate<'a> {
        self.gate
    }

    /// Counts one more entry listed, after asking the gate: [`Refusal::TooBig`] past the cap.
    fn count(&mut self) -> Result<(), Refusal> {
        self.gate.check()?;
        self.left = self.left.checked_sub(1).ok_or(Refusal::TooBig)?;

        Ok(())
    }
}

/// What the files of a repository add up to for the rules budget, and where its alternates are.
pub(super) struct Sources {
    /// The folders of the alternates' object stores that the helper may read (see `Layout::alternates`).
    pub(super) alternates: Vec<PathBuf>,
    /// The bytes of `info/exclude` and `info/attributes` of the git folder and of the common folder.
    pub(super) info_rules: u64,
}

/// A file that is not there or is plain is fine; anything else is a repository not to read.
pub(super) fn fine<T>(seen: Seen<T>) -> Result<(), Refusal> {
    match seen {
        Seen::Missing | Seen::Plain(_) => Ok(()),
        Seen::Odd | Seen::Large => Err(Refusal::Odd),
    }
}

/// Looks at every file of the repository that gix reads on opening it and on a status.
pub(super) fn files(layout: &Layout, looking: &mut Looking<'_>) -> Result<Sources, Refusal> {
    index(&layout.git_dir, looking)?;

    let mut folders = vec![layout.git_dir.as_path()];
    if layout.common_dir != layout.git_dir {
        folders.push(layout.common_dir.as_path());
    }
    let mut info_rules = 0;
    for folder in &folders {
        looking.gate().check()?;
        // The ignore and attribute rules of the repository's own folder: read whole, as they are the first of the rules that
        // the budget of a status counts. One that is over the cap of a source is a repository too big to read.
        for name in ["info/exclude", "info/attributes"] {
            match files::read_below(folder, name, SOURCE_BYTES) {
                Seen::Missing => {}
                Seen::Plain(bytes) => info_rules += bytes.len() as u64,
                Seen::Large => return Err(Refusal::TooBig),
                Seen::Odd => return Err(Refusal::Odd),
            }
        }
    }

    for config in [layout.common_dir.join("config"), layout.git_dir.join("config.worktree")] {
        looking.gate().check()?;
        match files::read(&config, CONFIG_BYTES) {
            Seen::Missing | Seen::Plain(_) => {}
            Seen::Odd | Seen::Large => return Err(Refusal::Odd),
        }
    }
    fine(files::seen(&layout.common_dir.join("packed-refs"), PACKED_REFS_BYTES))?;
    refs(layout)?;

    Ok(Sources {
        alternates: objects::alternates(&layout.common_dir, looking)?,
        info_rules,
    })
}

/// The index, and the shared index it may point to (a split index), which gix reads next to it.
fn index(git_dir: &Path, looking: &mut Looking<'_>) -> Result<(), Refusal> {
    check_index(&git_dir.join("index"))?;

    listed(git_dir, looking, |entry| {
        if entry.file_name().to_string_lossy().starts_with("sharedindex.") {
            check_index(&entry.path())?;
        }

        Ok(())
    })
}

/// Goes through the entries of `folder` (not there: none) with `each`, every one counted in `looking` (which also asks its gate
/// before each): past the entries a repository may have the look ends with [`Refusal::TooBig`].
pub(super) fn listed(
    folder: &Path,
    looking: &mut Looking<'_>,
    mut each: impl FnMut(&fs::DirEntry) -> Result<(), Refusal>,
) -> Result<(), Refusal> {
    looking.gate().check()?;
    let Ok(listing) = fs::read_dir(folder) else {
        return Ok(());
    };

    for entry in listing {
        looking.count()?;

        each(&entry.map_err(|_| Refusal::Odd)?)?;
    }

    Ok(())
}

fn check_index(path: &Path) -> Result<(), Refusal> {
    match files::seen(path, INDEX_BYTES) {
        Seen::Missing | Seen::Plain(()) => Ok(()),
        Seen::Odd => Err(Refusal::Odd),
        Seen::Large => Err(Refusal::TooBig),
    }
}

/// The ref `HEAD` is, followed through up to [`REF_HOPS`] symbolic refs to the loose file of a branch (its file is where gix
/// reads it from, in the common folder, or in the work tree's own folder for a ref of that work tree). A ref that is in
/// `packed-refs` has no file, and `HEAD` that is an id has none either.
fn refs(layout: &Layout) -> Result<(), Refusal> {
    let mut text = match files::read(&layout.git_dir.join("HEAD"), POINTER_BYTES) {
        Seen::Plain(bytes) => bytes,
        Seen::Missing => return Ok(()),
        Seen::Odd | Seen::Large => return Err(Refusal::Odd),
    };

    for _ in 0..=REF_HOPS {
        let Some(name) = symbolic_target(&text)? else {
            return Ok(());
        };

        text = match loose_ref(layout, &name)? {
            Some(bytes) => bytes,
            None => return Ok(()),
        };
    }

    // Longer than gix follows: it is no repository to read.
    Err(Refusal::Odd)
}

/// The ref a symbolic ref (`ref: refs/heads/main`) names, or `None` for a ref that holds an id.
fn symbolic_target(text: &[u8]) -> Result<Option<String>, Refusal> {
    let Ok(text) = std::str::from_utf8(text) else {
        return Err(Refusal::Odd);
    };
    let Some(name) = text.strip_prefix("ref:") else {
        return Ok(None);
    };
    let name = name.trim();
    let plain = name.starts_with("refs/")
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains(['\\', '\0', ':']));

    if plain {
        Ok(Some(name.to_owned()))
    } else {
        Err(Refusal::Odd)
    }
}

/// The content of the loose file of the ref `name`, or `None` when it has none (it is packed, or not there).
fn loose_ref(layout: &Layout, name: &str) -> Result<Option<Vec<u8>>, Refusal> {
    let mut found = None;
    let mut folders = vec![layout.common_dir.as_path()];
    if layout.git_dir != layout.common_dir {
        folders.push(layout.git_dir.as_path());
    }

    for folder in folders {
        match files::read_below(folder, name, POINTER_BYTES) {
            Seen::Plain(bytes) => {
                found.get_or_insert(bytes);
            }
            Seen::Missing => {}
            Seen::Odd | Seen::Large => return Err(Refusal::Odd),
        }
    }

    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_symbolic_ref_names_a_ref_below_refs_and_nothing_that_climbs() {
        assert_eq!(
            symbolic_target(b"ref: refs/heads/main\n"),
            Ok(Some("refs/heads/main".to_owned()))
        );
        assert_eq!(symbolic_target(b"0123456789012345678901234567890123456789\n"), Ok(None));

        for bad in [
            "ref: ../../etc/passwd",
            "ref: refs/../../x",
            "ref: /dev/zero",
            "ref: refs//x",
            "ref: refs\\x",
            "ref: refs/heads/a:b",
            "ref: HEAD",
            "ref: ",
        ] {
            assert_eq!(symbolic_target(bad.as_bytes()), Err(Refusal::Odd), "{bad}");
        }
    }
}
