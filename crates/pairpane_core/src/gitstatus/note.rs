//! Why a repository that was found is not read, when the page is to be told (`GitStatus::note`): a fixed text for each reason,
//! which holds no path. A folder that is in no repository has no note, and neither has one the helper could not read: that is an
//! error of its own kind (see [`super::GitError`]).

use crate::model::GitStatus;

/// What the parent's look at a repository refused it for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Note {
    /// Its files lead elsewhere: a link or a reparse point where a file or a folder of git's own is expected, a pointer to a
    /// place that is not a local one, or an object folder that holds anything but plain files.
    Elsewhere,
    /// The repository is owned by another user.
    Owner,
    /// A folder that it names (its own, or an alternate's) is too broad to give the helper to read: the root, the home folder or
    /// a folder above it, or a file system of the kernel's own.
    Broad,
}

impl Note {
    /// The text the page shows.
    pub(super) fn text(self) -> &'static str {
        match self {
            Self::Elsewhere => "This repository links git's own files elsewhere, which the app does not follow",
            Self::Owner => "This repository belongs to another user, so the app does not read it",
            Self::Broad => "This repository names a folder that is too broad to read, such as the home folder",
        }
    }

    /// The status of a folder in a repository that is refused for this.
    pub(super) fn status(self) -> GitStatus {
        GitStatus {
            repo: false,
            marks: Vec::new(),
            note: Some(self.text().to_owned()),
        }
    }
}
