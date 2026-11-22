//! Duplicate files: the candidates of the walk are grouped by size, then by the hash of their first [`HASH_HEAD`] bytes,
//! then by the hash of everything, and a group of two or more that is left is a group of duplicates. A hash is BLAKE3;
//! files are never compared by name or date. Hard links to one file are one file, not twins.
//!
//! The hashing runs on a pool of [`HASH_THREADS`] threads of its own and each group is sent to the caller as soon as it
//! is confirmed. Every read looks at the stop flag, so a stop is heard within one read of [`BUFFER`] bytes.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Sender};
use std::thread;

use rayon::prelude::*;

use crate::model::{EntryKind, FindHit};
use crate::ops::open_listed;

use super::out::{Out, Run, pump};
use super::walk::Candidate;
use super::{HASH_HEAD, HASH_THREADS};

/// The bytes one read takes.
const BUFFER: usize = 64 * 1024;

/// A confirmed group, from a hashing thread.
struct Group {
    size: u64,
    hits: Vec<FindHit>,
}

/// Finds the duplicates among `candidates` (files of a size above 0 that fit the tests of the search) and sends each
/// group to `out`. The progress counts the bytes hashed.
pub(crate) fn find(root: &Path, candidates: Vec<Candidate>, run: &Run<'_>, out: &mut Out<'_>) {
    let groups = same_size(candidates);
    let (sender, receiver) = mpsc::channel();

    run.bytes.store(0, Ordering::Relaxed);

    thread::scope(|scope| {
        scope.spawn(move || {
            let work = |group: Vec<Candidate>| confirm(root, group, run, &sender);

            match pool() {
                Some(pool) => pool.install(|| groups.into_par_iter().for_each(work)),
                None => groups.into_iter().for_each(work),
            }
            drop(sender);
        });

        pump(&receiver, run, out, |group: Group, out| {
            out.group(group.size, group.hits, run)
        });
    });
}

/// The candidates of every size that has two or more files, hard links to one file counted once, the files of a group in
/// the order of their paths.
fn same_size(candidates: Vec<Candidate>) -> Vec<Vec<Candidate>> {
    let mut sizes: HashMap<u64, Vec<Candidate>> = HashMap::new();
    for candidate in candidates {
        sizes.entry(candidate.size).or_default().push(candidate);
    }

    sizes
        .into_values()
        .map(|mut group| {
            group.sort_by(|a, b| a.path.cmp(&b.path));

            let mut seen = HashSet::new();
            group.retain(|candidate| candidate.identity.is_none_or(|identity| seen.insert(identity)));

            group
        })
        .filter(|group| group.len() > 1)
        .collect()
}

/// A pool of its own for the hashing, so a slow disk cannot hold up anything else; `None` when its threads cannot be
/// started (the hashing then runs on one thread).
fn pool() -> Option<rayon::ThreadPool> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(HASH_THREADS)
        .thread_name(|index| format!("pairpane-hash-{index}"))
        .build()
        .ok()
}

/// Splits the files of one size by the hash of their start, then by the hash of all of them, and sends the groups
/// that remain.
fn confirm(root: &Path, group: Vec<Candidate>, run: &Run<'_>, sender: &Sender<Group>) {
    let size = group[0].size;
    let mut heads = by_hash(root, group, HASH_HEAD.min(size), run);

    heads.retain(|same| same.len() > 1);

    for same in heads {
        let twins = if size <= HASH_HEAD {
            vec![same]
        } else {
            by_hash(root, same, size, run)
        };

        for twins in twins.into_iter().filter(|twins| twins.len() > 1) {
            let hits = twins.into_iter().map(hit).collect();

            if sender.send(Group { size, hits }).is_err() {
                return;
            }
        }
    }
}

/// `files` split by the hash of their first `length` bytes. A file that cannot be read, or whose size is not the one the
/// walk saw, is counted as skipped and left out.
fn by_hash(root: &Path, files: Vec<Candidate>, length: u64, run: &Run<'_>) -> Vec<Vec<Candidate>> {
    let mut hashes: HashMap<[u8; 32], Vec<Candidate>> = HashMap::new();

    for file in files {
        if run.raised() {
            break;
        }

        match hash(root, &file, length, run) {
            Some(Some(hash)) => hashes.entry(hash).or_default().push(file),
            Some(None) => run.skip(),
            None => break,
        }
    }

    hashes.into_values().collect()
}

/// The BLAKE3 hash of the first `length` bytes of `file`: `Some(None)` when it cannot be read or is not the size it was,
/// `None` when the search stopped.
fn hash(root: &Path, file: &Candidate, length: u64, run: &Run<'_>) -> Option<Option<[u8; 32]>> {
    let Ok((open, metadata)) = open_listed(root, &file.path) else {
        return Some(None);
    };
    if metadata.len() != file.size {
        return Some(None);
    }

    read_hash(open, length, run)
}

fn read_hash(mut open: File, length: u64, run: &Run<'_>) -> Option<Option<[u8; 32]>> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; BUFFER];
    let mut left = length;

    while left > 0 {
        if run.raised() {
            return None;
        }

        let want = usize::try_from(left).map_or(BUFFER, |left| left.min(BUFFER));
        let count = match open.read(&mut buffer[..want]) {
            Ok(0) => return Some(None),
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Some(None),
        };

        hasher.update(&buffer[..count]);
        left -= count as u64;
        run.bytes.fetch_add(count as u64, Ordering::Relaxed);
    }

    Some(Some(*hasher.finalize().as_bytes()))
}

fn hit(file: Candidate) -> FindHit {
    FindHit {
        path: file.path,
        inner: None,
        kind: EntryKind::File,
        link: false,
        size: file.size,
        modified: file.modified,
        matches: None,
        line: None,
        line_number: None,
    }
}
