//! What a synchronize job refuses when it is submitted, before any disk is touched: bad paths, plans a copy cannot carry out,
//! and an archive on the wrong side.

use std::fs;

use pretty_assertions::assert_eq;

use crate::model::{EntryKind, SyncFacts};

use super::sync_tests::{Pair, file, plan, with_plans};
use super::testing::{Rig, task};
use super::{ArchiveJob, Refused, SyncPlan, SyncRoot, Task};

use crate::model::SyncDirection::ToRight;

#[test]
fn what_the_page_cannot_ask_for_is_refused_when_the_job_is_submitted() {
    let pair = Pair::new();
    file(&pair.left().join("a.txt"), b"a", 100);
    file(&pair.left().join("dir/b.txt"), b"b", 100);
    let good = plan(&pair.left(), &pair.right(), "a.txt", ToRight);
    let engine = Rig::new().engine;
    let submit =
        |rels: &[&str], plans: Vec<SyncPlan>| engine.submit(with_plans(&pair.left(), &pair.right(), rels, plans));

    for bad in ["../a.txt", "/etc/passwd", "a/../b", "a//b", "a/", "", ".", "a/./b"] {
        assert_eq!(
            submit(&[bad], vec![good]).unwrap_err(),
            Refused::InvalidNames,
            "{bad:?}"
        );
    }
    assert_eq!(
        submit(&["a.txt", "a.txt"], vec![good, good]).unwrap_err(),
        Refused::InvalidNames
    );
    assert_eq!(
        submit(&["dir", "dir/b.txt"], vec![good, good]).unwrap_err(),
        Refused::InvalidNames
    );
    assert_eq!(submit(&["a.txt"], vec![good, good]).unwrap_err(), Refused::InvalidTask);
    assert_eq!(submit(&["a.txt"], Vec::new()).unwrap_err(), Refused::InvalidTask);
}

#[test]
fn a_plan_that_a_copy_cannot_carry_out_is_refused_when_the_job_is_submitted() {
    let pair = Pair::new();
    file(&pair.left().join("a.txt"), b"a", 100);
    fs::create_dir_all(pair.left().join("dir")).expect("dir");
    let engine = Rig::new().engine;
    let submit = |rel: &str, plan: SyncPlan| engine.submit(with_plans(&pair.left(), &pair.right(), &[rel], vec![plan]));

    let file_plan = plan(&pair.left(), &pair.right(), "a.txt", ToRight);
    let folder_plan = plan(&pair.left(), &pair.right(), "dir", ToRight);
    let folder_facts = folder_plan.source;

    // A file over a folder, a folder over a file or a folder, and something that is neither.
    let over_folder = SyncPlan {
        target: Some(folder_facts),
        ..file_plan
    };
    let folder_over_folder = SyncPlan {
        target: Some(folder_facts),
        ..folder_plan
    };
    let folder_over_file = SyncPlan {
        target: Some(file_plan.source),
        ..folder_plan
    };
    let other = SyncPlan {
        source: SyncFacts {
            kind: EntryKind::Other,
            ..file_plan.source
        },
        ..file_plan
    };

    assert_eq!(submit("a.txt", over_folder).unwrap_err(), Refused::InvalidTask);
    assert_eq!(submit("dir", folder_over_folder).unwrap_err(), Refused::InvalidTask);
    assert_eq!(submit("dir", folder_over_file).unwrap_err(), Refused::InvalidTask);
    assert_eq!(submit("a.txt", other).unwrap_err(), Refused::InvalidTask);
    assert!(submit("a.txt", file_plan).is_ok());
}

#[test]
fn a_copy_into_an_archive_and_two_archives_are_refused_when_the_job_is_submitted() {
    let pair = Pair::new();
    file(&pair.left().join("a.txt"), b"a", 100);
    let good = plan(&pair.left(), &pair.right(), "a.txt", ToRight);
    let archive = SyncRoot::Archive {
        name: "x.zip".to_owned(),
        inner: String::new(),
        source: ArchiveJob::default(),
    };
    let engine = Rig::new().engine;
    let submit = |left: &SyncRoot, right: &SyncRoot, plan: SyncPlan| {
        engine.submit(task(
            &pair.left(),
            &["a.txt"],
            &pair.right(),
            Task::Sync {
                left: left.clone(),
                right: right.clone(),
                plans: vec![plan],
                removals: Vec::new(),
                permanent: false,
            },
        ))
    };

    let into_archive = submit(&SyncRoot::Folder, &archive, good);
    let both = submit(&archive, &archive, good);
    let from_archive = submit(&archive, &SyncRoot::Folder, good);

    assert_eq!(into_archive.unwrap_err(), Refused::InvalidTask);
    assert_eq!(both.unwrap_err(), Refused::InvalidTask);
    assert!(from_archive.is_ok());
}

#[cfg(windows)]
#[test]
fn a_device_name_is_refused_as_a_name_on_the_way() {
    let pair = Pair::new();
    file(&pair.left().join("a.txt"), b"a", 100);
    let good = plan(&pair.left(), &pair.right(), "a.txt", ToRight);
    let engine = Rig::new().engine;

    for bad in ["CON", "dir/NUL", "COM1.txt", "a:b"] {
        let refused = engine.submit(with_plans(&pair.left(), &pair.right(), &[bad], vec![good]));

        assert_eq!(refused.unwrap_err(), Refused::InvalidNames, "{bad:?}");
    }
}
