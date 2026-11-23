//! The git status column: the state of the direct children of a folder that is inside a git repository, read in process
//! by `gix` (gitoxide, pure Rust). The `git` program is never run, and no shell is.
//!
//! # Trust model: nothing the repository says can run anything
//!
//! The folders a user browses are untrusted: a download or an unpacked archive can hold a `.git` folder that was made to
//! start a program the moment a tool "just reads its status" (`core.fsmonitor`, a filter driver, a hook, a credential
//! helper, `core.sshCommand`, an external diff, an `include` of a config file elsewhere). Reading a status here is built so
//! that none of these can happen, in layers, each of which is enough by itself for the part it covers:
//!
//! 1. **The configuration is the repository's own file only** (`open::open_options`). The options start from
//!    `gix::open::Options::isolated()`, whose `Permissions::isolated()` has the config sources `git_binary`, `system`,
//!    `git`, `user` and `env` off, `includes` off (an `include.path` or `includeIf` is never followed, so a config file
//!    elsewhere is never read) and every environment permission denied (`Environment::isolated()`: no `GIT_*`, `HOME`,
//!    `XDG_CONFIG_HOME`, `SSH_*` and so on are read). `git_binary: false` also means gix never starts `git` to find its
//!    installation config. As a result no setting of the user's own git configuration applies either, and the global
//!    ignore file (`core.excludesFile`, `$XDG_CONFIG_HOME/git/ignore`) is not read: only `.gitignore` files in the work
//!    tree and `.git/info/exclude` give the `ignored` mark.
//! 2. **The repository is opened as not owned, with reduced trust.** `Options::with_enforced_trust(Trust::Reduced)` sets
//!    the final trust level to `gix::sec::Trust::Reduced` whoever owns the folder, and discovery runs with
//!    `TrustPolicy::Assume(Trust::Reduced)`, so ownership (and `safe.directory`) is never used to raise it.
//! 3. **No section of the repository's config is used for any value that names a program.** gix reads the values that
//!    can start programs through a filter on the config sections (`Options::filter_config_section`): the drivers
//!    `filter.<name>.clean|smudge|process` (`gix::filter::Pipeline::options`), `diff.<name>.textconv|command`
//!    (`config::Cache::diff_drivers`), and `core.worktree` and the index file path while opening. Its default filter
//!    accepts only the sections of a fully trusted file or of a source outside the repository, so a reduced-trust open has
//!    none of the repository's; we also replace it with a stricter one that accepts no section that comes from the
//!    repository at all (`open::trusts_no_repository_section`), so that a change of the trust rules in gix could not
//!    let one through. What is still read from the repository's own config are values gix needs for correctness that start
//!    nothing: `core.bare`, `core.ignoreCase`, `core.filemode`, `core.autocrlf` and the like. Other keys that name a program
//!    (`core.fsmonitor`, `core.hooksPath`, `core.sshCommand`, `core.askPass`, `credential.helper`, `gpg.program`,
//!    `diff.external`, `merge.<name>.driver`) are not read by anything a status reaches.
//! 4. **The filter pipeline has no driver.** The status compares a changed file to the index by hashing it through
//!    gix's filter pipeline (`gix_filter::Pipeline`), which is where a `clean` or `process` driver would run. The drivers
//!    of that pipeline are built from the `filter.*` sections that point 3 filters out, so its driver list is empty, and
//!    `* filter=x` in `.gitattributes` names a driver that is not there: gix skips an attribute that names no known driver
//!    (`gix_filter::pipeline::util::Configuration`, `extract_driver`). What remains is the pipeline's own conversions
//!    (line endings, `ident`), done in process. A file whose index entry was made through a real `clean` filter compares by
//!    its raw content and so shows as `modified` (content that cannot be compared without the filter counts as changed).
//!    A text conversion for diffs (`textconv`) and an external diff are not used by a status at all.
//! 5. **Nothing else is compiled in that could start something.** The `gix` dependency has no default features: no network
//!    client (`gix-transport` has no client code and `gix-protocol` is linked for its types only), no `credentials`
//!    (`gix-credentials` and `gix-prompt` are not in the build: no credential helper, no askpass), no `fsmonitor`, `blame`,
//!    `merge` or `worktree-mutation`. gix has no hook runner (hooks are only written, as samples, when it creates a
//!    repository), and a status never reads `core.fsmonitor`. The crates in the build that can start a process
//!    (`gix-filter` for drivers, `gix-diff` for an external diff or `textconv`, `gix-command`, and `gix-path` for the `git`
//!    program) are given nothing to start by points 1, 3 and 4, and no code of ours calls them otherwise.
//! 6. **Submodules are never opened.** The submodule status of the gix status (which opens the submodule's own repository
//!    and runs a status there, with that repository's own trust) is replaced by `compare::NoSubmodules`, which answers "no
//!    change" without reading `.gitmodules` or touching the submodule. A submodule folder therefore has a mark only for
//!    what the superproject sees of it (an untracked or staged gitlink), never for its own state: it is never "dirty".
//!    Nested repositories that are not submodules are not entered either (`recurse_repositories` is off, so the
//!    directory walk only checks that a `.git` is there, reading its structure but not its config).
//! 7. **Read only.** Nothing is written: the index is not refreshed, no lock file is made, nothing is created in `.git`.
//!    A `.git` file (a linked worktree or a submodule) that points elsewhere is read like a `.git` folder; the work tree is
//!    the folder that holds the `.git` file, and it must contain the folder asked for, or the folder is reported as not in
//!    a repository (a `core.worktree` pointing at another checkout is ignored anyway by point 3).
//! 8. **What the repository names is looked at before it is read** (`locate`, `inspect`, `locality`, `files`). The
//!    repository is found by a walk of our own, with `lstat` and small reads that neither wait for a writer nor follow a
//!    link, before gix is let to read anything of it. A `gitdir:` line, a `commondir` file, and every line of
//!    `objects/info/alternates` (and the alternates' own lists, to a depth) must name a local place: a network path
//!    (`\\host\share`, `//host/share`, `\\?\UNC\...`, a device path, or on Windows a link leading to one) would make
//!    Windows connect to the host and hand it the user's credentials, and a path under `/net` makes macOS mount a share; such
//!    a repository is "not a repository" (`repo: false`), and nothing at the other end is touched. The files gix reads next
//!    (`HEAD`, `config`, `config.worktree`, `commondir`, `gitdir`, `index` and `sharedindex.*`, `packed-refs`,
//!    `info/exclude`, `info/attributes`, the alternates lists, the files of each `objects/pack`, and the loose files of the
//!    refs `HEAD` leads to, through up to five symbolic refs) must be regular files, not links, within a size (4 KiB for a
//!    `HEAD`, a ref or a pointer, 1 MiB for a config, 64 MiB for an index or `packed-refs`), or the repository is
//!    `repo: false`; an index over its cap is [`GitError::TooBig`]. So a named pipe, a device, or a link to `/dev/zero` where one
//!    of them is expected does not hold the status up. The refs below `refs/replace` are not read (`core.useReplaceRefs`
//!    is off).
//! 9. **A file of the work tree is only read when it is one.** A tracked file that was changed (by its stat) is read to be
//!    compared with its blob only when it is a regular file within [`Limits::compare_bytes`] (and a link where the index
//!    has a link), and its folders' `.gitattributes` are fine to read; otherwise it counts as modified, unopened. A folder
//!    whose `.gitignore` is a named pipe, a device or too big is not entered by the directory walk.
//!
//! Left out: the dirtiness of submodules (point 6), the global ignore file (point 1), rename tracking (a renamed file is a
//! deleted and an added file here) and the ahead/behind or branch state (never read). Bare repositories have no work tree
//! and so no marks (`repo: false`); neither has a repository of a format gix does not read (SHA-256, an unknown
//! extension), which is `repo: false` and not an error.
//!
//! # What is read and bounded
//!
//! - The repository is found by walking up from the folder (`locate::find`), and the walk never crosses a filesystem
//!   boundary (Unix), nor the user's home folder (the home folder itself is the last folder tried), nor the filesystem
//!   root. Which folder to start from is the folder as the page sent it, after its links are resolved, so that the
//!   work tree it is in can be told apart from the folder. Names are compared as the system spells them: precomposed on
//!   macOS, and without case on Windows (`path::prefix_of`).
//! - The status is limited to the folder's own subtree: its index entries are the range of the index with that prefix, the
//!   directory walk and the comparison with the work tree get the folder as a literal pathspec, the comparison with `HEAD`
//!   walks only the folder's subtree of `HEAD`, and nothing outside it is stat-ed (apart from the ignore rules of its
//!   parents).
//! - Three results are merged into one mark per direct child (`marks::Marks`): the index against `HEAD` (`staged`,
//!   `staged::staged`: our own walk of the subtree of `HEAD` against the index entries under the folder, not gix's
//!   `tree_index_status`, which builds an index of the whole tree first, with no cap), the work tree against the index
//!   (`modified`, `conflicted`), and the directory walk (`untracked`, `ignored`). A folder child gets the most severe mark
//!   of anything below it (`conflicted`, then `modified`, then `staged`, then `untracked`); an ignored entry below it never
//!   marks it, and it is `ignored` only when it is itself ignored. A folder that is untracked as a whole, or a nested
//!   repository, is `untracked`; a staged gitlink is `staged`.
//! - The work is bounded by [`Limits`]: a time limit and a cap on the entries walked (the index entries of the folder, the
//!   directories and entries the directory walk saw, the entries of the trees of `HEAD` walked or looked through, and the
//!   changes found). A watchdog thread sets gix's interrupt flag when the time is up, and the app's exit flag does the same,
//!   so a status stops at the next entry, not at the end. The walk of the trees of `HEAD` also bounds its depth (256), the
//!   length of a path (64 KiB) and the bytes of the trees it holds open (64 MiB), and each object it reads is held to gix's
//!   16 MiB limit for a repository of reduced trust. Some things cannot be interrupted once started, and are bounded by
//!   size instead: reading one file's content (skipped for files over [`Limits::compare_bytes`], which show as `modified`
//!   when their stat changed), decoding the index (at most 64 MiB), and reading one object (at most 16 MiB). A repository
//!   that is over the cap is [`GitError::TooBig`], and one that took the time is [`GitError::TimedOut`].
//! - The decoded index of a repository is kept for the next status of it (`cache`: four repositories at most, each for an
//!   index file of a given length and time that is not younger than two seconds), so that a flat view of many folders of one
//!   repository decodes it once.
//! - Anything that still holds a read up (a file that was swapped for a pipe after it was looked at, a loose object or a
//!   pack that is one) is not a bound of this module: the caller answers for the request at its deadline whether or not this
//!   function returns (see `columns_git`).
//! - Errors carry no path ([`GitError`]); the details go to the log at debug level only.

mod cache;
mod compare;
mod files;
mod inspect;
mod locality;
mod locate;
mod marks;
mod open;
mod path;
mod scan;
mod staged;
mod visit;
mod watch;

#[cfg(test)]
mod compare_tests;
#[cfg(test)]
mod fixture;
#[cfg(test)]
mod hostile_tests;
#[cfg(test)]
mod limit_tests;
#[cfg(test)]
mod security_tests;
#[cfg(test)]
mod tests;

use std::fmt;
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::model::GitStatus;

/// The largest file whose content is read to tell a changed stat from a changed file; a larger file with a changed stat
/// counts as modified without being read, so that the time limit is not at the mercy of one long read.
pub const MAX_COMPARE_BYTES: u64 = 64 * 1024 * 1024;

/// The time limit of a status.
const TIME_LIMIT: Duration = Duration::from_secs(5);

/// The cap on the entries one status walks.
const ENTRY_LIMIT: usize = 200_000;

/// What bounds one status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// How long a status may take.
    pub time: Duration,
    /// How many entries it may walk (see the module docs).
    pub entries: usize,
    /// The largest file whose content it reads to compare it (see [`MAX_COMPARE_BYTES`]).
    pub compare_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            time: TIME_LIMIT,
            entries: ENTRY_LIMIT,
            compare_bytes: MAX_COMPARE_BYTES,
        }
    }
}

/// Why a status could not be given. None of the texts holds a path.
#[derive(Debug)]
pub enum GitError {
    /// The cap on entries (or on the size of the index, or of what is walked) was reached: the repository is too big to read
    /// here.
    TooBig,
    /// The time limit was reached: the repository took too long to read.
    TimedOut,
    /// The app is exiting.
    Cancelled,
    /// The folder itself could not be read (it is gone, or there is no permission, or its path cannot be resolved).
    Folder(io::ErrorKind),
    /// The repository is there but cannot be read (damaged files). A repository of a format that is not read is not an error:
    /// it is `repo: false`.
    Repository,
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooBig => "the repository is too big to read here",
            Self::TimedOut => "the repository took too long to read",
            Self::Cancelled => "the read was cancelled",
            Self::Folder(io::ErrorKind::NotFound) => "the folder no longer exists",
            Self::Folder(io::ErrorKind::PermissionDenied) => "the folder cannot be read: no permission",
            Self::Folder(_) => "the folder cannot be read",
            Self::Repository => "the repository cannot be read",
        })
    }
}

impl std::error::Error for GitError {}

/// The git status of the direct children of `folder`, or `repo: false` when the folder is not in the work tree of a
/// repository. `cancel` is the app's exit flag. Blocks: run it on a blocking thread.
pub fn status(folder: &Path, limits: Limits, cancel: &AtomicBool) -> Result<GitStatus, GitError> {
    scan::status(folder, limits, cancel)
}
