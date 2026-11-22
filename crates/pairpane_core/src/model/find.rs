use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{EntryKind, IpcError};

/// Identifies one search. Monotonic within a run and never reused; a `number` in TypeScript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FindId(#[ts(type = "number")] pub u64);

/// A yes/no/any test of a file's attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Tri {
    /// The attribute is not looked at.
    #[default]
    Any,
    /// Only what has the attribute.
    Yes,
    /// Only what has not.
    No,
}

/// What to look for inside the files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FindText {
    /// The text, or a regular expression when `regex` is set. Not empty; at most 1024 bytes.
    pub pattern: String,
    pub regex: bool,
    pub case_sensitive: bool,
    /// Only whole words (the match must not touch a letter, digit or underscore).
    pub whole_word: bool,
    /// A label the viewer knows (`windows-1250`, `utf-16le`); `null` reads UTF-8 and the UTF-8 and UTF-16 byte order
    /// marks.
    pub encoding: Option<String>,
    /// Read files of any size; without it, files over 64 MiB are skipped.
    pub any_size: bool,
}

/// What a search looks for. Every part that is set narrows the result.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FindCriteria {
    /// `;`-separated wildcard masks (`*` any run, `?` one character), or a regular expression when `name_regex` is set.
    /// Blank means every name.
    pub names: String,
    pub name_regex: bool,
    /// Match against the path below the folder instead of the name alone.
    pub name_path: bool,
    /// Names are compared with their case.
    pub name_case: bool,
    /// Only files that hold this text.
    pub text: Option<FindText>,
    /// Smallest and largest size in bytes (inclusive). A size limit leaves out folders and links.
    #[ts(type = "number | null")]
    pub min_size: Option<u64>,
    #[ts(type = "number | null")]
    pub max_size: Option<u64>,
    /// Last modified from and to, in milliseconds since the Unix epoch (inclusive).
    #[ts(type = "number | null")]
    pub modified_from: Option<i64>,
    #[ts(type = "number | null")]
    pub modified_to: Option<i64>,
    pub hidden: Tri,
    pub readonly: Tri,
    /// Unix only: a regular file with an execute permission bit; refused on Windows unless `any`.
    pub executable: Tri,
    /// Do not look in hidden folders, and leave out hidden files.
    pub skip_hidden: bool,
    /// Leave out what `.gitignore`, `.ignore` and `.git/info/exclude` files ignore.
    pub skip_ignored: bool,
    /// Do not cross into another filesystem.
    pub one_filesystem: bool,
    /// Look at the names inside zip, tar and 7z archives too.
    pub archives: bool,
    /// Find files of the same content instead: same size, then the same hash.
    pub duplicates: bool,
}

/// A search to start: the folder to look in and what to look for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FindQuery {
    /// The folder, absolute.
    pub root: String,
    pub criteria: FindCriteria,
}

/// One result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FindHit {
    /// The path below the search's folder, names joined with `/`. For a hit inside an archive: the archive's path.
    pub path: String,
    /// For a hit inside an archive: the entry's path in it.
    pub inner: Option<String>,
    /// For a link: `other` (a link is never followed).
    pub kind: EntryKind,
    /// A symbolic link.
    pub link: bool,
    /// Size in bytes; 0 for folders.
    #[ts(type = "number")]
    pub size: u64,
    /// Last modification, in milliseconds since the Unix epoch.
    #[ts(type = "number | null")]
    pub modified: Option<i64>,
    /// The number of lines that hold the text; set when the search looks for text.
    pub matches: Option<u32>,
    /// The first of those lines, trimmed and cut to 256 characters.
    pub line: Option<String>,
    /// The number of that line, from 1.
    #[ts(type = "number | null")]
    pub line_number: Option<u64>,
}

/// What a search delivers: any number of `found`, `progress` and `group` events, then exactly one of `done` and
/// `error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum FindEvent {
    /// More results, in no particular order.
    Found { hits: Vec<FindHit> },
    /// What was looked at so far. While duplicates are hashed, `bytes` counts the bytes hashed.
    Progress {
        #[ts(type = "number")]
        folders: u64,
        #[ts(type = "number")]
        files: u64,
        #[ts(type = "number")]
        bytes: u64,
    },
    /// Files of the same content, confirmed (duplicates only): `size` is the size of each.
    Group {
        #[ts(type = "number")]
        size: u64,
        hits: Vec<FindHit>,
    },
    /// The search ended.
    Done {
        /// The results delivered.
        #[ts(type = "number")]
        found: u64,
        /// What could not be looked at: unreadable files and folders, files too big for a text search, damaged archives.
        #[ts(type = "number")]
        skipped: u64,
        /// It stopped at a limit and there is more to find.
        truncated: bool,
        /// It was stopped.
        cancelled: bool,
    },
    /// The search could not run.
    Error { error: IpcError },
}

/// A search kept by name, criteria only (the folder is the one the pane is in when it is loaded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedSearch {
    pub name: String,
    pub criteria: FindCriteria,
}
