//! The folders whose rules are remembered, and which of them may be forgotten.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

use super::{KEPT, Layer};

/// The place of one folder's rules in the memory of the search: read once, by whichever thread comes first.
pub(super) struct Slot {
    pub(super) layer: Mutex<Option<Arc<Layer>>>,
}

/// The folders whose rules are remembered.
pub(super) struct Kept {
    pub(super) slots: HashMap<PathBuf, Arc<Slot>>,
    /// The number of folders past which the idle ones are forgotten.
    pub(super) trim_at: usize,
}

impl Slot {
    pub(super) fn new() -> Self {
        Self {
            layer: Mutex::new(None),
        }
    }
}

impl Kept {
    pub(super) fn new() -> Self {
        Self {
            slots: HashMap::new(),
            trim_at: KEPT,
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

/// The remembered folders that nothing uses, oldest first.
fn idle(slots: &mut HashMap<PathBuf, Arc<Slot>>) -> Vec<Idle> {
    let mut idle: Vec<Idle> = slots
        .iter_mut()
        .filter_map(|(dir, slot)| {
            // Only the map holds the place of the folder: no lookup is reading or waiting on it, and none can start
            // while the map is locked.
            let held = Arc::get_mut(slot)?
                .layer
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner);

            let Some(layer) = held.as_ref() else {
                return Some(Idle {
                    age: 0,
                    dir: dir.clone(),
                    lines: 0,
                    bytes: 0,
                });
            };

            // Held by a folder below, a thread's pin or a lookup too: the rules are in use, and dropping the place
            // would free nothing.
            if Arc::strong_count(layer) > 1 {
                return None;
            }

            let (lines, bytes) = layer.charged();

            Some(Idle {
                age: layer.used.load(Ordering::Relaxed),
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
