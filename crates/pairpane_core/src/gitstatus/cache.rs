//! The decoded index of the repositories read lately. The page asks for a status of each folder it shows, and a flat view of
//! a hundred folders of one repository would otherwise decode the same index a hundred times; with this, the index is
//! decoded once while it does not change. What is kept is at most [`SLOTS`] decoded indexes (the least recently used goes
//! first), each for a key that says which file it was decoded from: the repository's folder, `HEAD`'s commit, and the
//! index file's length, time of modification, device and inode, and time of change (where the system has them: an index
//! that was replaced by a file of the same length and time of modification is another file, and a write cannot be hidden by
//! setting the time of modification back, which the time of change shows). A file that was changed in the last moments cannot be told from the one
//! that was decoded by its time (a coarse clock may give the same one for two writes, as git's own racy checks know), so
//! an index younger than the minimum age is decoded each time and not kept.

use std::fs::Metadata;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use crate::listing::platform;

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
    /// The device and the inode of the file, where the system has them.
    pub(super) identity: Option<(u64, u64)>,
    /// The time the file's status last changed (seconds and nanoseconds), where the system has it.
    pub(super) changed: Option<(i64, i64)>,
}

impl Key {
    /// The key of the index file whose metadata is `metadata`, in the repository of `git_dir` at the commit `head`.
    pub(super) fn of(git_dir: PathBuf, head: Option<gix::ObjectId>, metadata: &Metadata) -> Self {
        Self {
            git_dir,
            head,
            len: metadata.len(),
            modified: metadata.modified().ok(),
            identity: platform::identity(metadata),
            changed: changed_at(metadata),
        }
    }
}

#[cfg(unix)]
fn changed_at(metadata: &Metadata) -> Option<(i64, i64)> {
    use std::os::unix::fs::MetadataExt;

    Some((metadata.ctime(), metadata.ctime_nsec()))
}

#[cfg(not(unix))]
fn changed_at(_metadata: &Metadata) -> Option<(i64, i64)> {
    None
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
            identity: None,
            changed: None,
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

    /// Sets the time of modification of the file at `path` (the way a program that restores it does).
    #[cfg(unix)]
    fn set_modified(path: &std::path::Path, time: SystemTime) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .expect("the file")
            .set_modified(time)
            .expect("set");
    }

    #[cfg(unix)]
    #[test]
    fn an_index_replaced_by_a_file_of_the_same_length_and_time_is_not_the_one_that_was_decoded() {
        let folder = tempfile::tempdir().expect("a folder");
        let path = folder.path().join("index");
        let then = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let key = |path: &std::path::Path| {
            let metadata = std::fs::metadata(path).expect("metadata");

            Key::of(folder.path().to_path_buf(), None, &metadata)
        };
        std::fs::write(&path, b"0123456789").expect("an index");
        set_modified(&path, then);
        let original = key(&path);
        assert_eq!(original, key(&path), "the same file has the same key");

        // Another file in its place (the inode is another one), with the same length and time of modification.
        let other = folder.path().join("other");
        std::fs::write(&other, b"9876543210").expect("an index");
        set_modified(&other, then);
        std::fs::rename(&other, &path).expect("replaced");
        let replaced = key(&path);
        assert_eq!((replaced.len, replaced.modified), (original.len, original.modified));
        assert_ne!(replaced, original);

        // The same file written in place, with the time of modification put back: the time of change shows it.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&path, b"abcdefghij").expect("written");
        set_modified(&path, then);
        let rewritten = key(&path);
        assert_eq!((rewritten.len, rewritten.modified), (replaced.len, replaced.modified));
        assert_eq!(rewritten.identity, replaced.identity);
        assert_ne!(rewritten, replaced);

        // And the cache decodes each of them.
        let cache = Cache::new(Duration::ZERO);
        for key in [original, replaced, rewritten] {
            cache.get(key, index).expect("decoded");
        }
        assert_eq!(cache.decoded(), 3);
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
