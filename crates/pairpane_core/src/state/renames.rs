//! The saved rename rules of the state file.
//!
//! A saved rule is untrusted like everything else in the file, and it is only ever run later, on folders chosen then. So
//! [`clean_renames`] keeps the ones that could run: a name that is usable and not taken, and a rule that
//! [`rename::compile`] accepts (its templates, its counter, and its search within its limits).

use crate::model::SavedRename;
use crate::rename;

use super::named::clean_named;

/// Saved rename rules kept.
pub const MAX_RENAMES: usize = 50;

/// `renames` with everything unusable removed, the first [`MAX_RENAMES`] of the rest kept, in their order (see
/// [`clean_named`] for what is dropped).
pub(super) fn clean_renames(renames: Vec<SavedRename>) -> Vec<SavedRename> {
    clean_named(
        renames,
        MAX_RENAMES,
        |saved| &mut saved.name,
        |saved| rename::compile(&saved.rule).is_ok(),
    )
}
