//! (Unix only: they use symbolic links.)
//!
//! A move through a link a server cannot see through, a cancel right after a removal that went elsewhere, and a server
//! that answers fewer looks than it was asked.

use std::fs;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use pretty_assertions::assert_eq;

use crate::model::{OpKind, Outcome, SiteId};
use crate::vfs::{RemoteEntry, RemoteFs, RemoteMeta, RemotePath, RemoteReader};

use super::super::Request;
use super::super::testing::{Rig, long_ago, read, set_times, temp, tree};
use super::down_chain_tests::{cancelled_at, gate};
use super::down_swap_tests::{swap_for_victim, with_victim};
use super::testing::{FOLDER_CHANGED, Fake, REMOVED_ELSEWHERE, download, failure, said};

const NOT_SAID: &str =
    "this server does not say where a linked folder really is, so its items cannot be moved through a link";

/// A server with `s.txt` in `x/real/` and in `victim/`, alike; `x/real` is what the swap of a test takes the place of.
fn with_target() -> Arc<Fake> {
    let server = Fake::new();

    server.put("x/real/s.txt", b"s");
    server.put("victim/s.txt", b"V");
    for path in ["x/real/s.txt", "victim/s.txt"] {
        set_times(&server.at(path), long_ago());
    }

    server
}

/// The first read of a file moves `x/real` aside (as `x/real.real`) and puts a link to `victim` in its place.
fn swap_target(server: &Fake) {
    let swap = swap_for_victim(server, "x/real");

    server.switch(|switches| switches.at_first_read = Some(swap));
}

#[test]
fn a_move_through_a_link_is_refused_by_a_server_that_does_not_resolve_links_so_the_target_of_the_link_cannot_be_swapped()
 {
    for relative in [false, true] {
        let server = with_target();

        if relative {
            fs::create_dir(server.at("p")).expect("folder");
            std::os::unix::fs::symlink("../x/real", server.at("p/chosen")).expect("link");
        } else {
            std::os::unix::fs::symlink(server.at("x/real"), server.at("chosen")).expect("link");
        }
        server.switch(|switches| switches.lexical_realpath = true);
        swap_target(&server);
        let dir = if relative { "/p/chosen" } else { "/chosen" };
        let local = temp();

        let ended = Rig::new().run(download(OpKind::Move, &server, dir, &["s.txt"], local.path()));

        assert_eq!(failure(&ended), NOT_SAID, "relative: {relative}");
        assert_eq!(read(&server.at("victim/s.txt")), b"V", "relative: {relative}");
        assert_eq!(read(&server.at("x/real/s.txt")), b"s", "relative: {relative}");
        assert_eq!(tree(local.path()), Vec::<String>::new(), "relative: {relative}");
    }
}

#[test]
fn a_copy_through_a_link_is_not_refused_by_a_server_that_does_not_resolve_links() {
    let server = with_target();

    std::os::unix::fs::symlink(server.at("x/real"), server.at("chosen")).expect("link");
    server.switch(|switches| switches.lexical_realpath = true);
    let local = temp();

    let ended = Rig::new().run(download(OpKind::Copy, &server, "/chosen", &["s.txt"], local.path()));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(tree(local.path()), ["s.txt"]);
}

#[test]
fn the_target_of_a_link_the_server_resolves_swapped_for_a_link_during_the_job_keeps_the_originals() {
    let server = with_target();
    let local = temp();

    std::os::unix::fs::symlink(server.at("x/real"), server.at("chosen")).expect("link");
    swap_target(&server);

    let moved = Rig::new().run(download(OpKind::Move, &server, "/chosen", &["s.txt"], local.path()));

    assert_eq!(read(&server.at("victim/s.txt")), b"V");
    assert_eq!(read(&server.at("x/real.real/s.txt")), b"s");
    assert_eq!(said(&moved), [FOLDER_CHANGED]);
}

#[test]
fn a_cancel_right_after_a_removal_that_went_to_another_folder_still_reports_it() {
    let server = with_victim();
    let (hook, stopped, go) = gate();

    // The folder is as it was when it is checked, and is a link by the time the server removes the file.
    server.switch(|switches| {
        switches.lexical_realpath = true;
        switches.after_realpath = Some(("/sub".to_owned(), swap_for_victim(&server, "sub")));
        switches.after_remove = Some(hook);
    });

    let (ended, _local) = cancelled_at(&server, "sub", &stopped, &go);

    assert_eq!(ended.outcome, Outcome::Cancelled);
    assert_eq!(said(&ended), [REMOVED_ELSEWHERE]);
}

/// A server that answers the first `honest` batches of looks as it should, and every one after with no answer at all.
struct Short {
    inner: Arc<Fake>,
    honest: AtomicUsize,
}

impl RemoteFs for Short {
    fn site(&self) -> SiteId {
        self.inner.site()
    }

    fn label(&self) -> &str {
        "short"
    }

    fn read_dir(&self, dir: &RemotePath, cancel: &AtomicBool) -> io::Result<Vec<RemoteEntry>> {
        self.inner.read_dir(dir, cancel)
    }

    fn stat(&self, path: &RemotePath) -> io::Result<Option<RemoteMeta>> {
        self.inner.stat(path)
    }

    fn read_link(&self, path: &RemotePath) -> io::Result<String> {
        self.inner.read_link(path)
    }

    fn lstat(&self, path: &RemotePath) -> io::Result<Option<RemoteMeta>> {
        self.inner.lstat(path)
    }

    fn lstat_many(&self, paths: &[RemotePath], cancel: &AtomicBool) -> Vec<io::Result<Option<RemoteMeta>>> {
        let honest = self
            .honest
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1))
            .is_ok();

        if honest {
            self.inner.lstat_many(paths, cancel)
        } else {
            Vec::new()
        }
    }

    fn realpath(&self, path: &RemotePath) -> io::Result<RemotePath> {
        self.inner.realpath(path)
    }

    fn open_read(&self, path: &RemotePath) -> io::Result<Box<dyn RemoteReader>> {
        self.inner.open_read(path)
    }

    fn remove_file(&self, path: &RemotePath) -> io::Result<()> {
        self.inner.remove_file(path)
    }

    fn remove_dir(&self, path: &RemotePath) -> io::Result<()> {
        self.inner.remove_dir(path)
    }
}

/// Moves `s.txt` from `sub` of a server that answers `honest` batches of looks right, and then none, and has `sub` swapped for a link
/// to `victim` by the first read of a file.
fn move_from_short(honest: usize) -> (Arc<Fake>, crate::ops::testing::Ended) {
    let server = with_victim();

    server.switch(|switches| {
        switches.lexical_realpath = true;
        switches.at_first_read = Some(swap_for_victim(&server, "sub"));
    });
    let local = temp();
    let short = Arc::new(Short {
        inner: Arc::clone(&server),
        honest: AtomicUsize::new(honest),
    });
    let request = Request::download(
        OpKind::Move,
        short,
        RemotePath::new("/sub").expect("path"),
        vec!["s.txt".to_owned()],
        local.path().to_owned(),
        false,
    );

    (server, Rig::new().run(request))
}

#[test]
fn a_server_that_answers_fewer_looks_than_asked_when_the_move_starts_refuses_it() {
    let (server, ended) = move_from_short(0);

    assert_eq!(
        failure(&ended),
        "the server answered a different number of looks than were asked"
    );
    assert_eq!(read(&server.at("sub/s.txt")), b"s");
    assert_eq!(read(&server.at("victim/s.txt")), b"V");
}

#[test]
fn a_server_that_answers_fewer_looks_than_asked_before_a_removal_has_it_kept() {
    let (server, ended) = move_from_short(1);

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(
        said(&ended)[0].ends_with("the server answered a different number of looks than were asked"),
        "{:?}",
        said(&ended)
    );
    assert_eq!(read(&server.at("victim/s.txt")), b"V", "what the link leads to stays");
    assert_eq!(read(&server.at("sub.real/s.txt")), b"s", "so does the original");
}
