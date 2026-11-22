//! Multi-rename: the rule that turns the names of many items of one folder into new ones, and the preview of what it
//! would do.
//!
//! # API
//!
//! 1. [`compile`] checks a [`RenameRule`] and makes the [`Compiled`] rule: its templates (see `template`), its search and
//!    replace (a regular expression only within the limits of a search of files, see [`crate::find`]) and its case change,
//!    each field of at most [`crate::find::MAX_PATTERN_BYTES`] bytes. A fault is a [`RuleError`] that names the field.
//! 2. [`preview_in`] gives the new name of each item of a folder, and what is wrong with it (see [`RenameProblem`]).
//!    The preview looks at the folder (a name is taken, an item is gone); the job that renames (see
//!    [`crate::ops::Task::Rename`]) checks again when it runs.
//!
//! # Rules
//!
//! - The new name is the name template, a dot and the extension template (the dot only when the extension is not empty),
//!   then the search and replace over that whole name, then the case change of the name, the extension or both. The
//!   split between name and extension is the one the panes show, so a folder and a name that starts or ends with a dot
//!   have no extension.
//! - The counter of the item at position `i` of the list is `start + i * step`, padded with zeros to `digits`.
//! - Nothing is ever replaced: a new name that an item of the folder has is a problem, unless that item is renamed away
//!   too, so a swap, a cycle and a chain are fine (see `check`).
//! - A name over 1 KiB is cut to that (it is too long anyway), so a rule cannot make a preview of any size: a template
//!   or a replacement stops as soon as the name is over it, and a name that is over it before the search is not searched.

mod check;
mod template;
mod transform;

#[cfg(test)]
mod bomb_tests;
#[cfg(test)]
mod check_tests;
#[cfg(test)]
mod preview_tests;
#[cfg(test)]
mod rule_tests;
#[cfg(test)]
mod template_tests;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod transform_tests;

use std::fs;
use std::io;
use std::path::Path;

use jiff::tz::TimeZone;

use crate::find::MAX_PATTERN_BYTES;
use crate::listing::{modified_millis, platform};
use crate::model::{
    CaseChange, CasePart, RenameCounter, RenamePreview, RenameProblem, RenameRow, RenameRule, RuleError, RuleField,
};

use self::template::{Context, Local, Template, split_name};
use self::transform::{Search, apply_case};

/// The biggest value the counter starts at.
pub const MAX_COUNTER_START: u32 = 999_999_999;

/// The biggest step of the counter.
pub const MAX_COUNTER_STEP: u32 = 1_000;

/// The most digits of the counter.
pub const MAX_COUNTER_DIGITS: u8 = 10;

/// The longest new name kept, in bytes: far over what any system takes.
const MAX_NAME_BYTES: usize = 1_024;

/// A [`RenameRule`] that was checked and prepared to be applied to many names.
pub struct Compiled {
    name: Template,
    ext: Template,
    counter: RenameCounter,
    search: Option<Search>,
    case: CaseChange,
    case_part: CasePart,
    /// The time zone the dates are shown in; `None` when no template writes one.
    zone: Option<TimeZone>,
}

/// What the rule is applied to: an item, and where it is in the list.
struct Item<'a> {
    name: &'a str,
    folder: bool,
    /// Its position in the list, which the counter counts by.
    index: usize,
    /// The last modification, in milliseconds since the Unix epoch.
    modified: Option<i64>,
}

/// Checks `rule` and prepares it.
///
/// # Errors
///
/// A [`RuleError`] for the first field that cannot be used: a template that is too long or has a placeholder that does not
/// exist, a counter out of range, a search that is too long, not a valid regular expression or too complex to run, and
/// a replacement that is too long. The messages hold no path.
pub fn compile(rule: &RenameRule) -> Result<Compiled, RuleError> {
    let fields = [
        (RuleField::Name, &rule.name),
        (RuleField::Ext, &rule.ext),
        (RuleField::Search, &rule.search),
        (RuleField::Replace, &rule.replace),
    ];
    for (field, text) in fields {
        if text.len() > MAX_PATTERN_BYTES {
            return Err(fault(field, "this is too long"));
        }
    }

    let counter = &rule.counter;
    let in_range = counter.start <= MAX_COUNTER_START
        && (1..=MAX_COUNTER_STEP).contains(&counter.step)
        && (1..=MAX_COUNTER_DIGITS).contains(&counter.digits);
    if !in_range {
        return Err(fault(
            RuleField::Counter,
            "the counter starts at 0 to 999999999, steps by 1 to 1000 and has 1 to 10 digits",
        ));
    }

    let name = Template::parse(&rule.name).map_err(|message| fault(RuleField::Name, &message))?;
    let ext = Template::parse(&rule.ext).map_err(|message| fault(RuleField::Ext, &message))?;
    let search = Search::new(rule)?;
    let zone = (name.uses_date() || ext.uses_date()).then(TimeZone::system);

    Ok(Compiled {
        name,
        ext,
        counter: counter.clone(),
        search,
        case: rule.case,
        case_part: rule.case_part,
        zone,
    })
}

fn fault(field: RuleField, message: &str) -> RuleError {
    RuleError {
        field,
        message: message.to_owned(),
    }
}

impl Compiled {
    /// The new name of `item`; `NoDate` when the rule writes a date and the item has no time to take it from.
    fn new_name(&self, item: &Item<'_>) -> Result<String, RenameProblem> {
        let local = match &self.zone {
            None => None,
            Some(zone) => Some(
                item.modified
                    .and_then(|millis| Local::of(millis, zone))
                    .ok_or(RenameProblem::NoDate)?,
            ),
        };
        let (stem, ext) = split_name(item.name, item.folder);
        let counter = self.counter_text(item.index);
        let context = Context {
            name: stem,
            ext,
            counter: &counter,
            local,
        };

        // Nothing past the limit is made: such a name is too long whatever follows, so it is cut where it is.
        let mut name = self.name.render(&context, MAX_NAME_BYTES);
        if name.len() <= MAX_NAME_BYTES {
            let extension = self.ext.render(&context, MAX_NAME_BYTES);
            if !extension.is_empty() {
                name.push('.');
                name.push_str(&extension);
            }
        }

        if name.len() <= MAX_NAME_BYTES {
            name = self.change(name, item.folder);
        }
        cut(&mut name);

        Ok(name)
    }

    /// `name` (of at most [`MAX_NAME_BYTES`]) after the search and replace and the change of case.
    fn change(&self, mut name: String, folder: bool) -> String {
        if let Some(search) = &self.search {
            name = search.apply(&name, MAX_NAME_BYTES);
            cut(&mut name);
        }

        apply_case(&name, folder, self.case, self.case_part)
    }

    /// The counter of the item at `index`, with its leading zeros.
    fn counter_text(&self, index: usize) -> String {
        let steps = u64::try_from(index).unwrap_or(u64::MAX);
        let value = u64::from(self.counter.start).saturating_add(steps.saturating_mul(u64::from(self.counter.step)));

        format!("{value:0width$}", width = usize::from(self.counter.digits))
    }

    /// The row of the item `name` at `index` of the list: what it would be called, or why that is not known.
    fn draft(&self, folder: &Path, index: usize, name: &str) -> RenameRow {
        let row = |to: &str, problem| RenameRow {
            from: name.to_owned(),
            to: to.to_owned(),
            problem,
        };

        if name.contains(char::REPLACEMENT_CHARACTER) {
            return row(name, Some(RenameProblem::Unreadable));
        }
        let Some((is_folder, modified)) = look_at(folder, name) else {
            return row(name, Some(RenameProblem::Missing));
        };

        let item = Item {
            name,
            folder: is_folder,
            index,
            modified,
        };

        match self.new_name(&item) {
            Ok(to) => row(&to, None),
            Err(problem) => row(name, Some(problem)),
        }
    }
}

/// Cuts `name` to [`MAX_NAME_BYTES`] at a character.
fn cut(name: &mut String) {
    if name.len() <= MAX_NAME_BYTES {
        return;
    }

    let mut end = MAX_NAME_BYTES;
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name.truncate(end);
}

/// Whether the item `name` of `folder` is a folder (a link to one is), and when it was modified; `None` when it is
/// not there. A link is looked at, and followed only to tell a folder from a file, as far as a pane may follow it.
fn look_at(folder: &Path, name: &str) -> Option<(bool, Option<i64>)> {
    if !platform::is_plain_name(name) {
        return None;
    }

    let path = folder.join(name);
    let metadata = fs::symlink_metadata(&path).ok()?;
    let is_folder = if metadata.file_type().is_symlink() {
        platform::follow(&path).is_some_and(|target| target.is_dir())
    } else {
        metadata.is_dir()
    };

    Some((is_folder, modified_millis(&metadata)))
}

/// Whether something is at `name` in `folder`, a link that leads nowhere included.
fn is_taken(folder: &Path, name: &str) -> bool {
    match fs::symlink_metadata(folder.join(name)) {
        Ok(_) => true,
        Err(error) => error.kind() != io::ErrorKind::NotFound,
    }
}

/// What `rule` would do to the items `names` of `folder`: a row for each, in order (the counter counts by the position in
/// `names`), or the fault of the rule. `folder` is a resolved local path and the names are plain names. Blocking disk
/// access (two reads of metadata per item): call it off the command thread.
#[must_use]
pub fn preview_in(folder: &Path, names: &[String], rule: &RenameRule) -> RenamePreview {
    let compiled = match compile(rule) {
        Ok(compiled) => compiled,
        Err(error) => return RenamePreview::Rule { error },
    };

    let drafts = names
        .iter()
        .enumerate()
        .map(|(index, name)| compiled.draft(folder, index, name))
        .collect();
    let rows = check::check(drafts, |name| is_taken(folder, name));

    let problems = rows.iter().filter(|row| row.problem.is_some()).count();
    let changed = rows
        .iter()
        .filter(|row| row.problem.is_none() && row.to != row.from)
        .count();

    RenamePreview::Rows {
        rows,
        changed: u32::try_from(changed).unwrap_or(u32::MAX),
        problems: u32::try_from(problems).unwrap_or(u32::MAX),
    }
}
