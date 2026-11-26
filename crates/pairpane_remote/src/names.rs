//! The names of one listing, as far as they are taken: a server can send anything, so a name that could not be used is
//! left out and the total is capped.

use std::io;

use pairpane_core::vfs::{RemoteEntry, RemoteMeta};

use crate::error::fail;

/// The longest name taken (a file system's own limit is 255 bytes; this leaves room for any encoding of it).
pub(crate) const MAX_NAME: usize = 1024;

/// The most bytes of names one listing holds.
pub(crate) const MAX_NAME_BYTES: usize = 64 * 1024 * 1024;

/// What `char::REPLACEMENT_CHARACTER` is in a name that [`russh_sftp`] had to decode lossily. A name that holds a literal
/// U+FFFD (valid UTF-8, and rare) cannot be told apart from a lossy decode, so it is left out of the listing too.
const LOSSY: char = '\u{FFFD}';

/// The entries of a listing so far.
pub(crate) struct Collected {
    entries: Vec<RemoteEntry>,
    bytes: usize,
    max_bytes: usize,
}

impl Collected {
    pub(crate) fn new(max_bytes: usize) -> Self {
        Self {
            entries: Vec::new(),
            bytes: 0,
            max_bytes,
        }
    }

    /// Takes the entry, unless it is `.` or `..`, its name is longer than [`MAX_NAME`], or its name was not valid UTF-8
    /// (the SFTP layer replaces what it cannot decode, so the name that is left could not be opened). An error when the
    /// names of the listing add up to more than the most.
    pub(crate) fn add(&mut self, name: String, meta: RemoteMeta) -> io::Result<()> {
        if name == "." || name == ".." || name.len() > MAX_NAME || name.contains(LOSSY) {
            return Ok(());
        }

        self.bytes += name.len();

        if self.bytes > self.max_bytes {
            return Err(fail(io::ErrorKind::Other, "the folder is too large to list"));
        }

        self.entries.push(RemoteEntry { name, meta });

        Ok(())
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn into_entries(self) -> Vec<RemoteEntry> {
        self.entries
    }
}

#[cfg(test)]
mod tests {
    use pairpane_core::vfs::RemoteKind;
    use pretty_assertions::assert_eq;

    use super::*;

    fn meta() -> RemoteMeta {
        RemoteMeta {
            kind: RemoteKind::File,
            size: 0,
            modified_ms: None,
            mode: None,
        }
    }

    fn names(collected: Collected) -> Vec<String> {
        collected.into_entries().into_iter().map(|entry| entry.name).collect()
    }

    #[test]
    fn names_that_cannot_be_used_are_left_out() {
        let mut collected = Collected::new(MAX_NAME_BYTES);

        for name in [
            "kept".to_owned(),
            ".".to_owned(),
            "..".to_owned(),
            "bad\u{FFFD}name".to_owned(),
            "x".repeat(MAX_NAME + 1),
            "x".repeat(MAX_NAME),
        ] {
            collected.add(name, meta()).expect("taken or left out");
        }

        assert_eq!(names(collected), ["kept".to_owned(), "x".repeat(MAX_NAME)]);
    }

    #[test]
    fn the_names_of_a_listing_add_up_to_a_most() {
        let mut collected = Collected::new(10);

        collected.add("12345".to_owned(), meta()).expect("within");
        collected.add("67890".to_owned(), meta()).expect("exactly the most");

        let error = collected.add("1".to_owned(), meta()).expect_err("over");

        assert_eq!(error.to_string(), "the folder is too large to list");
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn a_name_that_is_left_out_does_not_count() {
        let mut collected = Collected::new(4);

        collected.add("x".repeat(MAX_NAME + 1), meta()).expect("left out");
        collected.add("bad\u{FFFD}".to_owned(), meta()).expect("left out");
        collected.add("four".to_owned(), meta()).expect("within");
    }
}
