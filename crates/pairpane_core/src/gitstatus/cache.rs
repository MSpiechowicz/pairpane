//! The decoded index of the repositories read lately. The page asks for a status of each folder it shows, and a flat view of
//! a hundred folders of one repository would otherwise decode the same index a hundred times; with this, the index is
//! decoded once while it does not change. What is kept is at most [`SLOTS`] decoded indexes (the least recently used goes
//! first), each for a key that says which file it was decoded from: the repository's folder, `HEAD`'s commit, and the
//! index file's length and time of modification. A file that was changed in the last moments cannot be told from the one
//! that was decoded by its time (a coarse clock may give the same one for two writes, as git's own racy checks know), so
//! an index younger than the minimum age is decoded each time and not kept.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

/// How many decoded indexes are kept.
const SLOTS: usize = 4;

/// How old an index file has to be to be kept: two seconds is the coarsest clock of the file systems in use (FAT).
const MIN_AGE: Duration = Duration::from_secs(2);

/// The decoded index of the repositories of the last statuses; the one the app uses is [`SHARED`].
pub(super) struct Cache {
    /// The most recently used first.
    slots: Mutex<Vec<(Key, Arc<gix::index::File>)>>,
    min_age: Duration,
    decoded: AtomicUsize,
}

/// What an index was decoded from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Key {
    pub(super) git_dir: PathBuf,
    pub(super) head: Option<gix::ObjectId>,
    pub(super) len: u64,
    pub(super) modified: Option<SystemTime>,
}

/// The cache of the app.
pub(super) static SHARED: Cache = Cache::new(MIN_AGE);

impl Cache {
    pub(super) const fn new(min_age: Duration) -> Self {
        Self {
            slots: Mutex::new(Vec::new()),
            min_age,
            decoded: AtomicUsize::new(0),
        }
    }

    /// How many times `decode` ran for this cache.
    #[cfg(test)]
    pub(super) fn decoded(&self) -> usize {
        self.decoded.load(Ordering::Relaxed)
    }

    /// The index for `key`: the one kept, or what `decode` gives (which is then kept, when the file is old enough).
    pub(super) fn get<E>(
        &self,
        key: Key,
        decode: impl FnOnce() -> Result<gix::index::File, E>,
    ) -> Result<Arc<gix::index::File>, E> {
        if let Some(kept) = self.find(&key) {
            return Ok(kept);
        }

        // Decoding takes its time, and is not done with the lock held: two statuses may decode the same index.
        let decoded = Arc::new(decode()?);
        self.decoded.fetch_add(1, Ordering::Relaxed);
        if self.keeps(&key) {
            self.put(key, &decoded);
        }

        Ok(decoded)
    }

    fn find(&self, key: &Key) -> Option<Arc<gix::index::File>> {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        let at = slots.iter().position(|(kept, _)| kept == key)?;
        let used = slots.remove(at);
        let index = Arc::clone(&used.1);
        slots.insert(0, used);

        Some(index)
    }

    fn put(&self, key: Key, index: &Arc<gix::index::File>) {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        // The same repository is kept once: an index that was changed leaves nothing behind but its place.
        slots.retain(|(kept, _)| kept.git_dir != key.git_dir);
        slots.insert(0, (key, Arc::clone(index)));
        slots.truncate(SLOTS);
    }

    /// Whether an index with this key is kept: it has a time of modification, and it is older than the minimum age.
    fn keeps(&self, key: &Key) -> bool {
        key.modified
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age >= self.min_age)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn key(name: &str, len: u64) -> Key {
        Key {
            git_dir: PathBuf::from(name),
            head: None,
            len,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000)),
        }
    }

    fn index() -> Result<gix::index::File, ()> {
        Ok(gix::index::File::from_state(
            gix::index::State::new(gix::hash::Kind::Sha1),
            "index",
        ))
    }

    /// A decode that must not run: the index is kept.
    fn kept() -> Result<gix::index::File, ()> {
        panic!("the index was decoded again")
    }

    #[test]
    fn the_same_key_decodes_once_and_another_key_decodes_again() {
        let cache = Cache::new(Duration::ZERO);

        let first = cache.get(key("a", 1), index).expect("decoded");
        let again = cache.get(key("a", 1), kept).expect("kept");
        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!(cache.decoded(), 1);

        // The index of the same repository changed: decoded again, and the old one is not kept next to it.
        cache.get(key("a", 2), index).expect("decoded");
        assert_eq!(cache.decoded(), 2);
        assert_eq!(cache.slots.lock().expect("lock").len(), 1);
    }

    #[test]
    fn only_the_four_most_recently_used_repositories_are_kept() {
        let cache = Cache::new(Duration::ZERO);

        for name in ["a", "b", "c", "d"] {
            cache.get(key(name, 1), index).expect("decoded");
        }
        // `a` is used again, so `b` is the one that goes when a fifth comes.
        cache.get(key("a", 1), kept).expect("kept");
        cache.get(key("e", 1), index).expect("decoded");

        assert_eq!(cache.decoded(), 5);
        for name in ["a", "c", "d", "e"] {
            cache.get(key(name, 1), kept).expect("kept");
        }
        cache.get(key("b", 1), index).expect("decoded");
        assert_eq!(cache.decoded(), 6);
    }

    #[test]
    fn an_index_changed_a_moment_ago_is_decoded_every_time() {
        let cache = Cache::new(Duration::from_secs(2));
        let fresh = Key {
            modified: Some(SystemTime::now()),
            ..key("a", 1)
        };
        let unknown = Key {
            modified: None,
            ..key("b", 1)
        };

        for key in [fresh, unknown] {
            cache.get(key.clone(), index).expect("decoded");
            cache.get(key, index).expect("decoded");
        }

        assert_eq!(cache.decoded(), 4);
    }

    #[test]
    fn a_failed_decode_is_not_kept() {
        let cache = Cache::new(Duration::ZERO);

        assert!(
            cache
                .get(key("a", 1), || Err::<gix::index::File, _>("damaged"))
                .is_err()
        );
        cache.get(key("a", 1), index).expect("decoded");

        assert_eq!(cache.decoded(), 1);
    }
}
