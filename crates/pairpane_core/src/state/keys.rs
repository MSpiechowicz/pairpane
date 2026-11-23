//! The keys the user set for the actions of the keymap, as the state file keeps them.
//!
//! They are untrusted like everything else in the file, and the page that reads them back knows the actions and which keys
//! it takes: here only what is plainly not a key of an action goes, and the list is kept small.

use std::collections::HashSet;

use crate::model::{KeyChord, KeyOverride};

/// Actions with keys of their own kept.
pub const MAX_KEY_OVERRIDES: usize = 64;

/// Keys kept for one action.
pub const MAX_KEYS_PER_ACTION: usize = 4;

/// The longest name of an action or a key, in characters.
const MAX_NAME_CHARS: usize = 32;

/// `keys` with what cannot be an action's key removed: an action whose name is not plain letters, one named twice (the
/// first kept), a key whose name is empty, too long or holds anything but letters, digits and a few signs. At most
/// [`MAX_KEY_OVERRIDES`] actions and [`MAX_KEYS_PER_ACTION`] keys of each are kept, in their order.
pub(super) fn clean_keys(keys: Vec<KeyOverride>) -> Vec<KeyOverride> {
    let mut seen = HashSet::new();

    keys.into_iter()
        .filter(|entry| is_action(&entry.action) && seen.insert(entry.action.clone()))
        .take(MAX_KEY_OVERRIDES)
        .map(|entry| KeyOverride {
            keys: entry
                .keys
                .into_iter()
                .filter(is_key)
                .take(MAX_KEYS_PER_ACTION)
                .collect(),
            action: entry.action,
        })
        .collect()
}

fn is_action(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= MAX_NAME_CHARS && name.chars().all(|char| char.is_ascii_alphabetic())
}

fn is_key(chord: &KeyChord) -> bool {
    let key = &chord.key;

    !key.is_empty()
        && key.chars().count() <= MAX_NAME_CHARS
        && key
            .chars()
            .all(|char| !char.is_control() && !char.is_whitespace() || char == ' ')
}
