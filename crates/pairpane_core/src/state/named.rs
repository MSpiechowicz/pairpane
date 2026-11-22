//! What the saved searches and the saved rename rules of the state file have in common: a name, and a cap on how many
//! are kept.

use std::collections::HashSet;

/// The longest name of a saved search or rename rule, in characters (after trimming).
pub const MAX_NAME_CHARS: usize = 64;

/// `items` with everything unusable removed, the first `cap` of the rest kept, in their order. `name` gives the name of an
/// item, and `can_run` says whether the item could be used at all.
///
/// Dropped: a name that is empty or only whitespace, one over [`MAX_NAME_CHARS`] characters, one that an earlier kept
/// item has (names are compared without regard to case; a dropped item does not take its name), and an item that cannot
/// run. A kept name is the trimmed one. Items beyond the cap are not even examined.
pub(super) fn clean_named<T>(
    items: Vec<T>,
    cap: usize,
    name: impl Fn(&mut T) -> &mut String,
    can_run: impl Fn(&T) -> bool,
) -> Vec<T> {
    let mut names = HashSet::new();

    items
        .into_iter()
        .filter_map(|mut item| {
            let trimmed = name(&mut item).trim().to_owned();
            if trimmed.is_empty() || trimmed.chars().count() > MAX_NAME_CHARS {
                return None;
            }
            if names.contains(&trimmed.to_lowercase()) || !can_run(&item) {
                return None;
            }

            names.insert(trimmed.to_lowercase());
            *name(&mut item) = trimmed;

            Some(item)
        })
        .take(cap)
        .collect()
}
