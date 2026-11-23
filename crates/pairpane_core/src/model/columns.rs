use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The size of a picture in pixels, as its header says: for a JPEG after the turn its EXIF orientation asks for, for a HEIC
/// or AVIF after the crop and the turn of its properties.
/// Never zero on either side: a header that says so has no dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

/// What the git status column shows for one entry of a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum GitMark {
    /// A file whose content differs from the index, or a folder with anything below it that changed (staged, modified,
    /// untracked or conflicted; a folder with only ignored files below it has no mark).
    Modified,
    /// A file whose index entry differs from `HEAD` (new files that were added included), with no further change in the
    /// work tree. A file that is both staged and modified is `Modified`.
    Staged,
    /// A file that is in no commit and not in the index, and is not ignored.
    Untracked,
    /// A file or folder that the ignore rules of the repository name.
    Ignored,
    /// A file with an unresolved merge conflict.
    Conflicted,
}

/// One entry of the folder that has a mark. Entries that are clean, or tracked and unchanged, are left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitEntry {
    /// The name of the entry in the folder (one plain name, never a path).
    pub name: String,
    pub mark: GitMark,
}

/// The git status of the direct children of a folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitStatus {
    /// Whether the folder is inside the work tree of a git repository. `false`: there are no marks to show.
    pub repo: bool,
    /// The marked entries, sorted by name.
    pub marks: Vec<GitEntry>,
    /// Why a repository is not read, when `repo` is `false` for that reason and not because the folder is in none: a fixed text
    /// that holds no path, for the page to show where the column is blank. Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
}
