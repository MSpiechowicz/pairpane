//! Helpers shared by the tests of the rename.

use crate::model::{CaseChange, CasePart, RenameCounter, RenameRule};

/// A rule that changes nothing: the name and the extension as they are.
pub(super) fn rule() -> RenameRule {
    RenameRule {
        name: "[N]".to_owned(),
        ext: "[E]".to_owned(),
        counter: RenameCounter {
            start: 1,
            step: 1,
            digits: 1,
        },
        search: String::new(),
        replace: String::new(),
        regex: false,
        match_case: true,
        first_only: false,
        case: CaseChange::Keep,
        case_part: CasePart::Both,
    }
}

/// The rule with the templates of `name` and `ext`.
pub(super) fn templated(name: &str, ext: &str) -> RenameRule {
    RenameRule {
        name: name.to_owned(),
        ext: ext.to_owned(),
        ..rule()
    }
}
