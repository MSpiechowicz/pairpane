//! The next or previous byte where two open files differ, for the hex diff: both files are read at offsets in chunks of
//! [`DIFF_CHUNK`] bytes and compared, so a huge pair is never held and a cancel is seen between chunks.

use std::io;

use crate::model::ByteDifference;

use super::cancelled;
use super::file::ViewFile;

/// The bytes of each file that one step reads and compares before it looks at the cancel flag.
pub const DIFF_CHUNK: usize = 256 * 1024;

/// The most bytes of each file one call of [`next_difference`] is given to scan; past that it answers `Searching`.
pub const DIFF_BUDGET: u64 = 64 * 1024 * 1024;

/// The first offset at or after `from` where `left` and `right` differ (going forward), or the last one before `from`
/// (backward). An offset that only the longer file has counts as a difference, so every offset in
/// `[shorter size, longer size)` does. After `budget` bytes of each file were scanned without an answer it returns
/// `Searching` with the offset to ask again from (in either direction). `cancel` is looked at before each chunk, and a
/// cancelled scan ends with an `Interrupted` error.
pub fn next_difference(
    left: &ViewFile,
    right: &ViewFile,
    from: u64,
    forward: bool,
    budget: u64,
    cancel: &dyn Fn() -> bool,
) -> io::Result<ByteDifference> {
    let (left_size, right_size) = (left.size()?, right.size()?);
    let (shorter, longer) = (left_size.min(right_size), left_size.max(right_size));
    let mut scanned = 0;

    if forward {
        let mut at = from;

        loop {
            if at >= longer {
                return Ok(ByteDifference::None);
            }

            if at >= shorter {
                return Ok(ByteDifference::Found { offset: at });
            }

            if cancel() {
                return Err(cancelled());
            }

            if scanned >= budget {
                return Ok(ByteDifference::Searching { from: at });
            }

            let length = (shorter - at).min(DIFF_CHUNK as u64) as usize;
            let (a, b) = (left.read_at(at, length)?, right.read_at(at, length)?);

            if let Some(found) = mismatch(&a, &b, length, true) {
                return Ok(ByteDifference::Found {
                    offset: at + found as u64,
                });
            }

            at += length as u64;
            scanned += length as u64;
        }
    }

    let mut end = from.min(longer);

    loop {
        if end == 0 {
            return Ok(ByteDifference::None);
        }

        if end > shorter {
            return Ok(ByteDifference::Found { offset: end - 1 });
        }

        if cancel() {
            return Err(cancelled());
        }

        if scanned >= budget {
            return Ok(ByteDifference::Searching { from: end });
        }

        let start = end.saturating_sub(DIFF_CHUNK as u64);
        let length = (end - start) as usize;
        let (a, b) = (left.read_at(start, length)?, right.read_at(start, length)?);

        if let Some(found) = mismatch(&a, &b, length, false) {
            return Ok(ByteDifference::Found {
                offset: start + found as u64,
            });
        }

        end = start;
        scanned += length as u64;
    }
}

/// Where in a chunk of `length` bytes the two reads first (or, not `forward`, last) differ. A read that came back
/// short (a file that shrank meanwhile) differs from the other at the bytes it lacks.
fn mismatch(a: &[u8], b: &[u8], length: usize, forward: bool) -> Option<usize> {
    if a.len() == length && b.len() == length {
        if a == b {
            return None;
        }

        let mut pairs = a.iter().zip(b).enumerate().filter(|(_, (x, y))| x != y);

        return if forward { pairs.next() } else { pairs.next_back() }.map(|(at, _)| at);
    }

    // At least one read is short: the common part is compared, and what is missing from either differs.
    let common = a.len().min(b.len());
    let shared = mismatch(&a[..common], &b[..common], common, forward);

    match (forward, shared) {
        (true, Some(at)) => Some(at),
        (true, None) => Some(common),
        (false, _) if common < length => Some(length - 1),
        (false, shared) => shared,
    }
}
