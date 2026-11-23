//! The thumbnails made so far, kept in memory so that a folder visited again shows at once.
//!
//! A thumbnail is kept under the file's canonical path and the size it was made at, together with the length and the
//! modification time of the file it was made of (its stamp). A lookup with another stamp is a miss and drops the entry
//! (the file changed, so it is made again), so a changed file never leaves a stale picture behind. The cache is bound by
//! the bytes of the pictures: when a new one does not fit, the ones used longest ago go first. A picture that is bigger
//! than the whole budget is not kept.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

/// What a thumbnail is of: the file, by its canonical path, and the size it was made at (see `thumb::bucket`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Key {
    pub(super) path: PathBuf,
    pub(super) size: u32,
}

/// What tells that a file is still the one a thumbnail was made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Stamp {
    pub(super) len: u64,
    pub(super) modified: Option<SystemTime>,
}

struct Entry {
    stamp: Stamp,
    picture: Arc<[u8]>,
    /// When it was last used: the key of its place in `Cache::order`.
    used: u64,
}

/// The pictures, at most `budget` bytes of them together.
pub(super) struct Cache {
    budget: usize,
    bytes: usize,
    /// The number of the latest use; each use takes the next one.
    clock: u64,
    entries: HashMap<Key, Entry>,
    /// The keys by the time they were last used, the least recent first.
    order: BTreeMap<u64, Key>,
}

impl Cache {
    pub(super) fn new(budget: usize) -> Self {
        Self {
            budget,
            bytes: 0,
            clock: 0,
            entries: HashMap::new(),
            order: BTreeMap::new(),
        }
    }

    /// The picture kept for `key` when it was made of a file with this `stamp`, which counts as a use. One made of a file
    /// with another stamp is dropped and is not given.
    pub(super) fn get(&mut self, key: &Key, stamp: &Stamp) -> Option<Arc<[u8]>> {
        let entry = self.entries.get(key)?;
        if entry.stamp != *stamp {
            self.remove(key);

            return None;
        }

        self.clock += 1;
        let entry = self.entries.get_mut(key)?;
        self.order.remove(&entry.used);
        entry.used = self.clock;
        self.order.insert(self.clock, key.clone());

        Some(Arc::clone(&entry.picture))
    }

    /// Keeps `picture` for `key`, made of a file with this `stamp`, in place of what was kept for it. The pictures used
    /// longest ago are dropped until it fits; one that cannot fit at all is not kept.
    pub(super) fn insert(&mut self, key: Key, stamp: Stamp, picture: Arc<[u8]>) {
        self.remove(&key);
        if picture.len() > self.budget {
            return;
        }

        while self.bytes + picture.len() > self.budget {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            if let Some(gone) = self.entries.remove(&oldest) {
                self.bytes -= gone.picture.len();
            }
        }

        self.clock += 1;
        self.bytes += picture.len();
        self.order.insert(self.clock, key.clone());
        self.entries.insert(
            key,
            Entry {
                stamp,
                picture,
                used: self.clock,
            },
        );
    }

    /// How many bytes of pictures are kept.
    #[cfg(test)]
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }

    fn remove(&mut self, key: &Key) {
        if let Some(gone) = self.entries.remove(key) {
            self.order.remove(&gone.used);
            self.bytes -= gone.picture.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    fn key(name: &str, size: u32) -> Key {
        Key {
            path: PathBuf::from(format!("/pictures/{name}")),
            size,
        }
    }

    fn stamp(len: u64, seconds: u64) -> Stamp {
        Stamp {
            len,
            modified: Some(UNIX_EPOCH + Duration::from_secs(seconds)),
        }
    }

    fn picture(bytes: usize, fill: u8) -> Arc<[u8]> {
        vec![fill; bytes].into()
    }

    #[test]
    fn a_picture_is_given_back_for_its_key_and_stamp_only() {
        let mut cache = Cache::new(1000);
        cache.insert(key("a", 128), stamp(10, 1), picture(100, 1));

        assert_eq!(
            cache.get(&key("a", 128), &stamp(10, 1)).as_deref(),
            Some(&[1_u8; 100][..])
        );
        assert!(
            cache.get(&key("a", 64), &stamp(10, 1)).is_none(),
            "another size is another picture"
        );
        assert!(
            cache.get(&key("b", 128), &stamp(10, 1)).is_none(),
            "another file is another picture"
        );
        assert_eq!(cache.bytes(), 100);
    }

    #[test]
    fn a_file_that_changed_in_length_or_time_is_made_again_and_its_old_picture_is_dropped() {
        for changed in [
            stamp(11, 1),
            stamp(10, 2),
            Stamp {
                len: 10,
                modified: None,
            },
        ] {
            let mut cache = Cache::new(1000);
            cache.insert(key("a", 128), stamp(10, 1), picture(100, 1));

            assert!(cache.get(&key("a", 128), &changed).is_none(), "{changed:?}");
            assert_eq!(cache.bytes(), 0, "the stale picture does not stay");
            assert!(
                cache.get(&key("a", 128), &stamp(10, 1)).is_none(),
                "and is not given back later"
            );
        }
    }

    #[test]
    fn the_pictures_used_longest_ago_go_first_when_the_bytes_run_out() {
        let mut cache = Cache::new(300);
        for (name, fill) in [("a", 1), ("b", 2), ("c", 3)] {
            cache.insert(key(name, 128), stamp(1, 1), picture(100, fill));
        }
        // `a` is used again, so `b` is now the one used longest ago.
        assert!(cache.get(&key("a", 128), &stamp(1, 1)).is_some());

        cache.insert(key("d", 128), stamp(1, 1), picture(100, 4));

        assert!(
            cache.get(&key("b", 128), &stamp(1, 1)).is_none(),
            "the least recently used went"
        );
        for name in ["a", "c", "d"] {
            assert!(cache.get(&key(name, 128), &stamp(1, 1)).is_some(), "{name}");
        }
        assert_eq!(cache.bytes(), 300);
    }

    #[test]
    fn a_bigger_picture_pushes_out_as_many_as_it_needs() {
        let mut cache = Cache::new(300);
        for name in ["a", "b", "c"] {
            cache.insert(key(name, 128), stamp(1, 1), picture(100, 0));
        }

        cache.insert(key("big", 128), stamp(1, 1), picture(250, 9));

        assert_eq!(cache.bytes(), 250);
        for name in ["a", "b", "c"] {
            assert!(cache.get(&key(name, 128), &stamp(1, 1)).is_none(), "{name}");
        }
        assert!(cache.get(&key("big", 128), &stamp(1, 1)).is_some());
    }

    #[test]
    fn a_picture_over_the_whole_budget_is_not_kept_and_pushes_nothing_out() {
        let mut cache = Cache::new(300);
        cache.insert(key("a", 128), stamp(1, 1), picture(100, 1));

        cache.insert(key("huge", 128), stamp(1, 1), picture(301, 2));

        assert!(cache.get(&key("huge", 128), &stamp(1, 1)).is_none());
        assert!(cache.get(&key("a", 128), &stamp(1, 1)).is_some());
        assert_eq!(cache.bytes(), 100);
    }

    #[test]
    fn a_picture_made_again_replaces_the_old_one_and_its_bytes() {
        let mut cache = Cache::new(300);
        cache.insert(key("a", 128), stamp(1, 1), picture(100, 1));
        cache.insert(key("a", 128), stamp(2, 2), picture(150, 2));

        assert_eq!(cache.bytes(), 150);
        assert_eq!(
            cache.get(&key("a", 128), &stamp(2, 2)).as_deref(),
            Some(&[2_u8; 150][..])
        );
        assert!(cache.get(&key("a", 128), &stamp(1, 1)).is_none());
    }

    #[test]
    fn the_bytes_and_the_order_stay_in_step_over_many_uses() {
        let mut cache = Cache::new(1000);
        for round in 0..200_u32 {
            let name = format!("f{}", round % 17);
            if cache.get(&key(&name, 64), &stamp(1, 1)).is_none() {
                cache.insert(key(&name, 64), stamp(1, 1), picture(90 + (round % 5) as usize, 0));
            }

            assert!(cache.bytes() <= 1000);
            assert_eq!(cache.entries.len(), cache.order.len());
            let sum: usize = cache.entries.values().map(|entry| entry.picture.len()).sum();
            assert_eq!(sum, cache.bytes());
        }
    }
}
