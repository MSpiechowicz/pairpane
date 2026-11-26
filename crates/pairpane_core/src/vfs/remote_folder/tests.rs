//! Tests of a folder on a server, against a server kept in memory.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;

use pretty_assertions::assert_eq;

use super::*;
use crate::model::SiteId;

#[path = "link_tests.rs"]
mod links;

const SITE: SiteId = SiteId(7);

fn path(text: &str) -> RemotePath {
    RemotePath::new(text).expect("valid path")
}

fn meta(kind: RemoteKind, size: u64, mode: u32) -> RemoteMeta {
    RemoteMeta {
        kind,
        size,
        modified_ms: Some(1_700_000_000_000),
        mode: Some(mode),
    }
}

fn item(name: &str, kind: RemoteKind) -> RemoteEntry {
    RemoteEntry {
        name: name.to_owned(),
        meta: meta(kind, 10, 0o644),
    }
}

/// A server with one folder's entries, the targets of its links, and a log of what was asked.
#[derive(Default)]
struct Server {
    entries: Vec<RemoteEntry>,
    /// What `stat` answers by the full path; a path that is not here does not exist.
    targets: HashMap<String, RemoteMeta>,
    /// What `read_dir` fails with.
    broken: Option<io::ErrorKind>,
    /// What `stat` fails with.
    stat_broken: Option<io::ErrorKind>,
    /// Raised by the first `stat`, as a user who moves on while the listing is busy.
    cancel_on_stat: Option<Arc<AtomicBool>>,
    stats: AtomicUsize,
    listed: Mutex<Vec<String>>,
}

impl RemoteFs for Server {
    fn site(&self) -> SiteId {
        SITE
    }

    fn label(&self) -> &str {
        "me@box"
    }

    fn read_dir(&self, dir: &RemotePath, cancel: &AtomicBool) -> io::Result<Vec<RemoteEntry>> {
        self.listed.lock().expect("log").push(dir.to_string());

        if cancel.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }

        match self.broken {
            Some(kind) => Err(kind.into()),
            None => Ok(self.entries.clone()),
        }
    }

    fn stat(&self, target: &RemotePath) -> io::Result<Option<RemoteMeta>> {
        self.stats.fetch_add(1, Ordering::Relaxed);

        if let Some(cancel) = &self.cancel_on_stat {
            cancel.store(true, Ordering::Relaxed);
        }

        match self.stat_broken {
            Some(kind) => Err(kind.into()),
            None => Ok(self.targets.get(target.as_str()).copied()),
        }
    }

    fn read_link(&self, _path: &RemotePath) -> io::Result<String> {
        panic!("a listing never reads where a link points");
    }
}

/// What a finished listing sent.
#[derive(Debug)]
struct Shown {
    events: Vec<ListEvent>,
}

impl Shown {
    fn start(&self) -> (&str, Option<&str>, &[Crumb], u32) {
        match self.events.first() {
            Some(ListEvent::Start {
                path,
                parent,
                crumbs,
                total,
                ..
            }) => (path, parent.as_deref(), crumbs, *total),
            other => panic!("start comes first: {other:?}"),
        }
    }

    fn entries(&self) -> Vec<&Entry> {
        let mut all = Vec::new();

        for event in &self.events {
            if let ListEvent::Batch { entries, .. } = event {
                all.extend(entries);
            }
        }

        all
    }

    fn names(&self) -> Vec<&str> {
        self.entries().iter().map(|entry| entry.name.as_str()).collect()
    }
}

fn show_in(server: impl Into<Arc<Server>>, dir: &str, show_hidden: bool) -> io::Result<Shown> {
    let folder = RemoteFolder::new(server.into(), path(dir));
    let mut events = Vec::new();
    let outcome = folder.list(show_hidden, &AtomicBool::new(false), &mut |event| events.push(event))?;

    assert_eq!(outcome, Outcome::Done);

    Ok(Shown { events })
}

fn show(entries: Vec<RemoteEntry>, show_hidden: bool) -> Shown {
    let server = Server {
        entries,
        ..Server::default()
    };

    show_in(server, "/srv", show_hidden).expect("listed")
}

#[test]
fn a_folder_says_where_it_is_and_cannot_be_written() {
    let server = Arc::new(Server {
        entries: vec![item("a.txt", RemoteKind::File), item("docs", RemoteKind::Dir)],
        ..Server::default()
    });
    let folder = RemoteFolder::new(server.clone(), path("/home/me"));
    let mut events = Vec::new();

    let outcome = folder
        .list(false, &AtomicBool::new(false), &mut |event| events.push(event))
        .expect("listed");

    let place = PaneLocation::Remote {
        site: SITE,
        label: "me@box".to_owned(),
        dir: "/home/me".to_owned(),
    };
    assert_eq!(outcome, Outcome::Done);
    assert_eq!(folder.location(), place);
    assert_eq!(folder.capabilities(), PaneCapabilities::READ_ONLY);
    assert!(folder.watch(Box::new(|| {})).is_none());
    assert_eq!(*server.listed.lock().expect("log"), ["/home/me"]);

    let ListEvent::Start {
        path,
        parent,
        total,
        location,
        capabilities,
        ..
    } = &events[0]
    else {
        panic!("start comes first: {events:?}");
    };
    assert_eq!(
        (path.as_str(), parent.as_deref(), *total),
        ("/home/me", Some("/home"), 2)
    );
    assert_eq!((location, *capabilities), (&place, PaneCapabilities::READ_ONLY));
    assert!(matches!(
        events.last(),
        Some(ListEvent::Done {
            dirs: 1,
            files: 1,
            bytes: 10,
            free: None,
            ..
        })
    ));
}

#[test]
fn the_crumbs_are_the_server_then_each_folder_with_its_path_on_the_server() {
    let shown = show_in(Server::default(), "/home/me/My Docs", false).expect("listed");

    let (_, _, crumbs, _) = shown.start();

    let seen: Vec<_> = crumbs
        .iter()
        .map(|crumb| (crumb.name.as_str(), crumb.path.as_str(), crumb.inside.as_deref()))
        .collect();
    assert_eq!(
        seen,
        [
            ("me@box", "/", None),
            ("home", "/home", None),
            ("me", "/home/me", None),
            ("My Docs", "/home/me/My Docs", None),
        ]
    );
}

#[test]
fn the_root_has_no_parent_and_only_the_server_for_a_crumb() {
    let shown = show_in(Server::default(), "/", false).expect("listed");

    let (dir, parent, crumbs, total) = shown.start();

    assert_eq!((dir, parent, total), ("/", None, 0));
    assert_eq!(crumbs.len(), 1);
    assert_eq!((crumbs[0].name.as_str(), crumbs[0].path.as_str()), ("me@box", "/"));
}

#[test]
fn a_folder_directly_below_the_root_goes_up_to_the_root() {
    let shown = show_in(Server::default(), "/srv", false).expect("listed");

    assert_eq!(shown.start().1, Some("/"));
}

#[test]
fn hidden_names_show_only_when_asked_for() {
    let listed = || vec![item(".profile", RemoteKind::File), item("notes", RemoteKind::File)];

    let without = show(listed(), false);
    let with = show(listed(), true);

    assert_eq!(without.names(), ["notes"]);
    assert_eq!(without.start().3, 1);
    assert_eq!(with.names(), [".profile", "notes"]);
    assert!(with.entries()[0].hidden);
    assert!(!with.entries()[1].hidden);
}

#[test]
fn entries_are_sorted_folders_first_then_by_name() {
    let shown = show(
        vec![
            item("b.txt", RemoteKind::File),
            item("z", RemoteKind::Dir),
            item("a.txt", RemoteKind::File),
            item("m", RemoteKind::Dir),
        ],
        false,
    );

    assert_eq!(shown.names(), ["m", "z", "a.txt", "b.txt"]);
}

#[test]
fn names_a_pane_could_not_open_safely_are_dropped() {
    let shown = show(
        vec![
            item(".", RemoteKind::Dir),
            item("..", RemoteKind::Dir),
            item("", RemoteKind::File),
            item("../etc", RemoteKind::File),
            item("a/b", RemoteKind::File),
            item("nul\0name", RemoteKind::File),
            item("fine", RemoteKind::File),
        ],
        true,
    );

    assert_eq!(shown.names(), ["fine"]);
    assert_eq!(shown.start().3, 1);
}

#[test]
fn an_entry_shows_what_the_server_said_about_it() {
    let mut locked = item("locked", RemoteKind::File);
    locked.meta = RemoteMeta {
        size: 42,
        mode: Some(0o100_444),
        ..locked.meta
    };
    let mut unknown = item("unknown", RemoteKind::File);
    unknown.meta.mode = None;
    unknown.meta.modified_ms = None;
    let mut folder = item("folder", RemoteKind::Dir);
    folder.meta = meta(RemoteKind::Dir, 4096, 0o755);
    let socket = item("socket", RemoteKind::Other);

    let shown = show(vec![locked, unknown, folder, socket], false);

    let by_name: HashMap<_, _> = shown
        .entries()
        .into_iter()
        .map(|entry| (entry.name.as_str(), entry))
        .collect();
    let locked = by_name["locked"];
    assert_eq!((locked.kind, locked.size, locked.readonly), (EntryKind::File, 42, true));
    assert_eq!((locked.mode, locked.modified), (Some(0o444), Some(1_700_000_000_000)));
    let unknown = by_name["unknown"];
    assert_eq!((unknown.readonly, unknown.mode, unknown.modified), (false, None, None));
    let folder = by_name["folder"];
    assert_eq!((folder.kind, folder.size, folder.readonly), (EntryKind::Dir, 0, false));
    assert_eq!(by_name["socket"].kind, EntryKind::Other);
    assert!(by_name.values().all(|entry| entry.link_target.is_none()));
}

#[test]
fn the_write_bit_of_the_owner_alone_decides_read_only() {
    let with_mode = |name: &str, mode| {
        let mut entry = item(name, RemoteKind::File);
        entry.meta.mode = Some(mode);
        entry
    };

    let shown = show(
        vec![
            with_mode("owner", 0o200),
            with_mode("group", 0o020),
            with_mode("all", 0o666),
            with_mode("none", 0),
        ],
        false,
    );

    let read_only: Vec<_> = shown
        .entries()
        .into_iter()
        .map(|entry| (entry.name.as_str(), entry.readonly))
        .collect();
    assert_eq!(
        read_only,
        [("all", false), ("group", true), ("none", true), ("owner", false)]
    );
}

#[test]
fn a_folder_the_server_cannot_list_fails_before_any_event() {
    for kind in [
        io::ErrorKind::NotFound,
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::TimedOut,
    ] {
        let server = Server {
            entries: vec![item("a", RemoteKind::File)],
            broken: Some(kind),
            ..Server::default()
        };
        let folder = RemoteFolder::new(Arc::new(server), path("/srv"));
        let mut events = Vec::new();

        let error = folder
            .list(false, &AtomicBool::new(false), &mut |event| events.push(event))
            .expect_err("not listed");

        assert_eq!(error.kind(), kind);
        assert!(events.is_empty(), "{events:?}");
    }
}

#[test]
fn a_cancelled_listing_sends_nothing_and_no_done() {
    let folder = RemoteFolder::new(Arc::new(Server::default()), path("/srv"));
    let mut events = Vec::new();

    let outcome = folder
        .list(false, &AtomicBool::new(true), &mut |event| events.push(event))
        .expect("not an error");

    assert_eq!(outcome, Outcome::Cancelled);
    assert!(events.is_empty(), "{events:?}");
}

#[test]
fn a_listing_cancelled_while_links_are_looked_at_stops_there() {
    let cancel = Arc::new(AtomicBool::new(false));
    let server = Arc::new(Server {
        entries: vec![item("one", RemoteKind::Link), item("two", RemoteKind::Link)],
        cancel_on_stat: Some(cancel.clone()),
        ..Server::default()
    });
    let folder = RemoteFolder::new(server.clone(), path("/srv"));
    let mut events = Vec::new();

    let outcome = folder
        .list(false, &cancel, &mut |event| events.push(event))
        .expect("not an error");

    assert_eq!(outcome, Outcome::Cancelled);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(server.stats.load(Ordering::Relaxed), 1);
}

#[test]
fn an_interrupted_read_that_nobody_cancelled_is_an_error() {
    let server = Server {
        broken: Some(io::ErrorKind::Interrupted),
        ..Server::default()
    };

    let error = show_in(server, "/srv", false).expect_err("failed");

    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}
