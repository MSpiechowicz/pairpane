//! Finding and opening the repository a folder is in, with the most restrictive trust (see the module docs of
//! [`super`] for why that keeps anything from running).

use std::path::{Path, PathBuf};

use gix::config::file::Metadata;
use gix::config::source::Kind;
use gix::discover::upwards::{Error as Upwards, Options as Search, TrustPolicy};
use gix::open::Options;
use gix::sec::Trust;

use super::GitError;

/// The options a repository is opened with: its own config file only, nothing from the environment or the user's files,
/// no `include`, reduced trust whoever owns it, and no section of its config trusted for any value.
pub(super) fn open_options() -> Options {
    Options::isolated()
        .with_enforced_trust(Trust::Reduced)
        .filter_config_section(trusts_no_repository_section)
        .lossy_config(true)
}

/// gix reads every value that can start a program (filter drivers, diff drivers and so on) through a filter on the
/// config sections, and this one lets through the sections that do not come from the repository. With the isolated
/// permissions there are none of those apart from the overrides the program itself gives (it gives none), so no value of
/// that kind exists, whatever the repository's `.git/config` says.
pub(super) fn trusts_no_repository_section(section: &Metadata) -> bool {
    section.source.kind() != Kind::Repository
}

/// The search for a repository from a folder: it assumes reduced trust and does not check who owns what, never crosses
/// a filesystem boundary, and ends at `ceiling` (the user's home folder, which is the last folder looked in) when the
/// folder is inside it.
fn search<'a>(ceiling: Option<PathBuf>) -> Search<'a> {
    Search {
        trust: TrustPolicy::Assume(Trust::Reduced),
        ceiling_dirs: ceiling.into_iter().collect(),
        // Outside the home folder, or at it, there is no ceiling to match; that is not an error.
        match_ceiling_dir_or_error: false,
        cross_fs: false,
        ..Search::default()
    }
}

/// The repository that has `folder` (a real path, its links resolved) in its work tree, or `None` when there is none.
/// `ceiling` is a real path too.
pub(super) fn find(folder: &Path, ceiling: Option<PathBuf>) -> Result<Option<gix::Repository>, GitError> {
    match gix::discover_opts(folder, search(ceiling), open_options()) {
        Ok(repository) => Ok(Some(repository)),
        Err(error) if is_not_a_repository(&error) => Ok(None),
        Err(error) => {
            tracing::debug!(%error, "git status: the repository cannot be opened");

            Err(GitError::Repository)
        }
    }
}

/// A search that ends without finding a repository (none above the folder, the ceiling or the filesystem boundary was
/// reached) is an answer, not an error.
fn is_not_a_repository(error: &gix::Error) -> bool {
    matches!(
        error.downcast_any_ref::<Upwards>(),
        Some(
            Upwards::NoGitRepository { .. }
                | Upwards::NoGitRepositoryWithinCeiling { .. }
                | Upwards::NoGitRepositoryWithinFs { .. }
        )
    )
}
