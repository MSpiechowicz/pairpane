//! The files the test server has open for reading: one handle each, counted in `counts.open_handles` while it lives.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use super::Counts;

/// An open file; it counts itself as a handle until it is dropped (closed, or its connection ended).
struct Open {
    file: File,
    counts: Arc<Counts>,
}

impl Drop for Open {
    fn drop(&mut self) {
        self.counts.open_handles.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct Table {
    files: HashMap<String, Open>,
    next: usize,
}

/// The open files of one connection.
#[derive(Clone, Default)]
pub(super) struct OpenFiles {
    table: Arc<Mutex<Table>>,
}

impl OpenFiles {
    /// Keeps `file` open; the handle it is known by.
    pub(super) fn add(&self, file: File, counts: &Arc<Counts>) -> String {
        let mut table = self.table.lock().expect("open files");

        table.next += 1;
        counts.open_handles.fetch_add(1, Ordering::Relaxed);

        let handle = format!("file-{}", table.next);

        table.files.insert(
            handle.clone(),
            Open {
                file,
                counts: Arc::clone(counts),
            },
        );

        handle
    }

    /// Up to `len` bytes of the file from `offset`; empty at its end, `None` for a handle that is not open.
    pub(super) fn read(&self, handle: &str, offset: u64, len: usize) -> Option<std::io::Result<Vec<u8>>> {
        let mut table = self.table.lock().expect("open files");
        let open = table.files.get_mut(handle)?;
        let mut data = Vec::new();

        let read = open
            .file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| (&open.file).take(len as u64).read_to_end(&mut data));

        Some(read.map(|_| data))
    }

    pub(super) fn metadata(&self, handle: &str) -> Option<std::io::Result<std::fs::Metadata>> {
        let table = self.table.lock().expect("open files");

        Some(table.files.get(handle)?.file.metadata())
    }

    /// Closes every file: the connection is over.
    pub(super) fn close_all(&self) {
        self.table.lock().expect("open files").files.clear();
    }

    /// Closes the file; whether it was open.
    pub(super) fn close(&self, handle: &str) -> bool {
        self.table.lock().expect("open files").files.remove(handle).is_some()
    }
}
