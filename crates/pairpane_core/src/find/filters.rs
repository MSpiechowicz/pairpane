//! What an entry must be to count: its name, size, date and attributes.

use std::fs::Metadata;
use std::io;

use regex::{Regex, RegexBuilder};

use crate::listing::{modified_millis, platform};
use crate::model::{FindCriteria, Tri};
use crate::view::refusal;

use super::mask::Mask;
use super::{NEST_LIMIT, REGEX_LIMIT};

/// What is known of an entry for the tests below: read from the disk (`Facts::of`) or from an archive's record.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Facts {
    /// A regular file: not a folder, a link or a device.
    pub(crate) file: bool,
    pub(crate) size: u64,
    pub(crate) modified: Option<i64>,
    pub(crate) hidden: bool,
    pub(crate) readonly: bool,
    pub(crate) executable: bool,
}

impl Facts {
    /// The facts of the entry `name` from its own metadata (a link is looked at, not followed).
    pub(crate) fn of(name: &str, metadata: &Metadata) -> Self {
        let file = metadata.is_file();

        Self {
            file,
            size: if file { metadata.len() } else { 0 },
            modified: modified_millis(metadata),
            hidden: platform::is_hidden(name, Some(metadata)),
            readonly: metadata.permissions().readonly(),
            executable: file && platform::mode(metadata).is_some_and(|mode| mode & 0o111 != 0),
        }
    }
}

/// The test of an entry's name: masks or a regular expression, on the name or on the path below the folder.
pub(crate) struct Names {
    test: Test,
    path: bool,
}

enum Test {
    Anything,
    Mask(Mask),
    Regex(Regex),
}

impl Names {
    /// The test of `criteria`. A blank `names` fits every name. A pattern that is not valid or is too big to run safely
    /// is refused with a fixed text.
    pub(crate) fn new(criteria: &FindCriteria) -> io::Result<Self> {
        let text = criteria.names.trim();
        let test = if text.is_empty() {
            Test::Anything
        } else if criteria.name_regex {
            Test::Regex(regex(text, criteria.name_case)?)
        } else {
            Test::Mask(Mask::new(text, criteria.name_case))
        };

        Ok(Self {
            test,
            path: criteria.name_path,
        })
    }

    /// Whether the entry called `name`, at `path` below the folder, fits.
    pub(crate) fn fits(&self, name: &str, path: &str) -> bool {
        let shown = if self.path { path } else { name };

        match &self.test {
            Test::Anything => true,
            Test::Mask(mask) => mask.fits(shown),
            Test::Regex(regex) => regex.is_match(shown),
        }
    }
}

/// A regular expression within the limits of a search: linear time (the engine has no backtracking), a bounded
/// program and cache, and a bounded nesting.
fn regex(pattern: &str, case_sensitive: bool) -> io::Result<Regex> {
    RegexBuilder::new(pattern)
        .case_insensitive(!case_sensitive)
        .size_limit(REGEX_LIMIT)
        .dfa_size_limit(REGEX_LIMIT)
        .nest_limit(NEST_LIMIT)
        .build()
        .map_err(|error| match error {
            regex::Error::CompiledTooBig(_) => refusal(io::ErrorKind::InvalidInput, "the name pattern is too complex"),
            _ => refusal(
                io::ErrorKind::InvalidInput,
                "the name pattern is not a valid regular expression",
            ),
        })
}

/// The tests of size, date and attributes.
pub(crate) struct Filters {
    min_size: Option<u64>,
    max_size: Option<u64>,
    from: Option<i64>,
    to: Option<i64>,
    hidden: Tri,
    readonly: Tri,
    executable: Tri,
    /// Only regular files can fit: a size limit or a text or content test says something folders and links cannot.
    files_only: bool,
}

impl Filters {
    pub(crate) fn new(criteria: &FindCriteria) -> Self {
        Self {
            min_size: criteria.min_size,
            max_size: criteria.max_size,
            from: criteria.modified_from,
            to: criteria.modified_to,
            hidden: criteria.hidden,
            readonly: criteria.readonly,
            executable: criteria.executable,
            files_only: criteria.min_size.is_some()
                || criteria.max_size.is_some()
                || criteria.text.is_some()
                || criteria.duplicates,
        }
    }

    /// Whether `facts` pass every test.
    pub(crate) fn fits(&self, facts: &Facts) -> bool {
        (facts.file || !self.files_only)
            && self.min_size.is_none_or(|min| facts.size >= min)
            && self.max_size.is_none_or(|max| facts.size <= max)
            && self
                .from
                .is_none_or(|from| facts.modified.is_some_and(|time| time >= from))
            && self.to.is_none_or(|to| facts.modified.is_some_and(|time| time <= to))
            && agrees(self.hidden, facts.hidden)
            && agrees(self.readonly, facts.readonly)
            && agrees(self.executable, facts.executable)
    }
}

fn agrees(wanted: Tri, has: bool) -> bool {
    match wanted {
        Tri::Any => true,
        Tri::Yes => has,
        Tri::No => !has,
    }
}
