//! The folders whose rules are remembered, and which of them may be forgotten.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use super::{KEPT, Layer};

/// The place of one folder's rules in the memory of the search: read once, by whichever thread comes first.
pub(super) struct Slot {
    pub(super) held: Mutex<Held>,
    /// When the folder was last asked for.
    pub(super) used: AtomicU64,
}

/// What a folder's place holds.
pub(super) enum Held {
    /// The folder was not read yet.
    Unread,
    /// The folder has rules (or is a repository): its own layer, which the place keeps alive.
    Own(Arc<Layer>),
    /// The folder has none: the layer of the nearest folder above that has stands for it. Only a weak link, so that the
    /// folders below one cannot keep its rules from being forgotten; when they are, the folder is read again.
    Shared(Weak<Layer>),
}

impl Held {
    /// The layer that applies in the folder, if it is read and still alive.
    pub(super) fn layer(&self) -> Option<Arc<Layer>> {
        match self {
            Self::Unread => None,
            Self::Own(layer) => Some(Arc::clone(layer)),
            Self::Shared(layer) => layer.upgrade(),
        }
    }
}

/// The folders whose rules are remembered.
pub(super) struct Kept {
    pub(super) slots: HashMap<PathBuf, Arc<Slot>>,
    /// The number of folders at which the idle ones are forgotten, before another is added.
    pub(super) trim_at: usize,
    /// The most folders that were remembered at once.
    #[cfg(test)]
    pub(super) peak: usize,
}

impl Slot {
    pub(super) fn new() -> Self {
        Self {
            held: Mutex::new(Held::Unread),
            used: AtomicU64::new(0),
        }
    }
}

impl Kept {
    pub(super) fn new() -> Self {
        Self {
            slots: HashMap::new(),
            trim_at: KEPT,
            #[cfg(test)]
            peak: 0,
        }
    }
}

/// A remembered folder that nothing uses: its place is held by the map alone and so are its rules.
struct Idle {
    /// When the folder was last asked for.
    age: u64,
    dir: PathBuf,
    lines: usize,
    bytes: usize,
}

/// The remembered folders that nothing uses, oldest first. A folder without rules of its own is always idle: it holds
/// nothing but its place.
fn idle(slots: &mut HashMap<PathBuf, Arc<Slot>>) -> Vec<Idle> {
    let mut idle: Vec<Idle> = slots
        .iter_mut()
        .filter_map(|(dir, slot)| {
            // Only the map holds the place of the folder: no lookup is reading or waiting on it, and none can start
            // while the map is locked.
            let slot = Arc::get_mut(slot)?;
            let used = *slot.used.get_mut();
            let held = slot.held.get_mut().unwrap_or_else(PoisonError::into_inner);

            let (age, lines, bytes) = match held {
                Held::Unread => (0, 0, 0),
                Held::Shared(layer) if layer.strong_count() == 0 => (0, 0, 0),
                Held::Shared(_) => (used, 0, 0),
                Held::Own(layer) => {
                    // Held by a folder below, a thread's pin or a lookup too: the rules are in use, and dropping the
                    // place would free nothing.
                    if Arc::strong_count(layer) > 1 {
                        return None;
                    }

                    let (lines, bytes) = layer.charged();

                    (layer.used.load(Ordering::Relaxed), lines, bytes)
                }
            };

            Some(Idle {
                age,
                dir: dir.clone(),
                lines,
                bytes,
            })
        })
        .collect();

    idle.sort_unstable_by_key(|idle| idle.age);

    idle
}

/// Removes the older half of the remembered folders that nothing uses and returns them, to be dropped once the lock is
/// released.
pub(super) fn forget_older_half(slots: &mut HashMap<PathBuf, Arc<Slot>>) -> Vec<Arc<Slot>> {
    let idle = idle(slots);
    let half = idle.len().div_ceil(2);

    idle.into_iter()
        .take(half)
        .filter_map(|idle| slots.remove(&idle.dir))
        .collect()
}

/// Removes the remembered folders that nothing uses, the oldest first, until they held `lines` and `bytes` of the
/// budget, and returns them; none at all when they hold less than that together.
pub(super) fn forget_to_free(slots: &mut HashMap<PathBuf, Arc<Slot>>, lines: usize, bytes: usize) -> Vec<Arc<Slot>> {
    let idle = idle(slots);

    if idle.iter().map(|idle| idle.lines).sum::<usize>() < lines
        || idle.iter().map(|idle| idle.bytes).sum::<usize>() < bytes
    {
        return Vec::new();
    }

    let (mut freed_lines, mut freed_bytes) = (0, 0);

    idle.into_iter()
        .filter(|idle| idle.lines > 0)
        .take_while(|idle| {
            let enough = freed_lines >= lines && freed_bytes >= bytes;

            freed_lines += idle.lines;
            freed_bytes += idle.bytes;

            !enough
        })
        .filter_map(|idle| slots.remove(&idle.dir))
        .collect()
}
