//! What a download does when the server does not behave: a reply smaller than was asked for (and what that costs), a read
//! that fails after some data, and a folder swapped for a link in the middle of a move.

use std::sync::atomic::{AtomicBool, Ordering};

use pretty_assertions::assert_eq;

use super::browse::connected;
use super::path;
use super::transfer_fs::pattern;
use crate::test_server::Settings;

#[test]
fn a_server_that_answers_with_little_does_not_cost_a_read_for_each_of_its_replies_to_a_window() {
    let (h, fs) = connected(Settings {
        short_reads: Some(4_000),
        ..Settings::default()
    });
    let content = pattern(400_000);

    h.server.put("small", &content);

    let mut reader = fs.open_read(&path("/small")).expect("opened");
    let mut buf = vec![0; 256 * 1024];
    let mut read = Vec::new();

    loop {
        let count = reader.read(&mut buf, &AtomicBool::new(false)).expect("read");

        if count == 0 {
            break;
        }

        read.extend_from_slice(&buf[..count]);
    }

    assert!(read == content);

    // A window asks for 16 reads, and one of them was of use before the size followed what the server sends: that would be
    // some 1,600 reads in all, against some 110 for a reader that follows it.
    let reads = h.server.counts.reads.load(Ordering::Relaxed);

    assert!(reads < 300, "{reads} reads for 100 replies");
}

#[test]
fn a_read_that_fails_after_some_data_gives_the_data_first_and_then_the_failure() {
    let (h, fs) = connected(Settings {
        fail_reads_from: Some(96 * 1024),
        ..Settings::default()
    });
    let content = pattern(300_000);

    h.server.put("broken", &content);

    let mut reader = fs.open_read(&path("/broken")).expect("opened");
    let mut buf = vec![0; 256 * 1024];
    let mut got = Vec::new();

    let error = loop {
        match reader.read(&mut buf, &AtomicBool::new(false)) {
            Ok(0) => panic!("the file was read to its end"),
            Ok(count) => got.extend_from_slice(&buf[..count]),
            Err(error) => break error,
        }
    };

    assert_eq!(error.kind(), std::io::ErrorKind::Other);
    assert!(
        got == content[..96 * 1024],
        "what came before the failure was given out, and no more"
    );
}

#[cfg(unix)]
#[test]
fn realpath_resolves_the_links_in_a_path_and_says_when_there_is_nothing() {
    let (h, fs) = connected(Settings::default());

    h.server.put("data/real/f", b"x");
    std::os::unix::fs::symlink(
        h.server.root.path().join("data/real"),
        h.server.root.path().join("data/link"),
    )
    .expect("link");

    assert_eq!(
        fs.realpath(&path("/data/link/f")).expect("resolved"),
        path("/data/real/f")
    );
    assert_eq!(fs.realpath(&path("/data/real")).expect("resolved"), path("/data/real"));
    assert_eq!(
        fs.realpath(&path("/data/nothing")).expect_err("nothing").kind(),
        std::io::ErrorKind::NotFound
    );
}

/// A move while the server swaps a folder for a link to another one, held back in the middle of a file for it.
#[cfg(unix)]
mod swap {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::Ordering;

    use pairpane_core::model::{OpKind, Outcome};
    use pretty_assertions::assert_eq;

    use super::super::down_rig::{Ended, Rig, set_times};
    use super::super::transfer_fs::pattern;
    use crate::test_server::Settings;

    const CHANGED: &str = "Kept the original: the folder on the server changed while it was moved";

    /// A server that holds READ back after ten of them.
    fn stalling() -> Rig {
        Rig::new(Settings {
            stall_reads_after: Some(10),
            ..Settings::default()
        })
    }

    /// Moves `names` of `data` while the server's folder `data/sub` becomes a link to `data/victim`, in the middle of the
    /// first file that is read.
    fn moved_during_swap(rig: &mut Rig, names: &[&str]) -> Ended {
        let job = rig.start(rig.request(OpKind::Move, "/data", names));

        rig.wait_for_reads(11);
        fs::remove_dir_all(rig.on_server("data/sub")).expect("removed");
        symlink(rig.on_server("data/victim"), rig.on_server("data/sub")).expect("link");
        rig.h.server.counts.release_reads();

        rig.ended(job)
    }

    #[test]
    fn a_folder_swapped_for_a_link_before_it_is_walked_is_not_followed() {
        let mut rig = stalling();

        rig.h.server.put("data/big.bin", &pattern(3_000_000));
        rig.h.server.put("data/sub/s.txt", b"s");
        rig.h.server.put("data/victim/v.txt", b"victim data");

        let ended = moved_during_swap(&mut rig, &["big.bin", "sub"]);

        assert_eq!(ended.outcome, Outcome::Done);
        assert_eq!(ended.failed(), ["sub"]);
        assert_eq!(
            fs::read(rig.on_server("data/victim/v.txt")).expect("file"),
            b"victim data"
        );
        assert_eq!(rig.local_tree(), ["big.bin"]);
        assert_eq!(
            rig.h.server.counts.removes.load(Ordering::Relaxed),
            1,
            "only the file that was moved"
        );
    }

    #[test]
    fn a_folder_swapped_for_a_link_while_a_file_is_read_keeps_that_original() {
        let mut rig = stalling();

        rig.h.server.put("data/sub/s.bin", &pattern(3_000_000));
        // Of the same size and time, so that nothing but where the folder is tells the two files apart.
        rig.h.server.put("data/victim/s.bin", &pattern(3_000_000));
        for name in ["data/sub/s.bin", "data/victim/s.bin"] {
            set_times(&rig.on_server(name));
        }

        let ended = moved_during_swap(&mut rig, &["sub"]);

        assert_eq!(ended.outcome, Outcome::Done);
        assert_eq!(ended.failed(), ["sub/s.bin"]);
        assert_eq!(ended.errors[0].error.message, CHANGED);
        assert!(
            rig.on_server("data/victim/s.bin").exists(),
            "the file behind the link is not removed"
        );
        assert_eq!(rig.h.server.counts.removes.load(Ordering::Relaxed), 0);
        assert!(fs::read(rig.local.path().join("sub/s.bin")).expect("file") == pattern(3_000_000));
    }
}
