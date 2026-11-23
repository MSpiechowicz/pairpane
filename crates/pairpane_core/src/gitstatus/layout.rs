//! The places of a repository that was looked at, and what the helper that reads its status is allowed to touch: the work
//! tree, the git folder, the common folder and the folders of the alternates' object stores, each a physical path (no link on
//! it, see [`super::links::is_physical`]; an alternate's is the folder its path leads to) that is a local place and no broad
//! one (the root, the home folder or a folder above it, a file system of the kernel's own: [`is_too_broad`]).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::files;
use super::links::{normalise, resolve};

/// The folders of a repository that was found, as gix is to open them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Layout {
    /// The `.git` that is handed to gix: a folder, a file that points to one, or a link to a folder.
    pub(super) dot_git: PathBuf,
    /// The folder with `HEAD`, the `index` and the files of a work tree of its own.
    pub(super) git_dir: PathBuf,
    /// The folder with `config`, the objects and the refs: the same as `git_dir` unless this is a linked work tree.
    pub(super) common_dir: PathBuf,
    /// The work tree: the folder that holds the `.git`.
    pub(super) workdir: PathBuf,
    /// The folders of the alternates' object stores that may be read: each store's `info` and `pack` and the `xx` folders of
    /// loose objects that are there, each looked at and no more (`objects::alternates`), and never the store's own folder, which
    /// may hold anything (its own is below the common folder, which is granted).
    pub(super) alternates: Vec<PathBuf>,
    /// The bytes of `info/exclude` and `info/attributes` of the git folder and the common folder: the ignore and attribute
    /// rules of the repository itself, which the rules budget of a status starts with.
    pub(super) info_rules: u64,
}

/// A folder the helper may read, and which folder that is: the device and the inode it had when it was looked at. The helper
/// opens each folder to allow it and checks that it is the one (`Grant::is_unchanged`, and on Linux the same check on the
/// descriptor Landlock takes), so that a folder that was swapped for a link or another folder after the parent looked at it is
/// not allowed. On Windows there are no such numbers and nothing is allowed or denied by the helper (see the module docs of
/// [`super`]): both are 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Grant {
    pub(super) path: PathBuf,
    pub(super) device: u64,
    pub(super) inode: u64,
}

impl Grant {
    /// The grant of `path` as it is now (it is followed if it is a link, as the confinement opens it).
    fn of(path: PathBuf) -> io::Result<Self> {
        let (device, inode) = identity(&fs::metadata(&path)?);

        Ok(Self { path, device, inode })
    }

    /// Whether the folder is still the one that was looked at.
    #[cfg(target_os = "macos")]
    pub(super) fn is_unchanged(&self) -> bool {
        fs::metadata(&self.path).is_ok_and(|metadata| identity(&metadata) == (self.device, self.inode))
    }
}

/// The device and the inode of a file.
#[cfg(unix)]
fn identity(metadata: &fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;

    (metadata.dev(), metadata.ino())
}

#[cfg(not(unix))]
fn identity(_metadata: &fs::Metadata) -> (u64, u64) {
    (0, 0)
}

impl Layout {
    /// The folders a process that reads the status of this repository may read, and nothing else: the work tree, the git
    /// folder, the common folder and the alternates' folders, each once.
    pub(super) fn folders(&self) -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = Vec::new();

        for folder in [&self.workdir, &self.git_dir, &self.common_dir]
            .into_iter()
            .chain(&self.alternates)
        {
            let folder = normalise(folder);
            if !folders.contains(&folder) {
                folders.push(folder);
            }
        }

        folders
    }

    /// The folders of [`Layout::folders`] with what they are (see [`Grant`]); an error for one that cannot be looked at.
    pub(super) fn grants(&self) -> io::Result<Vec<Grant>> {
        self.folders().into_iter().map(Grant::of).collect()
    }

    /// Whether any of the work tree, the git folder and the common folder is too broad for the helper to be given to read
    /// (see [`is_too_broad`]).
    pub(super) fn is_too_broad(&self) -> bool {
        let home = home();

        [&self.workdir, &self.git_dir, &self.common_dir]
            .into_iter()
            .any(|folder| is_too_broad_with(folder, home.as_deref()))
    }

    /// Whether the repository gix opened is the one that was looked at: the same git folder, common folder and work tree,
    /// as the system resolves them (a path with links on it is not the same as one that has none). When it is not, gix took a
    /// path from somewhere that this did not look at, and the repository must not be read.
    pub(super) fn is_opened(&self, repository: &gix::Repository) -> bool {
        same(repository.git_dir(), &self.git_dir)
            && same(repository.common_dir(), &self.common_dir)
            && repository.workdir().is_some_and(|workdir| same(workdir, &self.workdir))
    }
}

/// The user's home folder as the system resolves it, where the search for a repository ends.
pub(super) fn home() -> Option<PathBuf> {
    std::env::home_dir().and_then(|home| gix::path::realpath(home).ok())
}

/// Whether `folder` is too broad to be given to the helper to read: the root or a drive root, the user's home folder or a
/// folder above it (everything of the user's would be readable), or a place on a file system of the kernel's own (`/proc`,
/// `/sys`: a repository is not there, and what is, is what the kernel makes up).
pub(super) fn is_too_broad(folder: &Path) -> bool {
    is_too_broad_with(folder, home().as_deref())
}

/// [`is_too_broad`] with the home folder given (`None`: not known).
pub(super) fn is_too_broad_with(folder: &Path, home: Option<&Path>) -> bool {
    let folder = normalise(folder);

    folder.parent().is_none() || home.is_some_and(|home| home.starts_with(&folder)) || files::on_pseudo_path(&folder)
}

/// Whether two paths are the same place once the system has resolved both (and, on macOS, once they are precomposed, which
/// gix does to the paths of a repository that asks for it).
fn same(opened: &Path, looked_at: &Path) -> bool {
    let (Ok(opened), Ok(looked_at)) = (resolve(opened), resolve(looked_at)) else {
        return false;
    };

    if opened == looked_at {
        return true;
    }

    cfg!(target_os = "macos")
        && gix::utils::str::precompose_path(opened.as_path().into())
            == gix::utils::str::precompose_path(looked_at.as_path().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_the_home_folder_and_what_is_above_it_are_too_broad_and_what_is_below_it_is_not() {
        let root = if cfg!(windows) { r"C:\" } else { "/" };
        let home = if cfg!(windows) {
            PathBuf::from(r"C:\Users\a")
        } else {
            PathBuf::from("/home/a")
        };
        let below = home.join("work/repo");
        let elsewhere = if cfg!(windows) {
            PathBuf::from(r"D:\repos\a")
        } else {
            PathBuf::from("/srv/repos/a")
        };

        for broad in [Path::new(root), home.parent().expect("a parent"), home.as_path()] {
            assert!(is_too_broad_with(broad, Some(&home)), "{broad:?}");
        }
        for fine in [below.as_path(), elsewhere.as_path()] {
            assert!(!is_too_broad_with(fine, Some(&home)), "{fine:?}");
        }
        // Dots are read away first, and an unknown home folder leaves the root and the file systems of the kernel.
        assert!(is_too_broad_with(&below.join("../.."), Some(&home)));
        assert!(is_too_broad_with(Path::new(root), None));
        assert!(!is_too_broad_with(&home, None));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_folder_on_a_file_system_of_the_kernel_is_too_broad() {
        for folder in ["/proc", "/proc/self", "/sys", "/sys/kernel"] {
            if Path::new(folder).is_dir() {
                assert!(is_too_broad_with(Path::new(folder), None), "{folder}");
            }
        }
    }
}
