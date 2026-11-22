use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How the counter placeholder `[C]` counts: the first value, the step between items, and the width it is padded to
/// with zeros.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RenameCounter {
    /// At most 999 999 999.
    pub start: u32,
    /// 1 to 1000.
    pub step: u32,
    /// 1 to 10.
    pub digits: u8,
}

/// How the letters of a new name change case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CaseChange {
    Keep,
    Lower,
    Upper,
    /// The first letter after anything that is not a letter or a digit (an apostrophe excepted) is uppercase, the rest
    /// lowercase.
    Title,
}

/// What a case change applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CasePart {
    Name,
    Ext,
    Both,
}

/// The field of a rename rule that a [`RuleError`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RuleField {
    Name,
    Ext,
    Counter,
    Search,
    Replace,
}

/// A rename rule: a template for the name and one for the extension, a counter, a search and replace over the whole
/// new name, and a case change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RenameRule {
    /// The template of the name, with placeholders such as `[N]`.
    pub name: String,
    /// The template of the extension, without the dot.
    pub ext: String,
    pub counter: RenameCounter,
    pub search: String,
    pub replace: String,
    /// `search` is a regular expression (and `replace` may use `$1` and `${name}`).
    pub regex: bool,
    pub match_case: bool,
    /// Replace the first match only.
    pub first_only: bool,
    pub case: CaseChange,
    pub case_part: CasePart,
}

/// Why a rule cannot be used: the field it is in fault and a message for the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RuleError {
    pub field: RuleField,
    pub message: String,
}

/// What is wrong with one row of a rename preview. A row with a problem stops the rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RenameProblem {
    /// The new name is empty.
    Empty,
    /// The new name is not a plain file name on this system.
    Invalid,
    /// The new name is longer than the system takes.
    TooLong,
    /// Another row has the same new name.
    Duplicate,
    /// An item of that name is in the folder and the rename does not move it away.
    Exists,
    /// The item is no longer in the folder.
    Missing,
    /// The name of the item cannot be read as text.
    Unreadable,
    /// The rule uses a date and the item has no modification time.
    NoDate,
}

/// One item of a rename preview: its name and the name it would get.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RenameRow {
    pub from: String,
    pub to: String,
    pub problem: Option<RenameProblem>,
}

/// The answer to a preview request: a row for every item, or the fault of the rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum RenamePreview {
    /// `changed` rows get a new name; `problems` rows have a problem.
    Rows {
        rows: Vec<RenameRow>,
        changed: u32,
        problems: u32,
    },
    Rule {
        error: RuleError,
    },
}

/// A rename rule saved under a name, as it is kept in the state file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedRename {
    pub name: String,
    pub rule: RenameRule,
}
