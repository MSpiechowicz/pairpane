//! Tests of `ops_task`: what is refused before the engine is asked, the request it builds, and a job started for real.

use std::time::{Duration, Instant};

use pairpane_core::model::{AttributeChange, HashAlgorithm, IpcErrorCode, JobTask, ModeChange, OpKind, Outcome};
use pairpane_core::ops::{MAX_NAME_UNITS, MIN_PART_SIZE, Request, Task};

use super::tasks::task_request;
use super::tests::{collecting_channel, received};
use super::*;
use crate::commands::testing::{elsewhere, names, root};

fn nothing() -> AttributeChange {
    AttributeChange {
        mode: None,
        read_only: None,
        hidden: None,
        system: None,
        archive: None,
        modified: None,
        accessed: None,
        recursive: false,
    }
}

fn code(result: CommandResult<Request>) -> IpcErrorCode {
    result.expect_err("refused").code()
}

fn attributes(change: AttributeChange) -> JobTask {
    JobTask::Attributes { change }
}

fn checksum(output: &str) -> JobTask {
    JobTask::Checksum {
        algorithm: HashAlgorithm::Sha256,
        output: output.to_owned(),
    }
}

#[test]
fn a_checksum_request_keeps_its_folder_names_and_output_and_works_in_place() {
    let request = task_request(root(), names(&["a", "b", "a"]), checksum("S.sha256")).expect("valid request");

    assert_eq!(request.kind, OpKind::Checksum);
    assert_eq!(request.from, Path::new(&root()));
    assert_eq!(request.to, request.from, "in place");
    assert_eq!(request.names, names(&["a", "b"]));
    assert!(!request.verify);
    assert_eq!(
        request.task,
        Task::Checksum {
            algorithm: HashAlgorithm::Sha256,
            output: "S.sha256".to_owned()
        }
    );
}

#[test]
fn split_and_combine_write_into_the_folder_they_are_given() {
    let split = task_request(
        root(),
        names(&["big.bin"]),
        JobTask::Split {
            to: elsewhere(),
            part_size: 1_457_664,
        },
    )
    .expect("valid request");
    let combine =
        task_request(root(), names(&["big.bin.001"]), JobTask::Combine { to: elsewhere() }).expect("valid request");

    assert_eq!(
        (split.kind, split.to.clone()),
        (OpKind::Split, Path::new(&elsewhere()).to_owned())
    );
    assert_eq!(split.task, Task::Split { part_size: 1_457_664 });
    assert_eq!((combine.kind, combine.task), (OpKind::Combine, Task::Combine));
    assert_eq!(combine.to, Path::new(&elsewhere()));
}

#[test]
fn verify_split_and_combine_take_exactly_one_item() {
    let tasks = || {
        [
            JobTask::VerifyChecksum,
            JobTask::Split {
                to: elsewhere(),
                part_size: MIN_PART_SIZE,
            },
            JobTask::Combine { to: elsewhere() },
        ]
    };

    for task in tasks() {
        assert_eq!(
            code(task_request(root(), names(&["a", "b"]), task)),
            IpcErrorCode::InvalidInput
        );
    }
    for task in tasks() {
        assert!(
            task_request(root(), names(&["a", "a"]), task).is_ok(),
            "duplicates count once"
        );
    }
}

#[test]
fn a_part_shorter_than_a_kilobyte_is_refused() {
    let split = |part_size| JobTask::Split {
        to: elsewhere(),
        part_size,
    };

    assert_eq!(
        code(task_request(root(), names(&["a"]), split(MIN_PART_SIZE - 1))),
        IpcErrorCode::InvalidInput
    );
    assert_eq!(
        code(task_request(root(), names(&["a"]), split(0))),
        IpcErrorCode::InvalidInput
    );
    assert!(task_request(root(), names(&["a"]), split(MIN_PART_SIZE)).is_ok());
}

#[test]
fn the_name_of_the_checksum_file_must_be_a_plain_name_that_fits() {
    let too_long = "n".repeat(MAX_NAME_UNITS + 1);

    for output in ["", ".", "..", "a/b", "../x", &too_long] {
        assert_eq!(
            code(task_request(root(), names(&["a"]), checksum(output))),
            IpcErrorCode::InvalidInput,
            "{output:?}"
        );
    }
}

#[test]
fn folders_and_names_are_checked_like_a_copy() {
    assert_eq!(
        code(task_request(
            "relative".to_owned(),
            names(&["a"]),
            JobTask::VerifyChecksum
        )),
        IpcErrorCode::InvalidInput
    );
    assert_eq!(
        code(task_request(root(), vec![], JobTask::VerifyChecksum)),
        IpcErrorCode::InvalidInput
    );
    assert_eq!(
        code(task_request(root(), names(&["../a"]), JobTask::VerifyChecksum)),
        IpcErrorCode::InvalidInput
    );
    assert_eq!(
        code(task_request(
            root(),
            names(&["a"]),
            JobTask::Combine {
                to: "relative".to_owned()
            }
        )),
        IpcErrorCode::InvalidInput
    );
}

#[test]
fn an_attribute_change_must_change_something_that_exists_on_this_system() {
    let refused = |change| code(task_request(root(), names(&["a"]), attributes(change)));

    assert_eq!(refused(nothing()), IpcErrorCode::InvalidInput);
    assert_eq!(
        refused(AttributeChange {
            recursive: true,
            ..nothing()
        }),
        IpcErrorCode::InvalidInput
    );

    let other_systems = if cfg!(windows) {
        vec![AttributeChange {
            mode: Some(ModeChange::Set { mode: 0o644 }),
            ..nothing()
        }]
    } else {
        vec![
            AttributeChange {
                read_only: Some(true),
                ..nothing()
            },
            AttributeChange {
                hidden: Some(true),
                ..nothing()
            },
            AttributeChange {
                system: Some(false),
                ..nothing()
            },
            AttributeChange {
                archive: Some(true),
                ..nothing()
            },
        ]
    };
    for change in other_systems {
        assert_eq!(refused(change), IpcErrorCode::Unsupported);
    }
}

#[test]
fn a_permission_mask_beyond_the_twelve_bits_is_refused_and_the_rest_is_accepted() {
    if cfg!(windows) {
        return;
    }
    let mode = |mode| AttributeChange {
        mode: Some(mode),
        ..nothing()
    };
    let refused = |change| code(task_request(root(), names(&["a"]), attributes(change)));

    assert_eq!(
        refused(mode(ModeChange::Set { mode: 0o10000 })),
        IpcErrorCode::InvalidInput
    );
    assert_eq!(
        refused(mode(ModeChange::Adjust {
            add: 0,
            remove: 0o10000
        })),
        IpcErrorCode::InvalidInput
    );

    for accepted in [
        mode(ModeChange::Set { mode: 0o7777 }),
        mode(ModeChange::Adjust {
            add: 0o4000,
            remove: 0o022,
        }),
        AttributeChange {
            modified: Some(-1),
            ..nothing()
        },
    ] {
        let request = task_request(root(), names(&["a"]), attributes(accepted)).expect("accepted");

        assert_eq!(request.kind, OpKind::Attributes);
        assert_eq!(request.to, request.from);
        assert_eq!(request.task, Task::Attributes(accepted));
    }
}

#[test]
fn a_task_started_through_the_command_reports_over_the_subscribed_channel() {
    let folder = tempfile::tempdir().expect("folder");
    std::fs::write(folder.path().join("a.txt"), "abc").expect("file");
    let ops = Ops::default();
    let (channel, events) = collecting_channel();
    ops.subscribe(channel);

    let id = ops
        .task(
            folder.path().to_string_lossy().into_owned(),
            names(&["a.txt"]),
            checksum("S.sha256"),
        )
        .expect("accepted");

    let deadline = Instant::now() + Duration::from_secs(20);
    while !received(&events)
        .iter()
        .any(|event| matches!(event, OpsEvent::Ended { .. }))
    {
        assert!(Instant::now() < deadline, "the job ended");
        std::thread::sleep(Duration::from_millis(10));
    }
    let seen = received(&events);
    assert!(matches!(&seen[1], OpsEvent::Added { job } if job.id == id && job.kind == OpKind::Checksum));
    assert!(seen.iter().any(|event| matches!(
        event,
        OpsEvent::Ended {
            outcome: Outcome::Done,
            ..
        }
    )));
    assert_eq!(
        std::fs::read_to_string(folder.path().join("S.sha256")).expect("written"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  a.txt\n"
    );
}
