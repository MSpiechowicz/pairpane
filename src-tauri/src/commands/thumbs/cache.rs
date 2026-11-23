//! The thumbnails made so far, and the pictures that could not be made, kept in memory so that a folder visited again
//! shows at once and a file that is bad or slow to decode is not tried again and again.
//!
//! A thumbnail is kept under the file's path and the size it was made at, together with what the file was when it was
//! made of (its stamp: length, modification time and identity). A lookup with another stamp is a miss and drops the entry
//! (the file changed, so it is made again), so a changed file never leaves a stale picture behind. What is kept is
//! either the picture or the failure (`Failed`: the code and the fixed text the page was told, which are told again), and
//! a failure costs [`FAILURE_BYTES`] of the budget. The cache is bound by the bytes: when a new entry does not fit, the
//! ones used longest ago go first. A picture that is bigger than the whole budget is not kept.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use pairpane_core::model::IpcErrorCode;
use pairpane_core::view::thumb::Identity;

/// What a failure costs of the budget, whatever its text: a file that is bad is remembered cheaply.
pub(super) const FAILURE_BYTES: usize = 256;

/// What a thumbnail is of: the file, by its path in the folder the page named (the folder with links resolved where
/// the system needs it, joined with the name, never a link on the last component), and the size it was made at (see
/// `thumb::bucket`).
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
    pub(super) identity: Option<Identity>,
}

/// Why no thumbnail could be made, as the page was told: a bad picture, one that is too big, one that is of no format
/// that has a thumbnail (or has no decoder here). The same code and text are given again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Failed {
    pub(super) code: IpcErrorCode,
    pub(super) message: String,
}

/// What is kept for a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Cached {
    Picture(Arc<[u8]>),
    Failed(Failed),
}

impl Cached {
    /// What it takes of the budget.
    fn bytes(&self) -> usize {
        match self {
            Self::Picture(picture) => picture.len(),
            Self::Failed(_) => FAILURE_BYTES,
        }
    }
}

struct Entry {
    stamp: Stamp,
    value: Cached,
    /// When it was last used: the key of its place in `Cache::order`.
    used: u64,
}

/// The entries, at most `budget` bytes of them together.
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

    /// What is kept for `key` when it was made of a file with this `stamp`, which counts as a use. What was made of a file
    /// with another stamp is dropped and is not given.
    pub(super) fn get(&mut self, key: &Key, stamp: &Stamp) -> Option<Cached> {
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

        Some(entry.value.clone())
    }

    /// Keeps `value` for `key`, made of a file with this `stamp`, in place of what was kept for it. The entries used
    /// longest ago are dropped until it fits; one that cannot fit at all is not kept.
    pub(super) fn insert(&mut self, key: Key, stamp: Stamp, value: Cached) {
        self.remove(&key);

        let size = value.bytes();
        if size > self.budget {
            return;
        }

        while self.bytes + size > self.budget {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            if let Some(gone) = self.entries.remove(&oldest) {
                self.bytes -= gone.value.bytes();
            }
        }

        self.clock += 1;
        self.bytes += size;
        self.order.insert(self.clock, key.clone());
        self.entries.insert(
            key,
            Entry {
                stamp,
                value,
                used: self.clock,
            },
        );
    }

    /// How many bytes of the budget are used.
    #[cfg(test)]
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }

    fn remove(&mut self, key: &Key) {
        if let Some(gone) = self.entries.remove(key) {
            self.order.remove(&gone.used);
            self.bytes -= gone.value.bytes();
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
            identity: None,
        }
    }

    fn picture(bytes: usize, fill: u8) -> Cached {
        Cached::Picture(vec![fill; bytes].into())
    }

    fn failed(text: &str) -> Cached {
        Cached::Failed(Failed {
            code: IpcErrorCode::InvalidInput,
            message: text.to_owned(),
        })
    }

    /// The identity of the file `name` that `folder` holds.
    fn identity_of(folder: &std::path::Path, name: &str) -> Option<Identity> {
        pairpane_core::view::thumb::open(folder, name)
            .expect("a picture")
            .identity
    }

    #[test]
    fn a_picture_is_given_back_for_its_key_and_stamp_only() {
        let mut cache = Cache::new(1000);
        cache.insert(key("a", 128), stamp(10, 1), picture(100, 1));

        assert_eq!(cache.get(&key("a", 128), &stamp(10, 1)), Some(picture(100, 1)));
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
        let no_time = Stamp {
            modified: None,
            ..stamp(10, 1)
        };

        for changed in [stamp(11, 1), stamp(10, 2), no_time] {
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
    fn a_file_that_was_replaced_by_one_of_the_same_length_and_time_is_another_file() {
        let folder = tempfile::tempdir().expect("a folder");
        let head = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        std::fs::write(folder.path().join("a.png"), head).expect("a file");
        std::fs::write(folder.path().join("b.png"), head).expect("a file");
        let (Some(a), Some(b)) = (identity_of(folder.path(), "a.png"), identity_of(folder.path(), "b.png")) else {
            eprintln!("this system does not tell which file is which");

            return;
        };
        assert_ne!(a, b, "two files are two identities");
        let same = |identity| Stamp {
            identity: Some(identity),
            ..stamp(10, 1)
        };

        let mut cache = Cache::new(1000);
        cache.insert(key("a", 128), same(a), picture(100, 1));

        assert!(cache.get(&key("a", 128), &same(b)).is_none());
        assert_eq!(cache.bytes(), 0);
    }

    #[test]
    fn a_failure_is_kept_for_a_small_fixed_cost_and_given_back_as_it_was() {
        let mut cache = Cache::new(1000);
        let kept = failed("the image cannot be decoded");

        cache.insert(key("bad", 128), stamp(5, 1), kept.clone());

        assert_eq!(cache.bytes(), FAILURE_BYTES);
        assert_eq!(cache.get(&key("bad", 128), &stamp(5, 1)), Some(kept));
        assert!(
            cache.get(&key("bad", 128), &stamp(6, 1)).is_none(),
            "a file that changed may be a good picture now"
        );
        assert_eq!(cache.bytes(), 0);
    }

    #[test]
    fn failures_take_their_turn_with_the_pictures_when_the_bytes_run_out() {
        let mut cache = Cache::new(2 * FAILURE_BYTES + 100);
        cache.insert(key("a", 128), stamp(1, 1), failed("one"));
        cache.insert(key("b", 128), stamp(1, 1), failed("two"));
        cache.insert(key("c", 128), stamp(1, 1), picture(100, 1));
        assert_eq!(cache.bytes(), 2 * FAILURE_BYTES + 100);

        // The picture of 300 bytes needs room: the failure used longest ago goes, and then the next one.
        cache.insert(key("d", 128), stamp(1, 1), picture(300, 2));

        assert!(cache.get(&key("a", 128), &stamp(1, 1)).is_none());
        assert!(cache.get(&key("b", 128), &stamp(1, 1)).is_none());
        assert!(cache.get(&key("c", 128), &stamp(1, 1)).is_some());
        assert!(cache.get(&key("d", 128), &stamp(1, 1)).is_some());
        assert!(cache.bytes() <= 2 * FAILURE_BYTES + 100);
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
        assert_eq!(cache.get(&key("a", 128), &stamp(2, 2)), Some(picture(150, 2)));
        assert!(cache.get(&key("a", 128), &stamp(1, 1)).is_none());
    }

    #[test]
    fn the_bytes_and_the_order_stay_in_step_over_many_uses() {
        let mut cache = Cache::new(1000);
        for round in 0..200_u32 {
            let name = format!("f{}", round % 17);
            if cache.get(&key(&name, 64), &stamp(1, 1)).is_none() {
                let value = if round % 7 == 0 {
                    failed("bad")
                } else {
                    picture(90 + (round % 5) as usize, 0)
                };
                cache.insert(key(&name, 64), stamp(1, 1), value);
            }

            assert!(cache.bytes() <= 1000);
            assert_eq!(cache.entries.len(), cache.order.len());
            let sum: usize = cache.entries.values().map(|entry| entry.value.bytes()).sum();
            assert_eq!(sum, cache.bytes());
        }
    }
}
