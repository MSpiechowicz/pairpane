//! Comparing two files by content: both are read in step, a chunk at a time, and the first chunk that differs ends it, so
//! files that differ early cost almost nothing and equal ones cost one read of each. No hash is needed: the bytes
//! themselves are compared. A cancel is heard between chunks.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::ops::open_listed;

use super::out::Scan;

/// The bytes one read of each file takes.
const CHUNK: usize = 128 * 1024;

impl Scan<'_> {
    /// Whether the file `left_name` of `left` and the file `right_name` of `right` (both of `size` bytes as the walk saw)
    /// hold the same bytes. `None` when that cannot be told: a file cannot be read or is not the size it was (counted as
    /// skipped), or the comparison was stopped.
    pub(super) fn same_content(&mut self, left: (&Path, &str), right: (&Path, &str), size: u64) -> Option<bool> {
        let opened = open(left, size).and_then(|one| open(right, size).map(|other| (one, other)));
        let Ok((mut one, mut other)) = opened else {
            self.skipped += 1;

            return None;
        };

        let (mut first, mut second) = (vec![0_u8; CHUNK], vec![0_u8; CHUNK]);

        loop {
            if self.halted() {
                return None;
            }

            let (read_one, read_other) = match (fill(&mut one, &mut first), fill(&mut other, &mut second)) {
                (Ok(one), Ok(other)) => (one, other),
                _ => {
                    self.skipped += 1;

                    return None;
                }
            };
            self.bytes = self.bytes.saturating_add((read_one + read_other) as u64);
            self.tick();

            if first[..read_one] != second[..read_other] {
                return Some(false);
            }
            if read_one < CHUNK {
                return Some(true);
            }
        }
    }
}

/// Opens `name` in `folder` (a regular file, no link followed) when it is `size` bytes.
fn open((folder, name): (&Path, &str), size: u64) -> io::Result<File> {
    let (file, metadata) = open_listed(folder, name)?;

    if metadata.len() != size {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }

    Ok(file)
}

/// Reads into `buffer` until it is full or `file` ends; how many bytes there are.
fn fill(file: &mut File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;

    while filled < buffer.len() {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }

    Ok(filled)
}
