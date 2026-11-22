//! The folders whose rules are remembered, and which of them may be forgotten.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::mem;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use super::KEPT;
use super::layer::Layer;

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
    /// The folder has rules or a `.git`: its own layer, which the place keeps alive.
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

/// A remembered folder that nothing uses: its place is held by the map alone, and so are its rules, or are only held by
/// folders below that are themselves forgotten with it.
struct Idle<'a> {
    /// When the folder, or one below it, was last asked for.
    age: u64,
    /// How many layers are above its own (none when it has none).
    depth: usize,
    dir: &'a PathBuf,
    /// Whether the folder has a layer of its own, that costs nothing when it is a boundary.
    own: bool,
    lines: usize,
    bytes: usize,
}

/// A remembered folder with a layer of its own whose place only the map holds.
struct Owned<'a> {
    dir: &'a PathBuf,
    layer: *const Layer,
    parent: Option<*const Layer>,
    /// How many hold the layer: its place, the layers below it and whatever else uses it.
    holders: usize,
    age: u64,
    depth: usize,
    lines: usize,
    bytes: usize,
}

/// The remembered folders that nothing uses, oldest first and, as old, the deeper first: a folder with a layer of its own is
/// idle when nothing but its place and the layers of idle folders below it hold the layer, so a whole chain of layers is
/// idle at once and the ones below always come before the ones above. A folder without a layer of its own is always idle: it
/// holds nothing but its place.
fn idle(slots: &mut HashMap<PathBuf, Arc<Slot>>) -> Vec<Idle<'_>> {
    let mut idle = Vec::new();
    let mut owned = Vec::new();
    let mut below: HashMap<*const Layer, usize> = HashMap::new();

    for (dir, slot) in slots.iter_mut() {
        // Only the map holds the place of the folder: no lookup is reading or waiting on it, and none can start while the
        // map is locked.
        let Some(slot) = Arc::get_mut(slot) else {
            continue;
        };
        let used = *slot.used.get_mut();

        let (age, depth, lines, bytes) = match slot.held.get_mut().unwrap_or_else(PoisonError::into_inner) {
            Held::Unread => (0, 0, 0, 0),
            Held::Shared(layer) if layer.strong_count() == 0 => (0, 0, 0, 0),
            Held::Shared(_) => (used, 0, 0, 0),
            Held::Own(layer) => {
                let parent = layer.parent.as_ref().map(Arc::as_ptr);
                if let Some(parent) = parent {
                    *below.entry(parent).or_default() += 1;
                }

                let (lines, bytes) = layer.charged();
                owned.push(Owned {
                    dir,
                    layer: Arc::as_ptr(layer),
                    parent,
                    holders: Arc::strong_count(layer),
                    age: layer.used.load(Ordering::Relaxed),
                    depth: layer.depth,
                    lines,
                    bytes,
                });

                continue;
            }
        };

        idle.push(Idle {
            age,
            depth,
            dir,
            own: false,
            lines,
            bytes,
        });
    }

    // Deepest first, so that the layers below one are judged before it is.
    owned.sort_unstable_by_key(|owned| Reverse(owned.depth));

    let mut in_use: HashSet<*const Layer> = HashSet::new();
    let mut used_at: HashMap<*const Layer, u64> = HashMap::new();

    for owned in owned {
        let holders = 1 + below.get(&owned.layer).copied().unwrap_or(0);

        // A layer is no older than the ones below it, whatever the clock says of a race between lookups.
        let age = owned.age.max(used_at.get(&owned.layer).copied().unwrap_or(0));
        if let Some(parent) = owned.parent {
            let above = used_at.entry(parent).or_default();

            *above = (*above).max(age);
        }

        if owned.holders > holders || in_use.contains(&owned.layer) {
            if let Some(parent) = owned.parent {
                in_use.insert(parent);
            }

            continue;
        }

        idle.push(Idle {
            age,
            depth: owned.depth,
            dir: owned.dir,
            own: true,
            lines: owned.lines,
            bytes: owned.bytes,
        });
    }

    idle.sort_unstable_by_key(|idle| (idle.age, Reverse(idle.depth)));

    idle
}

/// Removes the older half of the remembered folders that nothing uses (of those that have a layer of their own only, when
/// `layers`) and returns them, to be dropped once the lock is released.
fn forget_older_half(slots: &mut HashMap<PathBuf, Arc<Slot>>, layers: bool) -> Vec<Arc<Slot>> {
    let idle: Vec<Idle> = idle(slots).into_iter().filter(|idle| !layers || idle.own).collect();
    let half = idle.len().div_ceil(2);
    let gone: Vec<PathBuf> = idle.into_iter().take(half).map(|idle| idle.dir.clone()).collect();

    remove(slots, gone)
}

/// Removes the places of the folders `dirs` and returns them.
fn remove(slots: &mut HashMap<PathBuf, Arc<Slot>>, dirs: Vec<PathBuf>) -> Vec<Arc<Slot>> {
    dirs.iter().filter_map(|dir| slots.remove(dir)).collect()
}

/// Forgets the older half of the idle folders and, while more than `target` are left and a round forgot something, the
/// older half of those with a layer of their own that are idle then (the places of folders without one are only forgotten
/// in the first round, so as not to empty the memory of a walk that has few places to spare). Returns the last of them,
/// to be dropped once the lock is released. What a round forgets is dropped here before the next, so that what only it
/// held is idle for the next.
pub(super) fn forget_down_to(slots: &mut HashMap<PathBuf, Arc<Slot>>, target: usize) -> Vec<Arc<Slot>> {
    let mut gone = forget_older_half(slots, false);

    while !gone.is_empty() && slots.len() > target {
        drop(mem::take(&mut gone));

        gone = forget_older_half(slots, true);
    }

    gone
}

/// Removes the remembered folders that nothing uses, the oldest first, until they held `lines` and `bytes` of the budget
/// and, when there is more to take, up to `extra` bytes more (no more than half of what else could be forgotten, so as not
/// to empty the memory for a small need), and returns them; none at all when they hold less than `lines` and `bytes`
/// together.
pub(super) fn forget_to_free(
    slots: &mut HashMap<PathBuf, Arc<Slot>>,
    lines: usize,
    bytes: usize,
    extra: usize,
) -> Vec<Arc<Slot>> {
    let idle = idle(slots);
    let (held_lines, held_bytes) = idle
        .iter()
        .fold((0, 0), |(lines, bytes), idle| (lines + idle.lines, bytes + idle.bytes));

    if held_lines < lines || held_bytes < bytes {
        return Vec::new();
    }

    let bytes = bytes + extra.min((held_bytes - bytes) / 2);
    let (mut freed_lines, mut freed_bytes) = (0, 0);

    let gone: Vec<PathBuf> = idle
        .into_iter()
        .filter(|idle| idle.own)
        .take_while(|idle| {
            let enough = freed_lines >= lines && freed_bytes >= bytes;

            freed_lines += idle.lines;
            freed_bytes += idle.bytes;

            !enough
        })
        .map(|idle| idle.dir.clone())
        .collect();

    remove(slots, gone)
}
