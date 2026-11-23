//! The places of a repository that was looked at, and what the helper that reads its status is allowed to touch: the work
//! tree, the git folder, the common folder and the folders of the alternates' object stores, each a physical path (no link on
//! it, see [`super::links::is_physical`]; an alternate's is the folder its path leads to) that is a local place and no broad
//! one (the root, the home folder or a folder above it, a file system of the kernel's own: [`is_too_broad`]).

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::files;
use super::links::{normalise, resolve_confined};
use super::names::{equals, starts_with};

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
        let mut seen: HashSet<PathBuf> = HashSet::new();

        for folder in [&self.workdir, &self.git_dir, &self.common_dir]
            .into_iter()
            .chain(&self.alternates)
        {
            let folder = normalise(folder);
            if seen.insert(folder.clone()) {
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
            .any(|folder| is_too_broad_with(folder, home.as_deref(), cfg!(target_os = "macos")))
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

/// The user's home folder as the system resolves it, where the search for a repository ends: the one `HOME` names, or, when
/// that is not set or is not a folder that is there, the one the user database (`getpwuid`) says.
pub(super) fn home() -> Option<PathBuf> {
    home_from(std::env::home_dir(), user_database_home)
}

/// The home folder from `named` (what the environment says) as the system resolves it, else from `database` (asked only then).
fn home_from(named: Option<PathBuf>, database: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    // `realpath` of a folder that is not there is the path itself, so the folder is looked at too.
    let resolved = |home: PathBuf| gix::path::realpath(home).ok().filter(|home| home.is_dir());

    named.and_then(resolved).or_else(|| database().and_then(resolved))
}

/// The home folder of the user who runs the app, as the user database has it.
#[cfg(unix)]
fn user_database_home() -> Option<PathBuf> {
    use uzers::os::unix::UserExt;

    uzers::get_user_by_uid(uzers::get_current_uid()).map(|user| user.home_dir().to_path_buf())
}

#[cfg(not(unix))]
fn user_database_home() -> Option<PathBuf> {
    None
}

/// Whether `folder` is too broad to be given to the helper to read: the root or a drive root, the user's home folder or a
/// folder above it (everything of the user's would be readable), a folder that every user shares (`/tmp`, `/var/tmp`, `/dev/shm`,
/// `/run/user/<uid>`, `/mnt`, `/media`: a repository whose own folder is one of them would be everything that anyone left
/// there; the folders below them are fine), or a place on a file system of the kernel's own (`/proc`, `/sys`: a repository is not
/// there, and what is, is what the kernel makes up).
pub(super) fn is_too_broad(folder: &Path) -> bool {
    is_too_broad_with(folder, home().as_deref(), cfg!(target_os = "macos"))
}

/// [`is_too_broad`] with the home folder given (`None`: not known), and whether this is macOS (`macos`: names are compared without
/// regard to case and to the way Unicode spells them, as its file system does; see [`super::names`]).
pub(super) fn is_too_broad_with(folder: &Path, home: Option<&Path>, macos: bool) -> bool {
    let folder = normalise(folder);

    folder.parent().is_none()
        || home.is_some_and(|home| starts_with(home, &folder, macos))
        || shared_folders().iter().any(|shared| equals(&folder, shared, macos))
        || files::on_pseudo_path(&folder)
}

/// The folders that are shared by every user of the machine, as they are spelled on Linux and (resolved) on macOS.
#[cfg(unix)]
fn shared_folders() -> Vec<PathBuf> {
    let uid = rustix::process::getuid().as_raw();
    let user = PathBuf::from(format!("/run/user/{uid}"));
    let fixed = [
        "/tmp",
        "/var/tmp",
        "/dev/shm",
        "/mnt",
        "/media",
        "/run/media",
        "/run/lock",
        "/private/tmp",
        "/private/var/tmp",
        "/Users/Shared",
        "/Volumes",
    ];
    // udisks2 mounts the user's removable media below `/run/media/<user name>`.
    let media = uzers::get_current_username().map(|name| PathBuf::from("/run/media").join(name));

    fixed
        .into_iter()
        .map(PathBuf::from)
        .chain([user])
        .chain(media)
        .collect()
}

/// Windows has no such folders by a fixed name.
#[cfg(not(unix))]
fn shared_folders() -> Vec<PathBuf> {
    Vec::new()
}

/// Whether two paths are the same place once the system has resolved both (and, on macOS, once they are precomposed, which
/// gix does to the paths of a repository that asks for it).
fn same(opened: &Path, looked_at: &Path) -> bool {
    let (Ok(opened), Ok(looked_at)) = (resolve_confined(opened), resolve_confined(looked_at)) else {
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
            assert!(is_too_broad_with(broad, Some(&home), false), "{broad:?}");
        }
        for fine in [below.as_path(), elsewhere.as_path()] {
            assert!(!is_too_broad_with(fine, Some(&home), false), "{fine:?}");
        }
        // Dots are read away first, and an unknown home folder leaves the root and the file systems of the kernel.
        assert!(is_too_broad_with(&below.join("../.."), Some(&home), false));
        assert!(is_too_broad_with(Path::new(root), None, false));
        assert!(!is_too_broad_with(&home, None, false));
    }

    #[test]
    fn the_home_folder_is_compared_without_regard_to_case_on_macos_only() {
        let home = PathBuf::from("/Users/Ann");

        for spelled in ["/users/ann", "/USERS", "/users/ANN", "/Users/An\u{6E}"] {
            assert!(is_too_broad_with(Path::new(spelled), Some(&home), true), "{spelled}");
        }
        // Where names are case-sensitive `/users/ann` is another folder than the home folder; `/USERS` is not above it.
        for spelled in ["/users/ann", "/USERS", "/users/ANN"] {
            assert!(!is_too_broad_with(Path::new(spelled), Some(&home), false), "{spelled}");
        }
        // A folder below the home folder is fine on both.
        assert!(!is_too_broad_with(Path::new("/USERS/ann/work"), Some(&home), true));
    }

    #[cfg(unix)]
    #[test]
    fn the_folders_every_user_shares_are_too_broad_themselves_and_what_is_below_them_is_not() {
        let user = format!("/run/user/{}", rustix::process::getuid().as_raw());
        let home = Path::new("/home/a");

        for shared in [
            "/tmp",
            "/tmp/",
            "/var/tmp",
            "/dev/shm",
            "/mnt",
            "/media",
            "/private/tmp",
            "/private/var/tmp",
            user.as_str(),
            // Dots are read away first.
            "/srv/../tmp",
        ] {
            assert!(is_too_broad_with(Path::new(shared), Some(home), false), "{shared}");
        }
        for fine in [
            "/tmp/repo",
            "/var/tmp/x/y",
            "/dev/shm/repo",
            "/mnt/disk",
            "/media/usb",
            "/private/tmp/work",
            "/run/user/0/x",
            "/run/user",
            "/tmpx",
            "/var",
        ] {
            assert!(!is_too_broad_with(Path::new(fine), Some(home), false), "{fine}");
        }
        // On macOS `/TMP` is `/tmp` too.
        assert!(is_too_broad_with(Path::new("/PRIVATE/TMP"), Some(home), true));
        assert!(!is_too_broad_with(Path::new("/PRIVATE/TMP"), Some(home), false));
    }

    #[test]
    fn the_home_folder_is_the_one_the_environment_names_and_else_the_user_database_has() {
        let folder = crate::gitstatus::testing::tempdir();
        let there = gix::path::realpath(folder.path()).expect("its real path");
        let gone = there.join("not-there");
        let database = there.join("database");
        std::fs::create_dir(&database).expect("a folder");
        let database_real = gix::path::realpath(&database).expect("its real path");

        // `HOME` is a folder that is there: the database is not asked.
        assert_eq!(
            home_from(Some(there.clone()), || panic!("not asked")),
            Some(there.clone())
        );
        // `HOME` is set but the folder is gone, or it is not set at all: the user database says.
        assert_eq!(
            home_from(Some(gone.clone()), || Some(database.clone())),
            Some(database_real.clone())
        );
        assert_eq!(home_from(None, || Some(database.clone())), Some(database_real));
        // Neither is a folder that is there.
        assert_eq!(home_from(Some(gone.clone()), || Some(gone)), None);
        assert_eq!(home_from(None, || None), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_folder_on_a_file_system_of_the_kernel_is_too_broad() {
        for folder in ["/proc", "/proc/self", "/sys", "/sys/kernel"] {
            if Path::new(folder).is_dir() {
                assert!(is_too_broad_with(Path::new(folder), None, false), "{folder}");
            }
        }
    }
}
