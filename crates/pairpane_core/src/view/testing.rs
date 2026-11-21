//! Helpers shared by the tests of the viewer: files on disk opened the way the viewer opens them.

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use tempfile::TempDir;

use crate::model::{TextQuery, TextWindow};

use super::{Opened, ViewFile, open, text_window};

/// A cancel flag that is never raised.
pub(super) fn idle() -> AtomicBool {
    AtomicBool::new(false)
}

pub(crate) fn temp() -> TempDir {
    tempfile::tempdir().expect("a temporary folder")
}

pub(crate) fn put(folder: &Path, name: &str, bytes: &[u8]) {
    fs::write(folder.join(name), bytes).expect("write");
}

/// A folder holding the file `name` with `bytes`, and the file opened through [`open`].
pub(super) fn opened(name: &str, bytes: &[u8]) -> (TempDir, Opened) {
    let folder = temp();
    put(folder.path(), name, bytes);
    let opened = open(folder.path(), name).expect("open");

    (folder, opened)
}

/// Just the open file.
pub(super) fn file_of(bytes: &[u8]) -> (TempDir, ViewFile) {
    let (folder, opened) = opened("file.bin", bytes);

    (folder, opened.file)
}

pub(super) fn query(encoding: &str, offset: u64, backward: bool, lines: u32, snap: bool) -> TextQuery {
    TextQuery {
        encoding: encoding.to_owned(),
        offset,
        backward,
        lines,
        snap,
    }
}

/// A window in UTF-8 that does not snap.
pub(super) fn window(file: &ViewFile, offset: u64, backward: bool, lines: u32) -> TextWindow {
    text_window(file, &query("UTF-8", offset, backward, lines, false)).expect("a window")
}

/// The texts of the lines of a window.
pub(super) fn texts(window: &TextWindow) -> Vec<&str> {
    window.lines.iter().map(|line| line.text.as_str()).collect()
}

/// Makes the pipe `name` in `folder`, which an ordinary open waits on for ever until someone writes to it.
#[cfg(unix)]
pub(crate) fn fifo(folder: &Path, name: &str) {
    let status = std::process::Command::new("mkfifo")
        .arg(folder.join(name))
        .status()
        .expect("mkfifo runs");
    assert!(status.success());
}
