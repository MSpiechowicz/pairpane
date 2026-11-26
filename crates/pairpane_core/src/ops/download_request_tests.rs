//! The request of a download: what `submit` accepts and refuses, how the queue shows it and what it is equal to.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pretty_assertions::assert_eq;

use crate::model::{JobId, JobRemote, OpKind, SiteId};
use crate::vfs::{RemoteEntry, RemoteFs, RemoteMeta, RemotePath};

use super::job::Job;
use super::validate::validate;
use super::{Direction, MAX_NAME_UNITS, Refused, RemoteJob, Request, Task};

/// A server that answers nothing: these tests look at the request, not at what a download does.
struct Server;

impl RemoteFs for Server {
    fn site(&self) -> SiteId {
        SiteId(7)
    }

    fn label(&self) -> &str {
        "me@files"
    }

    fn read_dir(&self, _: &RemotePath, _: &AtomicBool) -> io::Result<Vec<RemoteEntry>> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn stat(&self, _: &RemotePath) -> io::Result<Option<RemoteMeta>> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn read_link(&self, _: &RemotePath) -> io::Result<String> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

fn server() -> Arc<dyn RemoteFs> {
    Arc::new(Server)
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn download(kind: OpKind, items: &[&str]) -> Request {
    Request::download(
        kind,
        server(),
        RemotePath::new("/srv/data").expect("path"),
        names(items),
        PathBuf::from("/home/me"),
        false,
    )
}

fn upload(kind: OpKind) -> Request {
    Request {
        task: Task::Remote(RemoteJob {
            direction: Direction::Upload,
            fs: server(),
            dir: RemotePath::new("/srv/data").expect("path"),
        }),
        ..download(kind, &["a"])
    }
}

#[test]
fn a_download_is_a_copy_or_a_move_with_no_local_source_folder() {
    let request = download(OpKind::Move, &["a", "b"]);

    assert_eq!(request.kind, OpKind::Move);
    assert_eq!(request.from, PathBuf::new());
    assert_eq!(request.to, PathBuf::from("/home/me"));
    assert_eq!(request.task.kind(), None);
    assert!(matches!(&request.task, Task::Remote(job) if job.direction == Direction::Download));
}

#[test]
fn submit_accepts_a_download_that_copies_or_moves() {
    assert_eq!(validate(&download(OpKind::Copy, &["a", "b c", ".hidden"])), Ok(()));
    assert_eq!(validate(&download(OpKind::Move, &["a"])), Ok(()));
}

#[test]
fn submit_refuses_a_download_of_any_other_kind() {
    for kind in [OpKind::Checksum, OpKind::Extract, OpKind::Sync, OpKind::Rename] {
        assert_eq!(validate(&download(kind, &["a"])), Err(Refused::InvalidTask), "{kind:?}");
    }
}

#[test]
fn submit_refuses_an_upload_for_now() {
    assert_eq!(validate(&upload(OpKind::Copy)), Err(Refused::InvalidTask));
    assert_eq!(validate(&upload(OpKind::Move)), Err(Refused::InvalidTask));
}

#[test]
fn the_names_of_a_download_are_the_servers_plain_names_that_fit() {
    assert_eq!(validate(&download(OpKind::Copy, &[])), Err(Refused::InvalidNames));
    assert_eq!(
        validate(&download(OpKind::Copy, &["a", "a"])),
        Err(Refused::InvalidNames)
    );

    for name in ["", ".", "..", "a/b", "/a", "a\0b"] {
        assert_eq!(
            validate(&download(OpKind::Copy, &[name])),
            Err(Refused::InvalidNames),
            "{name:?}"
        );
    }

    let long = "a".repeat(MAX_NAME_UNITS + 1);
    assert_eq!(validate(&download(OpKind::Copy, &[&long])), Err(Refused::NamesTooLarge));
}

#[test]
fn a_name_this_computer_cannot_use_is_still_accepted() {
    // Windows reads these as a stream and a device; the run skips them and counts them.
    assert_eq!(validate(&download(OpKind::Copy, &["a:b", "NUL", "dots.."])), Ok(()));
}

#[test]
fn the_queue_shows_the_server_folder_as_the_source() {
    let view = Job::new(JobId(1), download(OpKind::Copy, &["a", "b"])).view().clone();

    assert_eq!(view.from, "me@files: /srv/data");
    assert_eq!(view.to, "/home/me");
    assert_eq!(view.label, "2 items");
    assert_eq!(
        view.remote,
        Some(JobRemote {
            site: SiteId(7),
            dir: "/srv/data".to_owned()
        })
    );
}

#[test]
fn a_job_between_local_folders_has_no_remote_side() {
    let request = Request {
        kind: OpKind::Copy,
        from: PathBuf::from("/a"),
        names: names(&["x"]),
        to: PathBuf::from("/b"),
        verify: false,
        task: Task::Transfer,
    };
    let view = Job::new(JobId(1), request).view().clone();

    assert_eq!((view.from.as_str(), view.to.as_str()), ("/a", "/b"));
    assert_eq!(view.remote, None);
}

#[test]
fn requests_are_equal_for_the_same_server_and_folder() {
    let fs = server();
    let make = |fs: &Arc<dyn RemoteFs>, dir: &str| {
        Request::download(
            OpKind::Copy,
            fs.clone(),
            RemotePath::new(dir).expect("path"),
            names(&["a"]),
            PathBuf::from("/home/me"),
            false,
        )
    };

    assert_eq!(make(&fs, "/srv"), make(&fs, "/srv"));
    assert_ne!(make(&fs, "/srv"), make(&fs, "/srv/x"));
    assert_ne!(make(&fs, "/srv"), make(&server(), "/srv"), "another connection");
}

#[test]
fn a_remote_job_is_debugged_by_its_site_folder_and_direction() {
    let request = download(OpKind::Copy, &["a"]);

    assert_eq!(
        format!("{:?}", request.task),
        r#"Remote(RemoteJob { direction: Download, site: SiteId(7), dir: RemotePath("/srv/data") })"#
    );
}
