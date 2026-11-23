//! Finding and opening the repository a folder is in, with the most restrictive trust (see the module docs of
//! [`super`] for why that keeps anything from running).

use std::path::Path;

use gix::config::file::Metadata;
use gix::config::source::Kind;
use gix::open::Options;
use gix::sec::Trust;

use super::GitError;

/// The options a repository is opened with: its own config file only, nothing from the environment or the user's files,
/// no `include`, reduced trust whoever owns it, and no section of its config trusted for any value. Replacement objects
/// are off (`core.useReplaceRefs`, from the program and not from the repository): a status looks at what the repository
/// has, and the refs below `refs/replace` would be listed (and so read) on opening otherwise.
pub(super) fn open_options() -> Options {
    Options::isolated()
        .with_enforced_trust(Trust::Reduced)
        .config_overrides(["core.useReplaceRefs=false"])
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

/// Opens the repository whose `.git` is at `dot_git` (found and looked at by [`super::locate`]), or `None` when it is
/// not one gix reads: not a repository after all, or one of a format this build does not read (a SHA-256 repository, an
/// extension it does not know), which is an answer and not a failure that would be asked again.
pub(super) fn open(dot_git: &Path) -> Result<Option<gix::Repository>, GitError> {
    match gix::open_opts(dot_git, open_options().open_path_as_is(true)) {
        Ok(repository) => Ok(Some(repository)),
        Err(error) if is_not_readable_here(&error) => {
            tracing::debug!(%error, "git status: the repository is not one to read here");

            Ok(None)
        }
        Err(error) => {
            tracing::debug!(%error, "git status: the repository cannot be opened");

            Err(GitError::Repository)
        }
    }
}

/// An error that says what is there is not a repository of a kind that is read, and not that the read went wrong.
fn is_not_readable_here(error: &gix::Error) -> bool {
    error.is_not_found() || error.is_unsupported() || error.is_validation()
}
