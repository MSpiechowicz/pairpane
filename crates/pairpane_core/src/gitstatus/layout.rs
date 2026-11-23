//! The places of a repository that was looked at, and what the helper that reads its status is allowed to touch: the work
//! tree, the git folder, the common folder and the object folders of the alternates, each a physical path (no link on it, see
//! [`super::links::is_physical`]) that is a local place.

use std::path::{Path, PathBuf};

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
    /// The object folders of the alternates the repository names (its own `objects` is below the common folder).
    pub(super) alternates: Vec<PathBuf>,
    /// The bytes of `info/exclude` and `info/attributes` of the git folder and the common folder: the ignore and attribute
    /// rules of the repository itself, which the rules budget of a status starts with.
    pub(super) info_rules: u64,
}

impl Layout {
    /// The folders a process that reads the status of this repository may read, and nothing else: the work tree, the git
    /// folder, the common folder and the alternates' object folders, each once.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
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

    /// Whether the repository gix opened is the one that was looked at: the same git folder, common folder and work tree,
    /// as the system resolves them (a path with links on it is not the same as one that has none). When it is not, gix took a
    /// path from somewhere that this did not look at, and the repository must not be read.
    pub(super) fn is_opened(&self, repository: &gix::Repository) -> bool {
        same(repository.git_dir(), &self.git_dir)
            && same(repository.common_dir(), &self.common_dir)
            && repository.workdir().is_some_and(|workdir| same(workdir, &self.workdir))
    }
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
