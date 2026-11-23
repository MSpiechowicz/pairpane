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

/// An `ftyp` box (the start of a HEIF file) with the major brand `major`, minor version 0 and the compatible brands
/// `compatible`, followed by zeroes.
pub(super) fn ftyp(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&u32::try_from(16 + 4 * compatible.len()).expect("small").to_be_bytes());
    bytes.extend_from_slice(b"ftyp");
    bytes.extend_from_slice(major);
    bytes.extend_from_slice(&[0; 4]);
    for brand in compatible {
        bytes.extend_from_slice(*brand);
    }
    bytes.extend_from_slice(&[0; 40]);

    bytes
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
/// What a probe, the test program started again to confine itself and report (see [`run_probe`]), said and how it ended.
pub(crate) struct Probe {
    /// Whether the process ended with success.
    pub(crate) success: bool,
    /// What it wrote to its standard output and to its standard error, for the message of a failed assertion.
    pub(crate) output: String,
    /// The one line it reported after `PROBE `, or nothing.
    pub(crate) report: String,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
/// Starts this test program again to run the single test `test` as a probe: with the environment variable `variable`
/// set, the test is to confine its process and print one line that starts with `PROBE `. (Confining the process of the
/// test itself would confine the other tests with it.)
pub(crate) fn run_probe(variable: &str, test: &str) -> Probe {
    run_probe_with(variable, "1", test)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
/// Like [`run_probe`], with `value` for the variable, for a probe that is told something (a folder it is to use).
pub(crate) fn run_probe_with(variable: &str, value: &str, test: &str) -> Probe {
    let output = std::process::Command::new(std::env::current_exe().expect("the test program"))
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(variable, value)
        .output()
        .expect("the probe starts");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    // The report follows the name of the test on its line.
    let report = stdout
        .lines()
        .find_map(|line| line.split_once("PROBE "))
        .map_or(String::new(), |(_, report)| report.to_owned());

    Probe {
        success: output.status.success(),
        output: format!("{stdout} {}", String::from_utf8_lossy(&output.stderr)),
        report,
    }
}
