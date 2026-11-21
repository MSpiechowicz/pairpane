//! Checksum jobs on real folders: the file they write, and the check of one against the files.

use std::fs;
use std::path::Path;

use pretty_assertions::assert_eq;

use crate::model::{HashAlgorithm, IpcErrorCode, ItemError, JobPhase, OpKind, OpsEvent, Outcome};

use super::testing::{Rig, has_no_partial_files, put, read, task, temp, tree};
use super::{Refused, Request, Task};

const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const ABC_MD5: &str = "900150983cd24fb0d6963f7d28e17f72";
const ABC_BLAKE3: &str = "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85";

fn sums(algorithm: HashAlgorithm, output: &str) -> Task {
    Task::Checksum {
        algorithm,
        output: output.to_owned(),
    }
}

fn text(path: &Path) -> String {
    String::from_utf8(read(path)).expect("text")
}

/// How many files the job went through, as its last progress report says.
fn done_files(rig: &Rig) -> u32 {
    rig.seen
        .iter()
        .rev()
        .find_map(|event| match event {
            OpsEvent::Progress { done_files, .. } => Some(*done_files),
            _ => None,
        })
        .expect("a progress report")
}

fn reasons(errors: &[ItemError]) -> Vec<(String, String)> {
    errors
        .iter()
        .map(|error| (error.path.clone(), error.error.message.clone()))
        .collect()
}

/// How the failed job ended: its error text.
fn failure(ended: &super::testing::Ended) -> String {
    match &ended.outcome {
        Outcome::Failed { error } => error.message.clone(),
        other => panic!("the job fails: {other:?}"),
    }
}

/// The item errors of the lines numbered `numbers` that cannot be read.
fn unreadable(numbers: impl IntoIterator<Item = usize>) -> Vec<(String, String)> {
    numbers
        .into_iter()
        .map(|line| (format!("line {line}"), "this line cannot be read".to_owned()))
        .collect()
}

/// Writes `content` as the checksum file `S.sha256` in `root/src` and checks it.
fn verified(root: &Path, content: &str) -> (Rig, super::testing::Ended) {
    put(root, "src/S.sha256", content.as_bytes());
    let mut rig = Rig::new();
    let ended = verify(&mut rig, root, "S.sha256");

    (rig, ended)
}

/// Runs the check of the checksum file `sums` in `root/src`.
fn verify(rig: &mut Rig, root: &Path, sums: &str) -> super::testing::Ended {
    let src = root.join("src");

    rig.run(task(&src, &[sums], &src, Task::VerifyChecksum))
}

#[test]
fn a_checksum_file_lists_every_file_below_the_items_in_order() {
    let root = temp();
    put(root.path(), "src/a.txt", b"abc");
    put(root.path(), "src/dir/b.txt", b"abc");
    put(root.path(), "src/dir/a.txt", b"abc");
    put(root.path(), "src/dir/sub/c.txt", b"abc");
    let src = root.path().join("src");
    let mut rig = Rig::new();

    let ended = rig.run(task(
        &src,
        &["dir", "a.txt"],
        &src,
        sums(HashAlgorithm::Sha256, "S.sha256"),
    ));

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    let line = |name: &str| format!("{ABC_SHA256}  {name}\n");
    assert_eq!(
        text(&src.join("S.sha256")),
        [
            line("dir/a.txt"),
            line("dir/b.txt"),
            line("dir/sub/c.txt"),
            line("a.txt")
        ]
        .concat()
    );
    assert!(has_no_partial_files(root.path()));
    assert!(super::testing::phases(&rig).contains(&JobPhase::Hashing));
}

#[test]
fn every_algorithm_writes_its_own_hex() {
    let root = temp();
    put(root.path(), "src/a.txt", b"abc");
    let src = root.path().join("src");
    let mut rig = Rig::new();

    for (algorithm, name, hex) in [
        (HashAlgorithm::Md5, "a.md5", ABC_MD5),
        (HashAlgorithm::Blake3, "a.blake3", ABC_BLAKE3),
    ] {
        let ended = rig.run(task(&src, &["a.txt"], &src, sums(algorithm, name)));

        assert_eq!(ended.outcome, Outcome::Done);
        assert_eq!(text(&src.join(name)), format!("{hex}  a.txt\n"));
    }
}

#[test]
fn a_name_with_a_backslash_or_a_line_break_is_written_escaped_and_checks_out() {
    let root = temp();
    put(root.path(), "src/a\\b.txt", b"abc");
    let odd = "line\nbreak.txt";
    if fs::write(root.path().join("src").join(odd), b"abc").is_err() {
        return;
    }
    let src = root.path().join("src");
    let mut rig = Rig::new();

    rig.run(task(
        &src,
        &["a\\b.txt", odd],
        &src,
        sums(HashAlgorithm::Sha256, "S.sha256"),
    ));

    assert_eq!(
        text(&src.join("S.sha256")),
        format!("\\{ABC_SHA256}  a\\\\b.txt\n\\{ABC_SHA256}  line\\nbreak.txt\n")
    );
    let ended = verify(&mut rig, root.path(), "S.sha256");
    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(done_files(&rig), 2);
}

#[test]
fn a_name_that_is_taken_fails_the_job_and_the_file_there_stays() {
    let root = temp();
    put(root.path(), "src/a.txt", b"abc");
    put(root.path(), "src/S.sha256", b"mine");
    let src = root.path().join("src");
    let mut rig = Rig::new();

    let ended = rig.run(task(&src, &["a.txt"], &src, sums(HashAlgorithm::Sha256, "S.sha256")));

    assert_eq!(failure(&ended), "something with this name exists already");
    assert_eq!(text(&src.join("S.sha256")), "mine");
    assert!(has_no_partial_files(root.path()));
}

#[test]
fn nothing_to_list_fails_the_job_and_writes_nothing() {
    let root = temp();
    put(root.path(), "src/dir/.keep", b"");
    fs::remove_file(root.path().join("src/dir/.keep")).expect("removed");
    let src = root.path().join("src");
    let mut rig = Rig::new();

    let ended = rig.run(task(&src, &["dir"], &src, sums(HashAlgorithm::Sha256, "S.sha256")));

    assert!(matches!(ended.outcome, Outcome::Failed { .. }));
    assert_eq!(tree(&src), vec!["dir/".to_owned()]);
}

#[cfg(unix)]
#[test]
fn links_are_skipped_and_counted_never_read() {
    use std::os::unix::fs::symlink;

    let root = temp();
    put(root.path(), "src/a.txt", b"abc");
    put(root.path(), "src/dir/b.txt", b"abc");
    put(root.path(), "outside/secret.txt", b"secret");
    let src = root.path().join("src");
    symlink(root.path().join("outside"), src.join("dir/to-outside")).expect("link");
    symlink(src.join("a.txt"), src.join("link.txt")).expect("link");
    let mut rig = Rig::new();

    let ended = rig.run(task(
        &src,
        &["dir", "link.txt", "a.txt"],
        &src,
        sums(HashAlgorithm::Sha256, "S.sha256"),
    ));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.skipped, 2);
    assert_eq!(
        text(&src.join("S.sha256")),
        format!("{ABC_SHA256}  dir/b.txt\n{ABC_SHA256}  a.txt\n")
    );
}

#[test]
fn a_cancel_while_hashing_leaves_nothing_behind() {
    let root = temp();
    put(root.path(), "src/a.txt", b"abc");
    let src = root.path().join("src");
    let mut rig = Rig::paused_in(JobPhase::Hashing);

    let job = rig.start(task(&src, &["a.txt"], &src, sums(HashAlgorithm::Sha256, "S.sha256")));

    assert_eq!(rig.cancel_when_paused(job).outcome, Outcome::Cancelled);
    assert_eq!(tree(&src), vec!["a.txt".to_owned()]);
}

#[test]
fn a_checksum_file_of_sha256sum_checks_out() {
    let root = temp();
    put(root.path(), "src/abc.txt", b"abc");
    put(root.path(), "src/bin.dat", b"abc");
    put(root.path(), "src/sub/abc.txt", b"abc");
    // Text mode, binary mode, a Windows line break, an empty line, upper case hex.
    let file = format!(
        "{ABC_SHA256}  abc.txt\n{ABC_SHA256} *bin.dat\r\n\n{}  sub/abc.txt\n",
        ABC_SHA256.to_uppercase()
    );
    put(root.path(), "src/SUMS.sha256", file.as_bytes());
    let mut rig = Rig::new();

    let ended = verify(&mut rig, root.path(), "SUMS.sha256");

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(done_files(&rig), 3);
}

#[test]
fn every_line_reports_on_its_own() {
    let root = temp();
    put(root.path(), "src/ok.txt", b"abc");
    put(root.path(), "src/bad.txt", b"abd");
    put(root.path(), "src/dir/inner", b"abc");
    let file = format!(
        "{ABC_SHA256}  ok.txt\n{ABC_SHA256}  bad.txt\n{ABC_SHA256}  gone.txt\nnot a line\n\
         {ABC_SHA256}  dir\n{ABC_SHA256}  nodir/inner\n{ABC_SHA256}  dir/inner\n"
    );
    let (rig, ended) = verified(root.path(), &file);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(
        reasons(&ended.errors),
        vec![
            ("bad.txt".to_owned(), "the checksum does not match".to_owned()),
            ("gone.txt".to_owned(), "the file is missing".to_owned()),
            ("line 4".to_owned(), "this line cannot be read".to_owned()),
            ("dir".to_owned(), "it is not a regular file".to_owned()),
            ("nodir/inner".to_owned(), "the file is missing".to_owned()),
        ]
    );
    assert_eq!(ended.errors[0].error.code, IpcErrorCode::Io);
    assert_eq!(ended.errors[1].error.code, IpcErrorCode::NotFound);
    assert_eq!(ended.errors[2].error.code, IpcErrorCode::InvalidInput);
    // Seven lines, five of them in error.
    assert_eq!(done_files(&rig), 7);
}

#[test]
fn a_path_that_leaves_the_folder_is_never_opened() {
    let root = temp();
    put(root.path(), "outside.txt", b"abc");
    put(root.path(), "src/in.txt", b"abc");
    let absolute = root.path().join("outside.txt");
    let file = format!(
        "{ABC_SHA256}  ../outside.txt\n{ABC_SHA256}  {}\n{ABC_SHA256}  in/../in.txt\n{ABC_SHA256}  ./in.txt\n",
        absolute.display()
    );
    let (_, ended) = verified(root.path(), &file);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(reasons(&ended.errors), unreadable(1..=4));
}

#[cfg(unix)]
#[test]
fn a_link_at_a_listed_name_or_on_the_way_to_it_is_refused() {
    use std::os::unix::fs::symlink;

    let root = temp();
    put(root.path(), "src/real/f.txt", b"abc");
    let src = root.path().join("src");
    symlink(src.join("real/f.txt"), src.join("link.txt")).expect("link");
    symlink(src.join("real"), src.join("linked")).expect("link");
    let file = format!("{ABC_SHA256}  link.txt\n{ABC_SHA256}  linked/f.txt\n{ABC_SHA256}  real/f.txt\n");
    let (_, ended) = verified(root.path(), &file);

    assert_eq!(
        reasons(&ended.errors),
        vec![
            ("link.txt".to_owned(), "the path goes through a link".to_owned()),
            ("linked/f.txt".to_owned(), "the path goes through a link".to_owned()),
        ]
    );
}

#[test]
fn the_algorithm_comes_from_the_extension_and_else_from_the_length() {
    let root = temp();
    put(root.path(), "src/a.txt", b"abc");
    let mut rig = Rig::new();

    for (name, line) in [
        ("S.md5", format!("{ABC_MD5}  a.txt\n")),
        ("S.blake3", format!("{ABC_BLAKE3}  a.txt\n")),
        ("S.b3", format!("{ABC_BLAKE3}  a.txt\n")),
        ("S.md5sum", format!("{ABC_MD5}  a.txt\n")),
        ("sums", format!("{ABC_MD5}  a.txt\n")),
        ("sums.txt", format!("{ABC_SHA256}  a.txt\n")),
    ] {
        put(root.path(), &format!("src/{name}"), line.as_bytes());

        let ended = verify(&mut rig, root.path(), name);

        assert_eq!(ended.outcome, Outcome::Done, "{name}");
        assert!(ended.errors.is_empty(), "{name}: {:?}", ended.errors);
    }
}

#[test]
fn a_file_that_does_not_say_which_checksum_it_holds_fails_the_job() {
    let root = temp();
    put(root.path(), "src/S.txt", b"abcd  a.txt\n");
    let mut rig = Rig::new();

    let ended = verify(&mut rig, root.path(), "S.txt");

    assert_eq!(failure(&ended), "the kind of checksum cannot be told");
}

#[test]
fn a_hash_of_the_wrong_length_for_the_extension_is_an_unreadable_line() {
    let root = temp();
    put(root.path(), "src/a.txt", b"abc");
    let (_, ended) = verified(root.path(), &format!("{ABC_MD5}  a.txt\n"));

    assert_eq!(reasons(&ended.errors), unreadable([1]));
}

#[test]
fn a_checksum_file_that_is_too_large_is_refused() {
    let root = temp();
    let (_, ended) = verified(root.path(), &"a".repeat(16 * 1024 * 1024 + 1));

    assert_eq!(failure(&ended), "the checksum file is too large");
}

#[test]
fn a_byte_order_mark_at_the_start_of_the_file_is_ignored() {
    let root = temp();
    put(root.path(), "src/a.txt", b"abc");

    let (_, ended) = verified(root.path(), &format!("\u{feff}{ABC_SHA256}  a.txt\n"));

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
}

#[test]
fn a_file_with_too_many_lines_fails_the_job() {
    let root = temp();
    let (_, ended) = verified(root.path(), &"a\n".repeat(1_000_001));

    assert_eq!(failure(&ended), "the checksum file has too many lines");

    let (_, ended) = verified(root.path(), &"a\n".repeat(1_000_000));
    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 100, "the first hundred are told");
    assert_eq!(ended.more_errors, 999_900);
}

#[test]
fn a_line_that_is_too_long_is_unreadable_and_the_next_one_is_read() {
    let root = temp();
    put(root.path(), "src/a.txt", b"abc");
    let long = "x".repeat(64 * 1024 + 1);
    let file = format!("{ABC_SHA256}  {long}\n{ABC_SHA256}  a.txt\n");
    let (_, ended) = verified(root.path(), &file);

    assert_eq!(reasons(&ended.errors), unreadable([1]));
}

#[test]
fn a_task_that_does_not_fit_its_kind_or_its_items_is_refused_at_submit() {
    let root = temp();
    let src = root.path().join("src");
    let rig = Rig::new();
    let mismatched = Request {
        kind: OpKind::Copy,
        ..task(&src, &["a"], &src, Task::VerifyChecksum)
    };
    let a_move_of_a_task = Request {
        kind: OpKind::Move,
        ..task(&src, &["a"], &src, sums(HashAlgorithm::Md5, "a.md5"))
    };

    let refused = |request: Request| rig.engine.submit(request).expect_err("refused");

    assert_eq!(refused(mismatched), Refused::InvalidTask);
    assert_eq!(refused(a_move_of_a_task), Refused::InvalidTask);
    assert_eq!(
        refused(task(&src, &["a", "b"], &src, Task::VerifyChecksum)),
        Refused::InvalidTask
    );
    assert_eq!(
        refused(task(&src, &["a", "b"], &src, Task::Combine)),
        Refused::InvalidTask
    );
    assert_eq!(
        refused(task(&src, &["a"], &src, sums(HashAlgorithm::Md5, "dir/a.md5"))),
        Refused::InvalidTask
    );
    assert_eq!(
        refused(task(&src, &["a"], &src, sums(HashAlgorithm::Md5, ".."))),
        Refused::InvalidTask
    );
}

#[test]
fn a_transfer_task_is_a_copy_unless_the_kind_says_move() {
    let request = Request::task("/a".into(), vec!["x".to_owned()], "/b".into(), Task::Transfer);

    assert_eq!(request.kind, OpKind::Copy);
    assert!(!request.verify);
    assert_eq!(Task::Transfer.kind(), None);
    assert_eq!(Task::Combine.kind(), Some(OpKind::Combine));
}
