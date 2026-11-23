//! The files of a repository that gix is about to read, looked at first (see [`super::files`]): each has to be a regular
//! file within its cap, so that a named pipe, a device or a link to `/dev/zero` in a place where a file is expected never
//! reaches an open that waits for ever or a read that does not end. The folders gix goes through to those files are looked at
//! too: `refs`, `objects`, `objects/info`, `objects/pack` and each `objects/xx` of every object folder must be real folders and
//! no link (or reparse point), as a link there would lead to files that were not looked at. Everything that is not here (the
//! content of a loose object, the blobs of the index) is read by gix, and what holds up a read of those is the time limit of
//! the caller, which answers for the request whether or not the read returns.

use std::fs;
use std::path::{Path, PathBuf};

use super::files::{self, Seen};
use super::locality::pointer_target;
use super::locate::{Layout, POINTER_BYTES};

/// The most a repository's `config` may hold.
const CONFIG_BYTES: u64 = 1024 * 1024;

/// The most an `index` (or a shared index) may hold: a bigger index is too big to read here.
pub(super) const INDEX_BYTES: u64 = 64 * 1024 * 1024;

/// The most a `packed-refs` may hold.
const PACKED_REFS_BYTES: u64 = 64 * 1024 * 1024;

/// The most the list of alternate object folders may hold, and how many lines and how deep a chain of them is followed.
const ALTERNATES_BYTES: u64 = 64 * 1024;
const ALTERNATES_LINES: usize = 32;
const ALTERNATES_DEPTH: usize = 5;

/// The most a ref may lead through with symbolic refs (gix's own limit is the same).
const REF_HOPS: usize = 5;

/// The most entries of one folder that is listed to be looked at (the pack folder, `.git`).
const LISTED: usize = 4_096;

/// Why a repository must not be read.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// A file is not a plain one, a pointer leads to a place that is not local, or a file is over its cap.
    Odd,
    /// The index is over its cap.
    TooBig,
}

/// A file that is not there or is plain is fine; anything else is a repository not to read.
fn fine<T>(seen: Seen<T>) -> Result<(), Refusal> {
    match seen {
        Seen::Missing | Seen::Plain(_) => Ok(()),
        Seen::Odd | Seen::Large => Err(Refusal::Odd),
    }
}

/// Looks at every file of the repository that gix reads on opening it and on a status.
pub(super) fn files(layout: &Layout) -> Result<(), Refusal> {
    index(&layout.git_dir)?;

    let mut folders = vec![layout.git_dir.as_path()];
    if layout.common_dir != layout.git_dir {
        folders.push(layout.common_dir.as_path());
    }
    for folder in &folders {
        // The ignore and attribute rules of the repository's own folder. gix reads them through a link, and a link is fine when
        // it leads to a regular file of a local place (or nowhere, which gix reads as no rules): see [`files::seen_following`].
        for name in ["info/exclude", "info/attributes"] {
            fine(files::seen_following_below(folder, name, CONFIG_BYTES))?;
        }
    }

    fine(files::seen(&layout.common_dir.join("config"), CONFIG_BYTES))?;
    fine(files::seen(&layout.git_dir.join("config.worktree"), CONFIG_BYTES))?;
    fine(files::seen(&layout.common_dir.join("packed-refs"), PACKED_REFS_BYTES))?;
    refs(layout)?;

    for objects in object_folders(&layout.common_dir)? {
        object_folder(&objects)?;
    }

    Ok(())
}

/// The index, and the shared index it may point to (a split index), which gix reads next to it.
fn index(git_dir: &Path) -> Result<(), Refusal> {
    check_index(&git_dir.join("index"))?;

    listed(git_dir, |entry| {
        if entry.file_name().to_string_lossy().starts_with("sharedindex.") {
            check_index(&entry.path())?;
        }

        Ok(())
    })
}

/// Goes through the entries of `folder` (not there: none), at most [`LISTED`] of them, with `each`.
fn listed(folder: &Path, mut each: impl FnMut(&fs::DirEntry) -> Result<(), Refusal>) -> Result<(), Refusal> {
    let Ok(listing) = fs::read_dir(folder) else {
        return Ok(());
    };

    for (position, entry) in listing.enumerate() {
        if position >= LISTED {
            return Err(Refusal::Odd);
        }

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
    let mut text = match files::read_following(&layout.git_dir.join("HEAD"), POINTER_BYTES) {
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

/// The object folders: the repository's own and the alternates it names, which gix opens too, each with its own
/// `info/alternates` (up to a depth). The lists are parsed by gix's own parser (`gix::odb::alternate::parse`: comments, quoted
/// entries, and `\r` is kept as part of the name), and every relative entry of every list, at any depth, is joined to the
/// objects folder of the repository itself, as gix joins it (`gix_odb::alternate::resolve`), and not to the folder of the list
/// it was read from. One that is not a local place, or a list that is too long, is a repository not to read.
fn object_folders(common_dir: &Path) -> Result<Vec<PathBuf>, Refusal> {
    let own = common_dir.join("objects");
    let mut folders = vec![own.clone()];
    let mut pending = vec![(own.clone(), 0)];

    while let Some((folder, depth)) = pending.pop() {
        let bytes = match files::read_below(&folder, "info/alternates", ALTERNATES_BYTES) {
            Seen::Missing => continue,
            Seen::Plain(bytes) => bytes,
            Seen::Odd | Seen::Large => return Err(Refusal::Odd),
        };
        let entries = gix::odb::alternate::parse(&bytes).map_err(|_| Refusal::Odd)?;

        for entry in entries {
            if depth + 1 > ALTERNATES_DEPTH || folders.len() >= ALTERNATES_LINES {
                return Err(Refusal::Odd);
            }

            // Not a local place: the repository is not read, and nothing at the other end is touched.
            let target = pointer_target(&own, &entry).ok_or(Refusal::Odd)?;
            folders.push(target.clone());
            pending.push((target, depth + 1));
        }
    }

    Ok(folders)
}

/// The structure of one object folder, which gix goes through to read an object: the folder itself, `info`, `pack` and each
/// of the 256 folders of loose objects that is there must be a real folder, and no link or reparse point (one would lead to
/// files that were not looked at), and the files of `pack` must be plain ones: gix opens the pack and its index when an object
/// is in it.
fn object_folder(objects: &Path) -> Result<(), Refusal> {
    let fanned_out = (0..=u8::MAX).map(|byte| objects.join(format!("{byte:02x}")));
    let folders = [objects.to_path_buf(), objects.join("info"), objects.join("pack")];

    for folder in folders.into_iter().chain(fanned_out) {
        match files::folder(&folder) {
            Seen::Plain(()) | Seen::Missing => {}
            Seen::Odd | Seen::Large => return Err(Refusal::Odd),
        }
    }

    listed(&objects.join("pack"), |entry| {
        fine(files::seen(&entry.path(), u64::MAX))
    })
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

    #[test]
    fn a_pointer_of_the_list_of_alternates_that_is_not_local_is_refused() {
        let folder = tempfile::tempdir().expect("a folder");
        let common = folder.path().join("repo.git");
        fs::create_dir_all(common.join("objects/info")).expect("folders");
        let alternates = common.join("objects/info/alternates");

        fs::write(&alternates, "# comment\n\n../../other/objects\n").expect("a list");
        assert_eq!(object_folders(&common).expect("local").len(), 2);

        for line in [
            r"\\host\share\objects",
            "//host/share/objects",
            r"\\?\UNC\host\share\objects",
        ] {
            fs::write(&alternates, format!("../../fine/objects\n{line}\n")).expect("a list");

            assert_eq!(object_folders(&common), Err(Refusal::Odd), "{line}");
        }
    }

    #[test]
    fn a_chain_of_alternates_is_followed_and_a_loop_ends() {
        let folder = tempfile::tempdir().expect("a folder");
        let (one, two) = (folder.path().join("one/objects"), folder.path().join("two/objects"));
        for objects in [&one, &two] {
            fs::create_dir_all(objects.join("info")).expect("folders");
        }
        fs::write(one.join("info/alternates"), format!("{}\n", two.display())).expect("a list");
        fs::write(two.join("info/alternates"), format!("{}\n", one.display())).expect("a list");

        // Each step names the other: more steps than the depth allows.
        assert_eq!(object_folders(&folder.path().join("one")), Err(Refusal::Odd));
    }
}
