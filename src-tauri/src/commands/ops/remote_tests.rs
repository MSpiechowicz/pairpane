//! Tests of `ops_remote`: what is refused before any job exists, and a download from a server in this process.

use pairpane_core::model::{IpcErrorCode, OpKind, Outcome, SiteId, TransferPlace};
use tauri::async_runtime::block_on;

use super::tests::{added_job, running, wait_for_end};
use super::*;
use crate::commands::remote::Remotes;
use crate::commands::remote::fixtures::{Fixture, without_pool};
use crate::commands::testing::{elsewhere, names, root};

fn folder(path: &str) -> TransferPlace {
    TransferPlace::Folder { path: path.to_owned() }
}

fn server(site: SiteId, dir: &str) -> TransferPlace {
    TransferPlace::Remote {
        site,
        dir: dir.to_owned(),
    }
}

/// `ops_remote` as the page calls it, on the fixture's remote state.
fn send(
    ops: &Ops,
    remotes: &Remotes,
    kind: OpKind,
    from: TransferPlace,
    to: TransferPlace,
    list: &[&str],
) -> CommandResult<JobId> {
    block_on(ops.remote(remotes, kind, from, to, names(list), false))
}

fn refused(result: CommandResult<JobId>) -> IpcErrorCode {
    result.expect_err("refused").code()
}

#[test]
fn only_a_copy_or_a_move_goes_between_a_server_and_this_computer() {
    let (ops, _target, _events) = running();
    let fixture = Fixture::new();
    let site = fixture.site();

    for kind in [OpKind::Rename, OpKind::Pack, OpKind::Sync, OpKind::Extract] {
        let result = send(&ops, &fixture.remotes, kind, server(site, "/"), folder(&root()), &["a"]);

        assert_eq!(refused(result), IpcErrorCode::InvalidInput, "{kind:?}");
    }
}

#[test]
fn the_places_decide_the_direction_and_only_a_download_is_accepted() {
    let (ops, _target, _events) = running();
    let fixture = Fixture::new();
    let site = fixture.site();
    let go = |from, to| send(&ops, &fixture.remotes, OpKind::Copy, from, to, &["a"]);

    assert_eq!(
        refused(go(folder(&root()), folder(&elsewhere()))),
        IpcErrorCode::InvalidInput
    );
    assert_eq!(
        refused(go(folder(&root()), server(site, "/"))),
        IpcErrorCode::Unsupported
    );
    assert_eq!(
        refused(go(server(site, "/"), server(site, "/x"))),
        IpcErrorCode::Unsupported
    );
}

#[test]
fn a_bad_folder_or_name_is_invalid_input() {
    let (ops, _target, _events) = running();
    let fixture = Fixture::new();
    let site = fixture.site();
    let go = |dir: &str, to: &str, list: &[&str]| {
        send(
            &ops,
            &fixture.remotes,
            OpKind::Copy,
            server(site, dir),
            folder(to),
            list,
        )
    };

    for dir in ["", "relative", "/a/../b", "/a//b", "/a/", "/nul\0"] {
        assert_eq!(refused(go(dir, &root(), &["a"])), IpcErrorCode::InvalidInput, "{dir:?}");
    }
    for to in ["", "relative/folder"] {
        assert_eq!(refused(go("/", to, &["a"])), IpcErrorCode::InvalidInput, "{to:?}");
    }
    for bad in [&[][..], &[""], &["."], &[".."], &["a/b"], &["/etc"], &["nul\0"]] {
        assert_eq!(refused(go("/", &root(), bad)), IpcErrorCode::InvalidInput, "{bad:?}");
    }
    let long = "x".repeat(70_000);
    assert_eq!(refused(go("/", &root(), &[&long])), IpcErrorCode::InvalidInput);
}

#[test]
fn without_a_pool_or_a_connected_site_the_command_refuses_with_a_fixed_error() {
    let (ops, _target, _events) = running();
    let place = server(SiteId(9), "/");

    let none = send(
        &ops,
        &without_pool(),
        OpKind::Copy,
        place.clone(),
        folder(&root()),
        &["a"],
    );
    assert_eq!(refused(none), IpcErrorCode::Unsupported);

    let fixture = Fixture::new();
    let gone = send(&ops, &fixture.remotes, OpKind::Copy, place, folder(&root()), &["a"]).expect_err("refused");
    assert_eq!(gone.code(), IpcErrorCode::Io);
    assert!(!format!("{gone:?}").contains("127.0.0.1"), "{gone:?}");

    let site = fixture.site();
    block_on(fixture.remotes.disconnect(site)).expect("closed");
    let closed = send(
        &ops,
        &fixture.remotes,
        OpKind::Copy,
        server(site, "/"),
        folder(&root()),
        &["a"],
    );
    assert_eq!(refused(closed), IpcErrorCode::Io);
}

#[test]
fn a_download_copies_the_file_and_the_job_is_announced_with_its_server() {
    let (ops, target, events) = running();
    let fixture = Fixture::new();
    let site = fixture.site();
    fixture.server.put("data/notes.txt", b"twelve bytes");
    fixture.server.put("data/sub/inner.txt", b"inside");
    let to = target.path().to_string_lossy().into_owned();

    let id = send(
        &ops,
        &fixture.remotes,
        OpKind::Copy,
        server(site, "/data"),
        folder(&to),
        &["notes.txt", "sub", "notes.txt"],
    )
    .expect("queued");

    assert_eq!(wait_for_end(&events), Outcome::Done);
    assert_eq!(
        std::fs::read(target.path().join("notes.txt")).expect("file"),
        b"twelve bytes"
    );
    assert_eq!(
        std::fs::read(target.path().join("sub/inner.txt")).expect("file"),
        b"inside"
    );
    let job = added_job(&events, id);
    assert_eq!(job.kind, OpKind::Copy);
    let remote = job.remote.expect("the job names its server");
    assert_eq!((remote.site, remote.dir.as_str()), (site, "/data"));
}

#[test]
fn a_download_move_removes_the_file_from_the_server() {
    let (ops, target, events) = running();
    let fixture = Fixture::new();
    let site = fixture.site();
    fixture.server.put("data/moved.txt", b"gone soon");
    let to = target.path().to_string_lossy().into_owned();

    send(
        &ops,
        &fixture.remotes,
        OpKind::Move,
        server(site, "/data"),
        folder(&to),
        &["moved.txt"],
    )
    .expect("queued");

    assert_eq!(wait_for_end(&events), Outcome::Done);
    assert_eq!(
        std::fs::read(target.path().join("moved.txt")).expect("file"),
        b"gone soon"
    );
    assert!(!fixture.server.root.path().join("data/moved.txt").exists());
}
