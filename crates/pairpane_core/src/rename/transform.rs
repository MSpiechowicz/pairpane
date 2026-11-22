//! What a rename does to a name after the templates: search and replace over the whole name, and the change of case.

use std::borrow::Cow;
use std::ops::Range;

use regex::Regex;

use crate::find::bounded_regex;
use crate::model::{CaseChange, CasePart, RenameRule, RuleError, RuleField};

use super::template::split_name;

/// The apostrophes that do not start a new word of a title.
const APOSTROPHES: [char; 2] = ['\'', '’'];

/// A search and replace over a name: the search compiled within the limits of a search of files, and what replaces it.
pub(super) struct Search {
    regex: Regex,
    replace: String,
    /// The replacement is text as it is, not with `$1` groups.
    literal: bool,
    first_only: bool,
}

impl Search {
    /// The search of `rule`, or `None` when it has none (an empty search finds nothing).
    pub(super) fn new(rule: &RenameRule) -> Result<Option<Self>, RuleError> {
        if rule.search.is_empty() {
            return Ok(None);
        }

        let pattern = if rule.regex {
            Cow::Borrowed(rule.search.as_str())
        } else {
            Cow::Owned(regex::escape(&rule.search))
        };
        let regex = bounded_regex(&pattern, rule.match_case).map_err(|error| RuleError {
            field: RuleField::Search,
            message: match error {
                regex::Error::CompiledTooBig(_) => "the search is too complex".to_owned(),
                _ => "the search is not a valid regular expression".to_owned(),
            },
        })?;

        Ok(Some(Self {
            regex,
            replace: rule.replace.clone(),
            literal: !rule.regex,
            first_only: rule.first_only,
        }))
    }

    /// `name` with the first match replaced, or every one. It stops as soon as the result is over `limit` bytes (what
    /// is returned is then longer than that by the rest of a replacement at most, and cut short of the rest of the
    /// name), so a replacement that is long or matches at every position cannot make a result of any size.
    pub(super) fn apply(&self, name: &str, limit: usize) -> String {
        // The groups of a match are only needed to expand `$` in the replacement; without that, finding the matches
        // alone is faster.
        if self.literal || !self.replace.contains('$') {
            return self.splice(
                name,
                limit,
                self.regex.find_iter(name),
                |found| Some(found.range()),
                |_, out| out.push_str(&self.replace),
            );
        }

        self.splice(
            name,
            limit,
            self.regex.captures_iter(name),
            |captures| captures.get(0).map(|found| found.range()),
            |captures, out| captures.expand(&self.replace, out),
        )
    }

    /// `name` with the span of each of `matches` (`span`) replaced by what `write` appends, as `apply` says.
    fn splice<M>(
        &self,
        name: &str,
        limit: usize,
        matches: impl Iterator<Item = M>,
        span: impl Fn(&M) -> Option<Range<usize>>,
        write: impl Fn(&M, &mut String),
    ) -> String {
        let mut out = String::with_capacity(name.len());
        let mut tail = 0;

        for found in matches {
            let Some(span) = span(&found) else {
                break;
            };

            out.push_str(&name[tail..span.start]);
            write(&found, &mut out);
            tail = span.end;

            if self.first_only || out.len() > limit {
                break;
            }
        }
        if out.len() <= limit {
            out.push_str(&name[tail..]);
        }

        out
    }
}

/// `text` in the case `change` says.
pub(super) fn change_case(text: &str, change: CaseChange) -> String {
    match change {
        CaseChange::Keep => text.to_owned(),
        CaseChange::Lower => text.to_lowercase(),
        CaseChange::Upper => text.to_uppercase(),
        CaseChange::Title => title(text),
    }
}

/// The first letter after anything that is not a letter or a digit (an apostrophe excepted) in uppercase, the rest in
/// lowercase.
fn title(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut starts_word = true;

    for char in text.chars() {
        if char.is_alphanumeric() {
            if starts_word {
                out.extend(char.to_uppercase());
            } else {
                out.extend(char.to_lowercase());
            }
            starts_word = false;
        } else {
            out.push(char);
            starts_word = starts_word || !APOSTROPHES.contains(&char);
        }
    }

    out
}

/// The name `name` of an item (`folder`: a folder) with the case of `part` of it changed.
pub(super) fn apply_case(name: &str, folder: bool, change: CaseChange, part: CasePart) -> String {
    if change == CaseChange::Keep {
        return name.to_owned();
    }

    let (stem, ext) = split_name(name, folder);

    match part {
        CasePart::Both => change_case(name, change),
        CasePart::Name => join(&change_case(stem, change), ext),
        CasePart::Ext => join(stem, &change_case(ext, change)),
    }
}

fn join(stem: &str, ext: &str) -> String {
    if ext.is_empty() {
        stem.to_owned()
    } else {
        format!("{stem}.{ext}")
    }
}
