//! The requests of the pack and add-to-archive tasks: what `submit` accepts and refuses, and what the kind of a task is.

use std::sync::Arc;

use pretty_assertions::assert_eq;

use crate::archive::Password;
use crate::model::{OpKind, Outcome, PackFormat, PackLevel};

use super::testing::{Rig, task, temp};
use super::{ArchiveJob, PackJob, Refused, Request, Task};

fn options(format: PackFormat) -> PackJob {
    PackJob {
        format,
        level: PackLevel::Normal,
        password: None,
        encrypt_names: false,
    }
}

fn locked(format: PackFormat, encrypt_names: bool) -> PackJob {
    PackJob {
        password: Some(Arc::new(Password::new("secret".to_owned()))),
        encrypt_names,
        ..options(format)
    }
}

fn pack(name: &str, options: PackJob) -> Task {
    Task::Pack {
        name: name.to_owned(),
        options,
    }
}

fn add(name: &str, inner: &str) -> Task {
    Task::AddToArchive {
        name: name.to_owned(),
        inner: inner.to_owned(),
        source: ArchiveJob::default(),
    }
}

/// What `submit` says to a request of `task` for the items `names`; an accepted job is waited for.
fn submitted(task: Task, names: &[&str]) -> Result<(), Refused> {
    let root = temp();
    let mut rig = Rig::new();
    let request = self::task(root.path(), names, root.path(), task);

    let accepted = rig.engine.submit(request);
    if let Ok(id) = accepted {
        let ended = rig.wait_ended(id);

        // The items are not there: the jobs that are accepted fail, whatever they are.
        assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
    }

    accepted.map(|_| ())
}

#[test]
fn the_new_tasks_have_their_own_kinds() {
    assert_eq!(pack("a.zip", options(PackFormat::Zip)).kind(), Some(OpKind::Pack));
    assert_eq!(add("a.zip", "").kind(), Some(OpKind::AddToArchive));

    let request = Request::task(
        "/a".into(),
        vec!["x".to_owned()],
        "/b".into(),
        pack("a.7z", options(PackFormat::SevenZ)),
    );
    assert_eq!(request.kind, OpKind::Pack);
}

#[test]
fn a_pack_needs_a_plain_archive_name_of_its_formats_extension() {
    let zip = || options(PackFormat::Zip);

    assert_eq!(submitted(pack("a.zip", zip()), &["x", "y"]), Ok(()));
    assert_eq!(submitted(pack("A.ZIP", zip()), &["x"]), Ok(()));
    assert_eq!(
        submitted(pack("a.tar.zst", options(PackFormat::TarZst)), &["x"]),
        Ok(())
    );
    assert_eq!(submitted(pack("a.7z", options(PackFormat::SevenZ)), &["x"]), Ok(()));

    for name in ["a.7z", "a.tar.zst", "a.rar", "a.txt", "a", "a.zip.part", ""] {
        assert_eq!(
            submitted(pack(name, zip()), &["x"]),
            Err(Refused::InvalidTask),
            "{name}"
        );
    }
    for name in ["d/a.zip", "..", "a/../b.zip"] {
        assert_eq!(
            submitted(pack(name, zip()), &["x"]),
            Err(Refused::InvalidTask),
            "{name}"
        );
    }
    assert_eq!(
        submitted(pack(&format!("{}.zip", "a".repeat(300)), zip()), &["x"]),
        Err(Refused::InvalidTask)
    );
}

#[test]
fn a_pack_has_the_names_of_its_sources_plain_and_unique() {
    let zip = || pack("a.zip", options(PackFormat::Zip));

    assert_eq!(submitted(zip(), &[]), Err(Refused::InvalidNames));
    assert_eq!(submitted(zip(), &["x", "x"]), Err(Refused::InvalidNames));
    assert_eq!(submitted(zip(), &[".."]), Err(Refused::InvalidNames));
}

#[test]
fn a_pack_takes_relative_paths_of_plain_names_none_inside_another() {
    let zip = || pack("a.zip", options(PackFormat::Zip));

    assert_eq!(submitted(zip(), &["a/b.txt", "c/d/e.txt", "f"]), Ok(()));
    assert_eq!(submitted(zip(), &["a/b", "a/c", "ab"]), Ok(()));

    for names in [
        &["../x"][..],
        &["a/../x"],
        &["/abs"],
        &["a//b"],
        &["a/./b"],
        &["./a/b"],
        &["a/"],
        &["a/b\\c"],
        &["a/b\0c"],
        &["a/b", "a/b"],
        &["a", "a/b"],
        &["a/b/c", "x", "a/b"],
    ] {
        assert_eq!(submitted(zip(), names), Err(Refused::InvalidNames), "{names:?}");
    }

    let long = format!("a/{}", "x".repeat(300));
    assert_eq!(submitted(zip(), &[&long]), Err(Refused::InvalidNames));
}

#[test]
fn only_a_pack_takes_relative_paths() {
    for task in [Task::Transfer, add("a.zip", ""), Task::Split { part_size: u64::MAX }] {
        assert_eq!(
            submitted(task.clone(), &["a/b"]),
            Err(Refused::InvalidNames),
            "{task:?}"
        );
    }
}

#[test]
fn a_pack_has_a_password_only_where_the_format_can_keep_it() {
    assert_eq!(submitted(pack("a.zip", locked(PackFormat::Zip, false)), &["x"]), Ok(()));
    assert_eq!(
        submitted(pack("a.7z", locked(PackFormat::SevenZ, true)), &["x"]),
        Ok(())
    );
    assert_eq!(
        submitted(pack("a.tar.zst", locked(PackFormat::TarZst, false)), &["x"]),
        Err(Refused::InvalidTask)
    );

    // Names are encrypted in a 7z with a password, and nowhere else.
    let unlocked_names = PackJob {
        encrypt_names: true,
        ..options(PackFormat::SevenZ)
    };
    assert_eq!(
        submitted(pack("a.7z", unlocked_names), &["x"]),
        Err(Refused::InvalidTask)
    );
    assert_eq!(
        submitted(pack("a.zip", locked(PackFormat::Zip, true)), &["x"]),
        Err(Refused::InvalidTask)
    );
}

#[test]
fn an_add_needs_a_writable_archive_name_and_a_normal_folder() {
    for name in [
        "a.zip",
        "a.tar",
        "a.tar.gz",
        "a.tgz",
        "a.tar.xz",
        "a.tar.zst",
        "a.tar.bz2",
        "a.7z",
    ] {
        assert_eq!(submitted(add(name, "a/b"), &["x"]), Ok(()), "{name}");
    }
    assert_eq!(submitted(add("a.zip", ""), &["x", "y"]), Ok(()));

    for name in ["a.rar", "a.gz", "a.txt", "a", "d/a.zip", ".."] {
        assert_eq!(submitted(add(name, ""), &["x"]), Err(Refused::InvalidTask), "{name}");
    }
    for inner in ["../x", "a/../b", "a//b", "/a", "a/", "a\\b", "C:x", "./a"] {
        assert_eq!(
            submitted(add("a.zip", inner), &["x"]),
            Err(Refused::InvalidTask),
            "{inner}"
        );
    }

    assert_eq!(submitted(add("a.zip", ""), &[]), Err(Refused::InvalidNames));
    assert_eq!(submitted(add("a.zip", ""), &["x", "x"]), Err(Refused::InvalidNames));
}

#[test]
fn a_pack_and_an_add_are_not_a_copy() {
    let root = temp();
    let rig = Rig::new();
    let mismatched = Request {
        kind: OpKind::Copy,
        ..task(
            root.path(),
            &["x"],
            root.path(),
            pack("a.zip", options(PackFormat::Zip)),
        )
    };
    let swapped = Request {
        kind: OpKind::Pack,
        ..task(root.path(), &["x"], root.path(), add("a.zip", ""))
    };

    assert_eq!(rig.engine.submit(mismatched), Err(Refused::InvalidTask));
    assert_eq!(rig.engine.submit(swapped), Err(Refused::InvalidTask));
}

#[test]
fn a_pack_job_compares_its_password_by_identity_and_never_shows_it() {
    let one = locked(PackFormat::Zip, false);
    let same = one.clone();
    let other = locked(PackFormat::Zip, false);

    assert_eq!(one, same);
    assert!(one != other, "two passwords that are two allocations are two jobs");
    assert!(one != options(PackFormat::Zip));

    let shown = format!("{:?}", pack("a.zip", one));
    assert!(!shown.contains("secret"), "{shown}");
    assert!(shown.contains("password: true"), "{shown}");
}
