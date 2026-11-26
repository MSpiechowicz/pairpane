//! Tests of `remote_open`: the arguments it checks, a listing in its place and read-only, a later open of the pane
//! superseding it, what stops the pane's size jobs, a site that is gone, and the pane's listing cap. The file system is a
//! fake for what needs a server to stall; the rest lists a server in this process.

use std::io;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use pairpane_core::model::{EntryKind, PaneCapabilities, PaneLocation};
use pairpane_core::vfs::{RemoteEntry, RemoteMeta};
use tauri::async_runtime::block_on;

use super::super::LISTING_THREADS;
use super::*;
use crate::commands::archives::fixtures::collecting;
use crate::commands::remote::fixtures::Fixture;

const SITE: SiteId = SiteId(7);

/// A server that holds `read_dir` until `release` is raised, or, when `stoppable`, until the listing is cancelled.
struct Stalled {
    release: Arc<AtomicBool>,
    stoppable: bool,
}

impl RemoteFs for Stalled {
    fn site(&self) -> SiteId {
        SITE
    }

    fn label(&self) -> &str {
        "me@box"
    }

    fn read_dir(&self, _dir: &RemotePath, cancel: &AtomicBool) -> io::Result<Vec<RemoteEntry>> {
        while !self.release.load(Ordering::Relaxed) {
            if self.stoppable && cancel.load(Ordering::Relaxed) {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }

            std::thread::sleep(Duration::from_millis(5));
        }

        Ok(Vec::new())
    }

    fn stat(&self, _path: &RemotePath) -> io::Result<Option<RemoteMeta>> {
        Ok(None)
    }

    fn read_link(&self, _path: &RemotePath) -> io::Result<String> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

fn stalled(stoppable: bool) -> (Arc<AtomicBool>, Lookup) {
    let release = Arc::new(AtomicBool::new(false));
    let fs: Arc<dyn RemoteFs> = Arc::new(Stalled {
        release: Arc::clone(&release),
        stoppable,
    });

    (release, Box::new(move || Ok(fs)))
}

fn dir(text: &str) -> RemotePath {
    RemotePath::new(text).expect("a path")
}

/// Opens `path` of `site` in the left pane and waits for the listing's thread to end; the events it sent.
fn listed(panes: &Panes, path: &str, lookup: Lookup) -> Vec<ListEvent> {
    let (channel, events) = collecting();
    let handle = start(panes, PaneId::Left, SITE, dir(path), true, channel, lookup).expect("started");

    block_on(handle).expect("the thread ended");

    events.lock().expect("not poisoned").clone()
}

#[test]
fn the_folder_is_a_path_of_the_server_and_the_child_one_plain_name() {
    assert_eq!(target("/", None).expect("root"), RemotePath::root());
    assert_eq!(target("/srv/data", None).expect("a folder").as_str(), "/srv/data");
    assert_eq!(target("/srv", Some("data")).expect("a child").as_str(), "/srv/data");
    assert_eq!(target("/", Some(".config")).expect("a child").as_str(), "/.config");

    for bad in [
        "",
        "relative/dir",
        "/a/../b",
        "/a/./b",
        "/a//b",
        "/a/",
        "/nul\0",
        &format!("/{}", "x".repeat(5000)),
    ] {
        let error = target(bad, None).expect_err(bad);

        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{bad:?}");
    }

    for bad in ["", ".", "..", "a/b", "/etc", "a/", "nul\0"] {
        let error = target("/srv", Some(bad)).expect_err(bad);

        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{bad:?}");
    }
}

#[test]
fn a_server_folder_is_listed_in_its_place_read_only_and_with_the_servers_own_crumbs() {
    let fixture = Fixture::new();
    let site = fixture.site();
    let pool = fixture.remotes.pool().expect("a pool");
    fixture.server.put("data/notes.txt", b"twelve bytes");
    fixture.server.put("data/.hidden", b"");
    fixture.server.put("data/sub/inner.txt", b"x");

    let events = listed(&Panes::default(), "/data", Box::new(move || pool.fs(site)));

    let Some(ListEvent::Start {
        path,
        parent,
        crumbs,
        location,
        capabilities,
        total,
    }) = events.first()
    else {
        panic!("a start comes first: {events:?}");
    };
    assert_eq!((path.as_str(), parent.as_deref(), *total), ("/data", Some("/"), 3));
    assert_eq!(*capabilities, PaneCapabilities::READ_ONLY);
    assert!(
        matches!(location, PaneLocation::Remote { dir, .. } if dir == "/data"),
        "{location:?}"
    );
    assert_eq!(
        crumbs.iter().map(|crumb| crumb.path.as_str()).collect::<Vec<_>>(),
        ["/", "/data"]
    );

    let mut entries = Vec::new();
    for event in &events {
        if let ListEvent::Batch { entries: batch, .. } = event {
            entries.extend(batch.iter().map(|entry| (entry.name.clone(), entry.kind)));
        }
    }
    assert_eq!(
        entries,
        [
            ("sub".to_owned(), EntryKind::Dir),
            (".hidden".to_owned(), EntryKind::File),
            ("notes.txt".to_owned(), EntryKind::File)
        ]
    );
    assert!(matches!(events.last(), Some(ListEvent::Done { .. })), "{events:?}");
}

#[test]
fn a_site_that_is_gone_or_a_folder_that_is_not_there_is_an_error_event() {
    let fixture = Fixture::new();
    let site = fixture.site();
    let pool = fixture.remotes.pool().expect("a pool");
    let panes = Panes::default();

    let missing = {
        let pool = Arc::clone(&pool);

        listed(&panes, "/no/such/folder", Box::new(move || pool.fs(site)))
    };
    block_on(fixture.remotes.disconnect(site)).expect("closed");
    let gone = listed(&panes, "/", Box::new(move || pool.fs(site)));

    for events in [missing, gone] {
        assert!(
            matches!(events.as_slice(), [ListEvent::Error { .. }]),
            "only the error: {events:?}"
        );
    }
}

#[test]
fn a_later_open_of_the_pane_supersedes_the_listing_that_is_waiting_on_the_server() {
    let panes = Panes::default();
    let (_release, lookup) = stalled(true);
    let (channel, events) = collecting();
    let handle = start(&panes, PaneId::Left, SITE, dir("/slow"), false, channel, lookup).expect("started");

    // `pane_open` of the same pane takes the slot, which raises the flag of the earlier listing.
    panes
        .claim(
            PaneId::Left,
            SlotKey::Folder(PathBuf::from(if cfg!(windows) { "C:\\data" } else { "/data" })),
            &Channel::new(|_| Ok(())),
        )
        .expect("claimed");
    block_on(handle).expect("the thread ended");

    assert_eq!(
        events.lock().expect("not poisoned").clone(),
        Vec::new(),
        "it stops quietly"
    );
}

#[test]
fn listing_a_server_folder_stops_the_size_jobs_of_the_pane() {
    let panes = Panes::default();
    let folder = PathBuf::from(if cfg!(windows) { "C:\\data" } else { "/data" });
    let key = SlotKey::Folder(folder.clone());
    let first = panes
        .claim(PaneId::Left, key.clone(), &Channel::new(|_| Ok(())))
        .expect("claimed");
    panes.listed(PaneId::Left, first.generation, key)();
    let stop = panes
        .with_sizing(PaneId::Left, |sizing| {
            sizing.begin(folder.to_string_lossy().into_owned(), "photos")
        })
        .expect("a pane")
        .expect("a job")
        .stop;
    assert!(!stop.load(Ordering::Relaxed));

    let (release, lookup) = stalled(false);
    release.store(true, Ordering::Relaxed);
    listed(&panes, "/srv", lookup);

    assert!(
        stop.load(Ordering::Relaxed),
        "the server's folder has nothing to measure"
    );
}

/// Opens a folder of a server that never answers in `pane`: what lets it go, and the open.
fn hang(panes: &Panes, pane: PaneId) -> (Arc<AtomicBool>, CommandResult<JoinHandle<()>>) {
    let (release, lookup) = stalled(false);
    let opened = start(panes, pane, SITE, dir("/hung"), false, Channel::new(|_| Ok(())), lookup);

    (release, opened)
}

#[test]
fn a_pane_has_at_most_its_listing_threads_for_servers_that_do_not_answer() {
    let panes = Panes::default();
    let mut hung = Vec::new();

    for _ in 0..LISTING_THREADS {
        let (release, opened) = hang(&panes, PaneId::Left);

        hung.push((release, opened.expect("a place")));
    }

    let (_release, refused) = hang(&panes, PaneId::Left);
    let Err(refused) = refused else {
        panic!("the fifth open is refused");
    };
    assert_eq!(refused.code(), IpcErrorCode::Busy);

    let (release, other) = hang(&panes, PaneId::Right);
    assert!(other.is_ok(), "the other pane has its own places");
    hung.push((release, other.expect("a place")));

    for (release, handle) in hung {
        release.store(true, Ordering::Relaxed);
        block_on(handle).expect("the thread ended");
    }
}
