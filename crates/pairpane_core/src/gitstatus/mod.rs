//! The git status column: the state of the direct children of a folder that is inside a git repository, read by `gix`
//! (gitoxide, pure Rust) in a helper process of its own, which is confined to read the repository's own folders and is held
//! to a memory and a time limit. The `git` program is never run, and no shell is.
//!
//! # Trust model: nothing the repository says can run anything, and what it makes gix read is bounded
//!
//! The folders a user browses are untrusted: a download or an unpacked archive can hold a `.git` folder that was made to
//! start a program the moment a tool "just reads its status" (`core.fsmonitor`, a filter driver, a hook, a credential
//! helper, `core.sshCommand`, an external diff, an `include` of a config file elsewhere), or to make the reader wait for ever,
//! spend its memory or read what is not the repository's (a pipe or `/dev/zero` where a file is expected, a link to `/proc`,
//! a rules file that is a gigabyte). Three audits found a new way through gix each, so the defence is in layers, each of which
//! holds when the one above it does not: the parent looks at the repository bluntly before anything reads it (points 8 and
//! 9), gix runs in a process that can read only the folders it was told and can write nothing (the helper, below), and that
//! process is killed at a deadline and held to a memory limit whatever gix does.
//!
//! The git status column is not available on Windows yet (the `columns_git` command answers `unsupported` there and never calls
//! anything here, see the shell's `commands::columns`): the helper there is held by a Job Object's memory limit only and nothing
//! confines what it reads or whether it reaches the network, so a gap in the parent's looks at a repository (points 8 and 9)
//! would be a connection to a host of a hostile repository's choosing, with the user's credentials. The code of this module is
//! built there all the same, so that its unit tests, which need no confinement, run on every system.
//!
//! 1. **The configuration is the repository's own file only** (`open::open_options`). The options start from
//!    `gix::open::Options::isolated()`, whose `Permissions::isolated()` has the config sources `git_binary`, `system`,
//!    `git`, `user` and `env` off, `includes` off (an `include.path` or `includeIf` is never followed, so a config file
//!    elsewhere is never read) and every environment permission denied (`Environment::isolated()`: no `GIT_*`, `HOME`,
//!    `XDG_CONFIG_HOME`, `SSH_*` and so on are read). `git_binary: false` also means gix never starts `git` to find its
//!    installation config. As a result no setting of the user's own git configuration applies either, and the global
//!    ignore file (`core.excludesFile`, `$XDG_CONFIG_HOME/git/ignore`) is not applied: only `.gitignore` files in the work
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
//!    program) are given nothing to start by points 1, 3 and 4, and no code of ours calls them otherwise; and the helper
//!    process may start no program whatever (below).
//! 6. **Submodules are never opened.** The submodule status of the gix status (which opens the submodule's own repository
//!    and runs a status there, with that repository's own trust) is replaced by `compare::NoSubmodules`, which answers "no
//!    change" without reading `.gitmodules` or touching the submodule. A submodule folder therefore has a mark only for
//!    what the superproject sees of it (an untracked or staged gitlink), never for its own state: it is never "dirty".
//!    Nested repositories that are not submodules are walked as the folders they are (`recurse_repositories` is on, and
//!    `classify_untracked_bare_repositories` stays off): with it off the directory walk of gix calls `gix_discover::is_git` on
//!    the `.git` of every untracked folder, which follows a `gitdir:` pointer (a network path, a named pipe, `/dev/zero` as a
//!    `HEAD` or a `commondir`) before anything of it was looked at, and no other probe of the walk is left with these options
//!    (`gix-dir` `walk::classify`). A `.git` of a folder is pruned and never read, so a nested repository is `untracked`
//!    through its files, as one folder. The walk does not enter what the index holds as a gitlink.
//! 7. **Read only.** Nothing is written: the index is not refreshed, no lock file is made, nothing is created in `.git`.
//!    A `.git` file (a linked worktree or a submodule) that points elsewhere is read like a `.git` folder; the work tree is
//!    the folder that holds the `.git` file, and it must contain the folder asked for, or the folder is reported as not in
//!    a repository (a `core.worktree` pointing at another checkout is ignored anyway by point 3).
//! 8. **What the repository names is looked at before it is read, bluntly** (`locate`, `inspect`, `objects`, `locality`, `links`,
//!    `files`, `layout`; in the parent, before the helper is started). The repository is found by a walk of our own, with `lstat` and
//!    small reads that neither wait for a writer nor follow a link, and what is read is read as gix reads it: a `.git` file
//!    with gix's own `gitdir:` parser, a `commondir` with its trailing white space cut off and nothing else, the lists of
//!    alternates with gix's parser (every relative entry of every list joined to the objects folder of the repository
//!    itself), a `HEAD` with gix's own parser of a loose reference (`locate::is_head`, which is what decides that a folder is a
//!    git folder at all), and a `.git` folder with both a `commondir` and a `gitdir` only when that file names this very
//!    `.git`. Then the rules are these, and each failure is `repo: false` (an index or rules over their size is
//!    [`GitError::TooBig`]):
//!    - **Physical equals lexical.** Every place a pointer names (the `gitdir:` of a `.git` file, a `commondir`, each entry of
//!      the alternates, but for what follows) and every folder of the [`Layout`](layout::Layout) must be a path with no link
//!      anywhere on it (`links::is_physical`: `resolve(path) == normalise(path)`), so that what a program that takes the `..` out
//!      by reading the path (gix does) means is what the system opens. The exception is an alternate: `git clone --shared` writes
//!      the path of its source as it was named, which may go through a linked home or data folder, so an alternate is the
//!      folder that its path leads to (`locality::alternate_target`: `links::resolve`, which checks every hop for locality and
//!      for an automount), and a link on it is fine as long as no `..` comes straight after one (`links::is_followable`). `..` is allowed where it follows real folders only (a submodule's
//!      `gitdir: ../.git/modules/x`). A `.git` that is a link to a folder is followed, and what it leads to must pass that and
//!      the locality checks. A place that is no local one (a network or device path, an automounted folder, `\\host\share`,
//!      `/net`) is refused, hop by hop (`links::resolve`: `lstat` and `readlink`, at most 40 links, on every system), and so
//!      is a magic link (`/proc/self/fd/N`: `pipe:[..]`, `socket:[..]`, `anon_inode:..`, a deleted file) that the walk cannot
//!      follow and the system can. `Layout::is_opened`, which says whether the repository gix opened is the one that was
//!      looked at, compares the places as the system resolves them.
//!    - **No links among git's own files.** `HEAD`, `info/exclude`, `info/attributes`, `config`, `config.worktree`,
//!      `commondir`, `gitdir`, the `index` and `sharedindex.*`, `packed-refs`, the loose files of the refs `HEAD` leads to
//!      (up to five symbolic refs), the alternates' lists and the files of each `objects/pack` must be regular files, and
//!      `refs`, `info`, `objects`, `objects/info`, `objects/pack` and each present `objects/xx`, of every object folder, real
//!      folders: a link or a reparse point is `repo: false`. So is a loose object that is a link, or anything in an `objects/xx`
//!      that is not a regular file (a link to `\\host\share\x` makes the system connect to the host): each present `objects/xx` of
//!      every object folder is listed, `DirEntry::file_type` of every entry (and the reparse attribute, on Windows) says a plain
//!      file, and the loose objects listed add up to 65,536 at most, past which the repository is [`GitError::TooBig`]
//!      (`objects`). An incremental multi-pack index (`objects/pack/multi-pack-index.d`) and `objects/info/commit-graphs` may be
//!      real folders, and their entries are looked at as the files of `pack` are. This is a deliberate trade, which reverts the follow of a link to
//!      a local regular file that an earlier version had (git itself follows one, and a repository whose `HEAD` is a link, or
//!      whose `info/exclude` is shared by a link, has no status here).
//!    - **Whose, and how broad.** The `.git` that was found, the git folder and the common folder must be owned by the user who
//!      reads them, as git's own check of the owner has it (`locate::owned`, the effective user on Unix; a repository somebody
//!      else planted in a folder above the one browsed is `repo: false`; not checked on Windows, which has no column). A folder
//!      the helper is to be given to read that is the root or a drive root, the home folder or a folder above it, or on a file
//!      system of the kernel's own (`/proc`, `/sys`: `files::on_pseudo_path`, by `statfs`) is refused too (`layout::is_too_broad`:
//!      a work tree that is the home folder itself has no status here), and of an alternate only its `info`, `pack` and loose
//!      folders that were looked at are given, never the folder of the store itself, so that an alternate that names `/tmp` or
//!      a folder of the user's gives the helper nothing but what a store holds.
//!    - **Real sizes.** What is looked at is the opened file (`files::read`, `files::seen`): `lstat` first (nothing that is not
//!      a regular file is opened), then an open that does not wait and does not follow, then `fstat` of what was opened, and on
//!      Linux a file of procfs, sysfs, debugfs, cgroup or tracefs is refused (it says it is 0 bytes and reads without an end).
//!      Within their caps (4 KiB for a `HEAD`, a ref or a pointer, 1 MiB for a config, 256 KiB for `info/exclude` and
//!      `info/attributes`, 64 MiB for an index or `packed-refs`) the small ones are read whole, one byte over the cap
//!      included.
//! 9. **One budget for every rules source** (`budget`). gix reads an ignore or attributes file for every folder it goes
//!    through and holds all their patterns in memory. Every such source is counted before gix reads it, in one budget of 8
//!    MiB for a status, a single source being 256 KiB at most: the `.gitignore` and `.gitattributes` of the work tree on the
//!    way from its root to the folder, in each folder the directory walk enters and in the folder of each file that is
//!    compared, the repository's own `info/exclude` and `info/attributes`, and the blob of an index entry that stands in for a
//!    rules file that is not in the work tree (deleted, or skip-worktree), sized by its header (`find_header`). A folder
//!    whose rules are over their size, or a pipe, is not entered by the walk and is `modified`, which is the safe thing to
//!    say of what was not looked at; one on the way to the folder asked for is [`GitError::TooBig`] (a pipe there is
//!    `repo: false`); a file whose rules cannot be read counts as `modified` without being opened; and past the total the
//!    status is [`GitError::TooBig`]. A work-tree file that is changed is read only when it is a regular file within
//!    [`Limits::compare_bytes`] (and a link where the index has a link).
//!
//! # The helper
//!
//! The parent (`helper`, called by the `columns_git` command) does the looks of points 8 and 9 that need no gix, and so
//! settles `repo: false` and the folders the helper may read, without starting anything. Then it starts the app's own
//! executable with [`CHILD_ARG`] through `process::spawn`, as the thumbnail helper is started (the cleared environment, one
//! pipe each way, no standard error), gives it the request (`request`: the folder, the limits and the layout: work tree, git
//! folder, common folder, alternates' object folders, each with the device and inode it had when it was looked at) and reads at
//! most 8 MiB of its answer: a status or the kind of an error, never a path, and checked as it is decoded (every name one plain
//! name, at most one entry for a name and no more marks than `Limits::entries`, nothing after the end). The child (`child`)
//! starts a session of its own (it has no controlling terminal, so nothing can be pushed into one with `TIOCSTI`), closes what
//! it inherited, reads the request, confines itself before gix opens anything (and a folder that is not the one that was looked
//! at is a failure to confine), opens exactly the layout (and checks it again with `Layout::is_opened`) and answers.
//!
//! - **Linux**: it dies with the app (`PR_SET_PDEATHSIG`), gains no privileges (`PR_SET_NO_NEW_PRIVS`), has no core dump, 256
//!   open files, the time limit and a second of processor time (`RLIMIT_CPU`, with a hard limit a second above, so that the soft
//!   limit's `SIGXCPU` is what ends it, which the parent reads as a time out) and a gigabyte of data (`RLIMIT_DATA`: the heap
//!   and the private writable mappings; not the address space, which would refuse to map a pack as big as the repository is).
//!   Landlock lets it read, and only read, below the layout's folders (each opened and `fstat`ed against the device and inode
//!   of the request), and handles every network right and the abstract-socket and signal scopes with none allowed, so `/proc`,
//!   the rest of the file system and the user's other files are out of its reach. On a kernel that has Landlock, a ruleset that
//!   cannot be made or is not enforced is a failure: the helper exits without reading (only a kernel without it runs on the
//!   filter alone, which the parent logs once). A seccomp filter on every thread, which is required, refuses sockets, io_uring,
//!   programs and processes (threads stay), `ptrace` and the like, every call that writes or changes a file (and `flock`), an
//!   open that would write, every `ioctl` but two questions (`FIONREAD`, `TCGETS`) and every call numbered above the highest that
//!   was reviewed (`Filter::ReadOnly`), so where the kernel has no Landlock it is still a read-only file system. See
//!   `view/heic/sandbox.rs` for what is stopped by what, and on which kernel.
//! - **macOS**: the same descriptors and limits, and `sandbox_init` with `(deny default)` and the read of the layout's folders
//!   (`seatbelt.rs`; each is checked against the device and inode of the request first, as the profile names paths). It does
//!   not die with the app (macOS has no such signal), and nothing watches for it: it ends when it writes its answer to a pipe
//!   that nobody reads, and at the latest at its processor-time limit, and one that is blocked in a read that does not return
//!   uses no processor time and stays until the read does (the parent's deadline kills it while the app runs). Written and
//!   unit-tested for its profile on every system, but entered only on macOS, where the CI step "Git status helper, real
//!   executable" is what shows that it works.
//! - **Windows**: not available (see the top of this page). Were it started, the parent would put the child in a Job Object that
//!   has a gigabyte of memory, one active process and kill-on-close (it never outlives the app), and there is no AppContainer (it
//!   would need a change of the access lists of the user's own repositories, which this must not do): what it may read is not
//!   bounded, and the parent's checks (point 8) would be all that a repository is held to, with a time of check and a time of
//!   use between them that nothing closes (a path swapped for a network one in between), which is what keeps the column off.
//! - **The deadline**: the parent waits for the time limit and half a second, then kills the child and answers
//!   [`GitError::TimedOut`]; the exit flag of the app kills it too. So a read of a pipe, a chain of deltas that spends the
//!   processor, or a mount that does not answer costs at most that, and no thread of the app is ever left behind. A child that
//!   ends with no answer (a crash, the memory limit) is [`GitError::Repository`].
//! - **No cache.** The decoded index is not kept between statuses: the process is made for one. A flat view over many
//!   folders of one repository decodes the index once for each folder, and starts a helper for each, two at a time; the
//!   page asks for a folder at a time, and this does not batch them.
//!
//! # Known residuals
//!
//! - A time-of-check/time-of-use race: the files and places of a repository are looked at in the parent, and opened a moment
//!   later by gix in the helper. Someone who swaps one for a pipe or a link in between can hold a read up, which only the
//!   deadline answers for (and the helper's confinement bounds what it may read: a folder that is swapped is not the one
//!   that was looked at, and the helper does not go on); that needs write access to the folders of the repository. On Windows
//!   nothing bounds what is read, and the race is a way to a network path: it is the reason the column is not there.
//! - One read of one object cannot be interrupted, and gix has no hook for it: a long chain of deltas (or one made to repeat
//!   its copies) spends the processor of the helper for as long as it takes, bounded in size (16 MiB) and in time by the
//!   deadline and the limit on processor time, which kill the helper: the cost is one core for that long, for two helpers at
//!   most.
//! - On Windows a mapped network drive (`Z:`) is a drive letter like any other, and is taken for a local place: nothing in the
//!   path says otherwise. Nothing there confines what the helper reads either (see the top of this page).
//! - A repository of the reftable format is `repo: false`: gix 0.91 cannot read it.
//! - The global ignore file (`core.excludesFile`, `$XDG_CONFIG_HOME/git/ignore`) is not applied (point 1).
//! - A nested repository (a folder with a `.git` of its own that is no submodule) is walked as the folder it is, and one whose
//!   own `.gitignore` is `*` shows `!` for what it holds that the rules of the outer repository do not name, which is the
//!   marks of a folder and not of a repository.
//! - `objects`, `refs` or `info` that is a link, in the git folder or in the common folder, is `repo: false` (point 8), and so
//!   is a work tree that is the user's home folder. The page is told why for a repository that is refused for what it names
//!   (`GitStatus::note`, a fixed text with no path); a folder in no repository has none.
//! - The parent's own look at the files of a repository (`lstat`, small non-blocking reads) can still wait on a mount that does
//!   not answer, as every other call of the app on such a mount can, holding its thread and one of the two places for a status.
//!
//! Left out: the dirtiness of submodules (point 6), the global ignore file (point 1), rename tracking (a renamed file is a
//! deleted and an added file here) and the ahead/behind or branch state (never read). Bare repositories have no work tree
//! and so no marks (`repo: false`); neither has a repository of a format gix does not read (SHA-256, an unknown
//! extension, reftable), which is `repo: false` and not an error. A sparse index (a folder out of the work tree held as one
//! entry of the index) is read: the folder is as `HEAD` has it, and is `staged` only when the entry's tree is another one.
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
//!   walks only the folder's subtree of `HEAD`, and nothing outside it is stat-ed (apart from the rules of its parents).
//! - Three results are merged into one mark per direct child (`marks::Marks`): the index against `HEAD` (`staged`,
//!   `staged::staged`: our own walk of the subtree of `HEAD` against the index entries under the folder, not gix's
//!   `tree_index_status`, which builds an index of the whole tree first, with no cap), the work tree against the index
//!   (`modified`, `conflicted`), and the directory walk (`untracked`, `ignored`). A folder child gets the most severe mark
//!   of anything below it (`conflicted`, then `modified`, then `staged`, then `untracked`); an ignored entry below it never
//!   marks it, and it is `ignored` only when it is itself ignored. A folder that is untracked as a whole, or a nested
//!   repository, is `untracked`; a staged gitlink is `staged`. A child folder of a folder that is untracked as a whole is
//!   `ignored` when the rules name it or when it holds only ignored files, as in git (`ignored`: a look that is bounded by
//!   the meter, the budget and a cap of its own).
//! - The work is bounded by [`Limits`]: a time limit and a cap on the entries walked, which each of the three steps has for
//!   itself (the index entries of the folder; the entries of the trees of `HEAD` walked or looked through; the directories
//!   and entries the directory walk saw, and the changes found), so that a repository of 100,000 files is not too big for its
//!   root because each file is counted three times. The time limit is the bound of the whole. A watchdog thread of the helper
//!   sets gix's interrupt flag when the time is up, so a status stops at the next entry, not at the end. The walk of the trees
//!   of `HEAD` also bounds its depth (256), the length of a path (64 KiB) and the bytes of the trees it holds open (64 MiB),
//!   and each object it reads is held to gix's 16 MiB limit for a repository of reduced trust. Some things cannot be
//!   interrupted once started, and are bounded by size instead: reading one file's content (skipped for files over
//!   [`Limits::compare_bytes`], which show as `modified` when their stat changed), decoding the index (at most 64 MiB), and
//!   reading one object (at most 16 MiB). A repository that is over the cap is [`GitError::TooBig`], and one that took the
//!   time is [`GitError::TimedOut`].
//! - Errors carry no path ([`GitError`]); the details go to the log at debug level only.

mod budget;
mod child;
mod compare;
mod files;
mod helper;
mod ignored;
mod inspect;
mod layout;
mod links;
mod locality;
mod locate;
mod marks;
mod note;
mod objects;
mod open;
mod path;
mod request;
mod scan;
#[cfg(any(target_os = "macos", test))]
mod seatbelt;
mod staged;
mod visit;
mod watch;

#[cfg(test)]
mod budget_tests;
#[cfg(test)]
mod compare_tests;
#[cfg(all(test, target_os = "linux"))]
mod confine_tests;
#[cfg(test)]
mod files_tests;
#[cfg(test)]
mod fixture;
#[cfg(test)]
mod hostile_tests;
#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod limit_tests;
#[cfg(test)]
mod nested_tests;
#[cfg(test)]
mod security_tests;
#[cfg(test)]
mod sparse_tests;
#[cfg(test)]
mod testing;
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

/// The cap on the entries each step of a status walks (the index range, the trees of `HEAD`, the work tree).
const ENTRY_LIMIT: usize = 200_000;

/// What bounds one status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// How long a status may take.
    pub time: Duration,
    /// How many entries each of its steps may walk (see the module docs).
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

pub use child::{CHILD_ARG, child_main};

/// The git status of the direct children of `folder`, or `repo: false` when the folder is not in the work tree of a
/// repository, or in one that must not be read. `cancel` is the app's exit flag. The repository is read by the helper
/// `program` (the app's own executable, which hands the process to [`child_main`] when it is started with [`CHILD_ARG`]): see
/// the module docs. Blocks, for at most [`Limits::time`] and a short grace: run it on a blocking thread.
pub fn status(folder: &Path, limits: Limits, cancel: &AtomicBool, program: &Path) -> Result<GitStatus, GitError> {
    helper::status(folder, limits, cancel, program)
}

/// The same status as [`status`], read in this process instead of the helper: the process that calls it is the one that is not
/// confined and has no memory or time limit but the time limit of the status. For the tests of what stands around the status
/// (the shell's commands), which cannot start the app's own executable as the helper; the app never calls it.
#[doc(hidden)]
pub fn status_in_process(folder: &Path, limits: Limits, cancel: &AtomicBool) -> Result<GitStatus, GitError> {
    in_process(folder, layout::home(), limits, cancel)
}

/// The request that [`status`] gives the helper for `folder` (the bytes on its standard input), or `None` when the folder is in
/// no repository that is read. For the tests of the real executable as the helper, which start it themselves to look at it
/// while it works; the app never calls it.
#[doc(hidden)]
pub fn helper_request(folder: &Path, limits: Limits) -> Result<Option<Vec<u8>>, GitError> {
    let request = helper::prepare(folder, layout::home(), limits, &AtomicBool::new(false))?;

    request
        .repository()
        .map(|request| request.encode().map_err(|_| GitError::Repository))
        .transpose()
}

/// Like [`status`] for a request that was made by [`helper_request`] (and a repository that was changed since, which is how a
/// test makes a file that was looked at a pipe: the one thing that the look at the files of a repository cannot see coming), with
/// the helper supervised as it is: the deadline, the exit flag, the answer read. For the same tests.
#[doc(hidden)]
pub fn status_for_request(request: &[u8], cancel: &AtomicBool, program: &Path) -> Result<GitStatus, GitError> {
    let request = request::Request::decode(request).map_err(|_| GitError::Repository)?;

    helper::run(program, &request, cancel)
}

/// The result an answer of the helper (the bytes on its standard output) stands for, as the parent reads it. For the same tests.
#[doc(hidden)]
pub fn helper_answer(bytes: &[u8]) -> Result<GitStatus, GitError> {
    request::decode_answer(bytes, ENTRY_LIMIT)
}

/// [`status_in_process`] with the folder the search for a repository must not go above given.
fn in_process(
    folder: &Path,
    ceiling: Option<std::path::PathBuf>,
    limits: Limits,
    cancel: &AtomicBool,
) -> Result<GitStatus, GitError> {
    let request = match helper::prepare(folder, ceiling, limits, cancel)? {
        locate::Found::Repository(request) => request,
        locate::Found::Nowhere => return Ok(scan::not_a_repository()),
        locate::Found::Refused(note) => return Ok(note.status()),
    };
    // The request is what the helper is given, so it is what is read.
    let request = request::Request::decode(&request.encode().map_err(|_| GitError::Repository)?)
        .map_err(|_| GitError::Repository)?;

    scan::read(&request, cancel)
}
