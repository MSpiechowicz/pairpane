//! What a download needs of a server: `lstat`, a file opened for reading (in order, with short replies, cancelled while
//! the server is silent), removing, and what an open file does to the connection.

use std::io::ErrorKind;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use pairpane_core::vfs::{RemoteFs, RemoteKind, RemoteReader};
use pretty_assertions::assert_eq;

use super::browse::connected;
use super::{Harness, assert_fixed, path, quick};
use crate::test_server::Settings;
use crate::{AgentSource, Timeouts};

const PASSWORD: &str = "correct horse";

/// Bytes that differ from place to place, so that a chunk read from the wrong offset shows.
pub(super) fn pattern(length: usize) -> Vec<u8> {
    (0..length).map(|at| (at * 7 % 251) as u8).collect()
}

fn read_all(reader: &mut dyn RemoteReader, buffer: usize) -> Vec<u8> {
    let cancel = AtomicBool::new(false);
    let mut buf = vec![0; buffer];
    let mut content = Vec::new();

    loop {
        let count = reader.read(&mut buf, &cancel).expect("read");

        if count == 0 {
            return content;
        }

        content.extend_from_slice(&buf[..count]);
    }
}

fn opened(fs: &dyn RemoteFs, name: &str) -> Box<dyn RemoteReader> {
    fs.open_read(&path(name)).expect("opened")
}

/// Waits (a server's close is answered in the background) until the server holds `count` files open.
pub(super) fn wait_for_open(h: &Harness, count: usize) {
    let started = Instant::now();

    while h.server.counts.open_handles.load(Ordering::Relaxed) != count {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the server still has handles open"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_file_is_read_whole_and_in_order_over_several_windows() {
    let (h, fs) = connected(Settings::default());
    let content = pattern(700_001);

    h.server.put("data/big", &content);

    for buffer in [256 * 1024, 1000, 40_000] {
        assert!(
            read_all(&mut *opened(&*fs, "/data/big"), buffer) == content,
            "buffer {buffer}"
        );
    }
}

#[test]
fn files_of_awkward_sizes_are_read_whole() {
    let (h, fs) = connected(Settings::default());

    for (name, length) in [
        ("empty", 0),
        ("one", 1),
        ("chunk", 32 * 1024),
        ("two_chunks", 64 * 1024),
    ] {
        let content = pattern(length);

        h.server.put(name, &content);

        assert!(
            read_all(&mut *opened(&*fs, &format!("/{name}")), 100_000) == content,
            "{name}"
        );
    }
}

#[test]
fn replies_shorter_than_asked_lose_no_byte_and_repeat_none() {
    let (h, fs) = connected(Settings {
        short_reads: Some(5000),
        ..Settings::default()
    });
    let content = pattern(100_003);

    h.server.put("short", &content);

    assert!(read_all(&mut *opened(&*fs, "/short"), 64 * 1024) == content);
}

#[test]
fn a_read_waiting_for_a_silent_server_ends_when_cancelled() {
    let (h, fs) = connected(Settings {
        stall_reads: true,
        ..Settings::default()
    });

    h.server.put("stuck", &pattern(100_000));

    let mut reader = opened(&*fs, "/stuck");
    let cancel = Arc::new(AtomicBool::new(false));
    let setter = Arc::clone(&cancel);
    let started = Instant::now();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        setter.store(true, Ordering::Release);
    });

    let error = reader.read(&mut vec![0; 64 * 1024], &cancel).expect_err("cancelled");

    assert_eq!(error.kind(), ErrorKind::Interrupted);
    assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    assert_fixed(&error, h.server.addr);
    assert_eq!(
        reader.read(&mut [0; 8], &cancel).expect_err("still cancelled").kind(),
        ErrorKind::Interrupted
    );
}

#[test]
fn an_open_file_says_what_the_server_says_of_it() {
    let (h, fs) = connected(Settings::default());

    h.server.put("data/file", &pattern(12_345));

    let on_disk = std::fs::metadata(h.server.root.path().join("data/file")).expect("metadata");
    let seconds = on_disk
        .modified()
        .expect("a time")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after 1970")
        .as_secs() as i64;
    let meta = opened(&*fs, "/data/file").meta().expect("fstat");

    assert_eq!(
        (meta.kind, meta.size, meta.modified_ms),
        (RemoteKind::File, 12_345, Some(seconds * 1000))
    );
    assert_eq!(
        Some(meta),
        fs.lstat(&path("/data/file")).expect("lstat"),
        "the same as of the path"
    );
}

#[test]
fn a_file_that_is_not_there_or_is_not_a_file_does_not_open() {
    let (h, fs) = connected(Settings::default());

    h.server.put("data/file", b"x");

    let missing = fs.open_read(&path("/data/none")).err().expect("not opened");
    let folder = fs.open_read(&path("/data")).err().expect("not opened");

    assert_eq!(missing.kind(), ErrorKind::NotFound);
    assert_fixed(&missing, h.server.addr);
    assert_eq!(folder.kind(), ErrorKind::Other);
    assert_eq!(h.server.counts.open_handles.load(Ordering::Relaxed), 0);
}

#[test]
fn lstat_tells_files_folders_and_links_apart_and_none_for_nothing() {
    let (h, fs) = connected(Settings::default());

    h.server.put("data/file", b"abc");
    std::fs::create_dir(h.server.root.path().join("data/folder")).expect("folder");

    let kind = |name: &str| fs.lstat(&path(name)).expect("lstat").map(|meta| meta.kind);

    assert_eq!(kind("/data/file"), Some(RemoteKind::File));
    assert_eq!(kind("/data/folder"), Some(RemoteKind::Dir));
    assert_eq!(kind("/data/none"), None);

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("file", h.server.root.path().join("data/link")).expect("link");

        assert_eq!(kind("/data/link"), Some(RemoteKind::Link), "the link itself");
        assert_eq!(
            fs.stat(&path("/data/link")).expect("stat").map(|meta| meta.kind),
            Some(RemoteKind::File),
            "stat follows it"
        );
    }
}

#[test]
fn a_file_is_removed_once_and_a_folder_only_when_empty() {
    let (h, fs) = connected(Settings::default());
    let on_disk = |name: &str| h.server.root.path().join(name);

    h.server.put("data/file", b"abc");
    h.server.put("data/full/inner", b"abc");
    std::fs::create_dir(on_disk("data/empty")).expect("folder");

    fs.remove_file(&path("/data/file")).expect("removed");
    assert!(!on_disk("data/file").exists());
    assert_eq!(
        fs.remove_file(&path("/data/file")).expect_err("gone").kind(),
        ErrorKind::NotFound
    );

    assert!(fs.remove_file(&path("/data/empty")).is_err(), "not a file");
    assert!(on_disk("data/empty").exists());

    fs.remove_dir(&path("/data/empty")).expect("removed");
    assert!(!on_disk("data/empty").exists());

    let full = fs.remove_dir(&path("/data/full")).expect_err("not empty");

    assert_fixed(&full, h.server.addr);
    assert!(on_disk("data/full/inner").exists());
    assert!(fs.remove_dir(&path("/data/full/inner")).is_err(), "not a folder");
}

#[test]
fn a_dropped_reader_lets_the_server_close_its_handle() {
    let (h, fs) = connected(Settings::default());

    h.server.put("data/file", &pattern(100));

    let first = opened(&*fs, "/data/file");
    let second = opened(&*fs, "/data/file");

    assert_eq!(h.server.counts.open_handles.load(Ordering::Relaxed), 2);
    drop(first);
    wait_for_open(&h, 1);
    drop(second);
    wait_for_open(&h, 0);
}

#[test]
fn a_connection_with_an_open_file_is_not_closed_for_being_idle() {
    let timeouts = Timeouts {
        idle: Duration::from_millis(200),
        ..quick()
    };
    let h = Harness::with(Settings::default(), timeouts, AgentSource::None);
    let content = pattern(100_000);

    h.server.put("data/file", &content);

    let fs = h.pool.fs(h.site(Some(PASSWORD))).expect("a site");
    let mut reader = opened(&*fs, "/data/file");

    // As a paused transfer: several times the idle time, and no call.
    std::thread::sleep(Duration::from_millis(1_200));

    assert!(read_all(&mut *reader, 64 * 1024) == content);
    assert_eq!(h.server.counts.connections.load(Ordering::Relaxed), 1);

    drop(reader);
    wait_for_open(&h, 0);
    std::thread::sleep(Duration::from_millis(1_200));
    fs.lstat(&path("/data/file")).expect("lstat");

    assert_eq!(
        h.server.counts.connections.load(Ordering::Relaxed),
        2,
        "without the file the idle connection is closed, as always"
    );
}

#[test]
fn a_reader_on_a_lost_connection_fails_and_the_next_call_opens_a_new_one() {
    let (h, fs) = connected(Settings::default());
    let content = pattern(100_000);

    h.server.put("data/file", &content);

    let mut reader = opened(&*fs, "/data/file");
    let mut buf = vec![0; content.len()];

    assert_eq!(
        reader.read(&mut buf, &AtomicBool::new(false)).expect("read"),
        content.len()
    );

    h.server.drop_connections();

    let error = reader.read(&mut buf, &AtomicBool::new(false)).expect_err("lost");

    assert_eq!(error.kind(), ErrorKind::ConnectionAborted);
    assert_fixed(&error, h.server.addr);
    assert!(read_all(&mut *opened(&*fs, "/data/file"), 64 * 1024) == content);
    assert_eq!(h.server.counts.connections.load(Ordering::Relaxed), 2);
}

#[test]
fn a_change_that_met_a_lost_connection_is_not_repeated_on_a_new_one() {
    let (h, fs) = connected(Settings {
        stalled: vec!["/data/doomed".to_owned()],
        ..Settings::default()
    });

    h.server.put("data/doomed", b"abc");

    let removing = {
        let fs = Arc::clone(&fs);

        std::thread::spawn(move || fs.remove_file(&path("/data/doomed")))
    };
    let started = Instant::now();

    // The server has the request, and does not answer it.
    while h.server.counts.removes.load(Ordering::Relaxed) == 0 {
        assert!(started.elapsed() < Duration::from_secs(5), "the request did not arrive");
        std::thread::sleep(Duration::from_millis(10));
    }

    h.server.drop_connections();

    let error = removing.join().expect("joined").expect_err("lost");

    assert_eq!(error.kind(), ErrorKind::ConnectionAborted);
    assert_eq!(h.server.counts.removes.load(Ordering::Relaxed), 1, "asked once");
    assert_eq!(
        h.server.counts.connections.load(Ordering::Relaxed),
        1,
        "no new connection for it"
    );
    assert!(h.server.root.path().join("data/doomed").exists());
}
