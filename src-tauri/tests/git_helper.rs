//! The real executable as the git status helper: started with `--pairpane-git-status` it reads its request on its standard
//! input, confines itself before it opens the repository (Linux: limits, Landlock, seccomp; macOS: the sandbox; Windows: the
//! parent's Job Object), and answers on its standard output without opening a window or starting anything else of the app;
//! and the status the core crate reads with it, which is killed at its deadline and with the app.

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use pairpane_core::gitstatus::{self, CHILD_ARG, GitError, Limits};
use pairpane_core::model::GitMark;

mod common;

/// The app's own executable, which is the helper.
fn program() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_pairpane"))
}

fn idle() -> AtomicBool {
    AtomicBool::new(false)
}

/// A folder that is the work tree of a repository with no commit: the few files and folders git needs to see one.
fn repository() -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("a folder");
    write_git_folder(&folder.path().join(".git"));

    folder
}

/// The few files and folders of a git folder with no commit.
fn write_git_folder(git: &Path) {
    fs::create_dir_all(git.join("objects")).expect("objects");
    fs::create_dir_all(git.join("refs")).expect("refs");
    fs::write(git.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
    fs::write(
        git.join("config"),
        "[core]\n\trepositoryformatversion = 0\n\tbare = false\n",
    )
    .expect("config");
}

fn status(folder: &Path, limits: Limits) -> Result<pairpane_core::model::GitStatus, GitError> {
    gitstatus::status(folder, limits, &idle(), program())
}

#[test]
fn the_status_of_a_repository_comes_back_from_the_helper() {
    let folder = repository();
    fs::write(folder.path().join("new.txt"), b"n").expect("a file");
    fs::write(folder.path().join(".gitignore"), b"*.log\n").expect("rules");
    fs::write(folder.path().join("debug.log"), b"l").expect("a file");
    fs::create_dir(folder.path().join("fresh")).expect("a folder");
    fs::write(folder.path().join("fresh/inner.txt"), b"i").expect("a file");

    let answer = status(folder.path(), Limits::default());

    let answer = answer.unwrap_or_else(|error| panic!("a status: {error:?}"));
    let marks: Vec<_> = answer
        .marks
        .iter()
        .map(|entry| (entry.name.as_str(), entry.mark))
        .collect();
    assert!(answer.repo);
    assert_eq!(
        marks,
        vec![
            (".gitignore", GitMark::Untracked),
            ("debug.log", GitMark::Ignored),
            ("fresh", GitMark::Untracked),
            ("new.txt", GitMark::Untracked),
        ]
    );
}

#[test]
fn a_folder_in_no_repository_is_no_repo_and_an_error_of_the_folder_is_its_kind() {
    let folder = tempfile::tempdir().expect("a folder");

    let answer = status(folder.path(), Limits::default()).expect("a status");
    assert!(!answer.repo && answer.marks.is_empty());

    let gone = folder.path().join("gone");
    assert!(matches!(
        status(&gone, Limits::default()),
        Err(GitError::Folder(std::io::ErrorKind::NotFound))
    ));
}

#[test]
fn a_repository_over_the_cap_is_too_big_whatever_process_reads_it() {
    let folder = repository();
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write(folder.path().join(name), b"x").expect("a file");
    }
    let none = Limits {
        entries: 0,
        ..Limits::default()
    };

    assert!(matches!(status(folder.path(), none), Err(GitError::TooBig)));
}

#[test]
fn a_helper_that_cannot_start_is_a_repository_that_cannot_be_read_and_nothing_hangs() {
    let folder = repository();

    let answer = gitstatus::status(
        folder.path(),
        Limits::default(),
        &idle(),
        &folder.path().join("no-such-program"),
    );

    assert!(matches!(answer, Err(GitError::Repository)), "{answer:?}");
}

#[test]
fn a_raised_exit_flag_refuses_a_status_that_comes_after_it() {
    let folder = repository();
    let flag = AtomicBool::new(true);

    let answer = gitstatus::status(folder.path(), Limits::default(), &flag, program());

    assert!(matches!(answer, Err(GitError::Cancelled)), "{answer:?}");
}

/// A request that is no request, a request that is cut short and no request at all end the helper without an answer, at once.
#[test]
fn the_helper_answers_nothing_to_what_is_not_a_request() {
    use std::process::Stdio;

    for input in [&b""[..], b"garbage", b"\x01\x00\x00", &[0xff; 4096]] {
        let child = pairpane_core::process::spawn(
            pairpane_core::process::command(program(), [CHILD_ARG], &[]).stdin(Stdio::piped()),
        )
        .expect("the helper starts");
        let (answer, exited_well) = common::finish(child, input);

        assert!(answer.is_empty(), "{input:?}");
        assert!(!exited_well, "{input:?}");
    }
}

/// What the tests that need a helper that is held up need: the request is made by the parent's look at a repository whose
/// `.git/info` is a folder with nothing in it, and then `info/exclude` becomes a named pipe, which a plain open of waits on for
/// ever. That is the one thing the look at the files of a repository cannot see coming (it is looked at, and opened a moment
/// later), and which only the deadline answers for.
#[cfg(unix)]
mod by_hand {
    use std::process::{Command, Stdio};

    use super::*;

    /// The request for `folder`, and then the pipe in the place of `info/exclude`.
    pub(super) fn request_then_a_pipe(folder: &Path, limits: Limits) -> Vec<u8> {
        let info = folder.join(".git/info");
        fs::create_dir_all(&info).expect("a folder");
        let request = gitstatus::helper_request(folder, limits)
            .expect("a request")
            .expect("a repository");

        let made = Command::new("mkfifo")
            .arg(info.join("exclude"))
            .status()
            .expect("mkfifo runs");
        assert!(made.success());

        request
    }

    /// The helper, started by hand with `request`.
    pub(super) fn started(request: &[u8]) -> std::process::Child {
        use std::io::Write;

        let mut child = pairpane_core::process::spawn(
            pairpane_core::process::command(program(), [CHILD_ARG], &[]).stdin(Stdio::piped()),
        )
        .expect("the helper starts");
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(request).expect("the request is written");
        drop(stdin);

        child
    }
}

/// A status that the system never ends (a pipe where an object is) is answered at its deadline, the time limit and a grace, as
/// the time limit, and the helper is gone.
#[cfg(unix)]
#[test]
fn a_helper_that_is_held_up_is_killed_at_the_deadline_and_the_answer_is_the_time_limit() {
    let folder = repository();
    let limits = Limits {
        time: Duration::from_millis(1000),
        ..Limits::default()
    };
    let request = by_hand::request_then_a_pipe(folder.path(), limits);

    let started = Instant::now();
    let answer = gitstatus::status_for_request(&request, &idle(), program());
    let elapsed = started.elapsed();

    assert!(matches!(answer, Err(GitError::TimedOut)), "{answer:?}");
    assert!(elapsed >= Duration::from_millis(1000), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(10), "{elapsed:?}");
}

#[cfg(unix)]
#[test]
fn the_exit_flag_kills_a_helper_that_is_held_up_and_the_answer_is_cancelled() {
    let folder = repository();
    let request = by_hand::request_then_a_pipe(folder.path(), Limits::default());
    let flag = AtomicBool::new(false);

    let started = Instant::now();
    let answer = std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(700));
            flag.store(true, Ordering::Relaxed);
        });

        gitstatus::status_for_request(&request, &flag, program())
    });

    assert!(matches!(answer, Err(GitError::Cancelled)), "{answer:?}");
    assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());
}

/// The real helper, held up where it reads the repository, is limited and filtered, and holds nothing but its streams.
#[cfg(target_os = "linux")]
#[test]
fn the_helper_confines_itself_before_it_opens_the_repository() {
    let folder = repository();
    let request = by_hand::request_then_a_pipe(folder.path(), Limits::default());
    let mut child = by_hand::started(&request);
    let proc = format!("/proc/{}", child.id());

    // Limited on its data (the memory it may write, not the address space a pack is mapped in) and every thread filtered.
    let limited = |text: &str| {
        text.lines()
            .any(|line| line.starts_with("Max data size") && !line.contains("unlimited"))
    };
    let filtered = || {
        fs::read_dir(format!("{proc}/task")).is_ok_and(|tasks| {
            tasks.filter_map(Result::ok).all(|task| {
                fs::read_to_string(task.path().join("status")).is_ok_and(|status| status.contains("Seccomp:\t2"))
            })
        })
    };
    let limits = || fs::read_to_string(format!("{proc}/limits")).unwrap_or_default();
    for _ in 0..500 {
        if limited(&limits()) && filtered() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let text = limits();
    assert!(limited(&text), "the helper limits its memory: {text}");
    assert!(filtered(), "every thread of the helper has the seccomp filter");

    // The processor time is the time limit of the status rounded up, and a second, and the hard limit is a second more: the
    // signal of the soft limit (`SIGXCPU`) is what ends the helper, and not a kill that the parent could not tell from a crash.
    let cpu: Vec<&str> = text
        .lines()
        .find(|line| line.starts_with("Max cpu time"))
        .expect("a limit on processor time")
        .split_whitespace()
        .collect();
    assert_eq!(&cpu[3..5], ["6", "7"], "{text}");
    for line in [
        "Max cpu time              6 ",
        "Max core file size        0 ",
        "Max open files            256 ",
    ] {
        assert!(text.contains(line), "{text}");
    }
    let status = fs::read_to_string(format!("{proc}/status")).expect("status");
    assert!(status.contains("NoNewPrivs:\t1"), "{status}");

    // It leads a session of its own, so it has no controlling terminal (the field after the group is the session).
    let stat = fs::read_to_string(format!("{proc}/stat")).expect("stat");
    let after_the_name = stat.rsplit_once(')').expect("the name ends").1;
    let session: u32 = after_the_name
        .split_whitespace()
        .nth(3)
        .expect("a session")
        .parse()
        .expect("a number");
    assert_eq!(session, child.id(), "{stat}");

    // Nothing of the app reaches it: its streams, and no more.
    let mut open: Vec<u32> = fs::read_dir(format!("{proc}/fd"))
        .expect("the descriptors")
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    open.sort_unstable();
    assert_eq!(open, [0, 1, 2], "{open:?}");

    child.kill().expect("killed");
    child.wait().expect("reaped");
}

/// The answer of a helper is what the parent reads: the tag and the marks, with nothing that is a path.
#[cfg(unix)]
#[test]
fn what_the_helper_writes_is_an_answer_the_parent_reads_and_nothing_follows_it() {
    use std::io::Read;

    let folder = repository();
    fs::write(folder.path().join("new.txt"), b"n").expect("a file");
    let request = gitstatus::helper_request(folder.path(), Limits::default())
        .expect("a request")
        .expect("a repository");
    let mut child = by_hand::started(&request);

    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .expect("stdout")
        .read_to_end(&mut bytes)
        .expect("an answer");
    let ended = child.wait().expect("reaped");

    assert!(ended.success());
    let answer = gitstatus::helper_answer(&bytes).expect("a status");
    assert!(answer.repo);
    assert_eq!(answer.marks.len(), 1);
    assert!(!String::from_utf8_lossy(&bytes).contains(&folder.path().to_string_lossy().into_owned()));
}

/// A folder that was swapped for another with the same path after the parent looked at it (a link, or a folder that someone else
/// made) is not the one the helper is allowed to read: it ends without reading, and the parent answers as for a repository that
/// cannot be read. With the same request and the folder as it was, the status is read.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn a_folder_that_is_swapped_after_it_was_looked_at_is_not_read_by_the_helper() {
    let folder = repository();
    let request = gitstatus::helper_request(folder.path(), Limits::default())
        .expect("a request")
        .expect("a repository");

    let control = gitstatus::status_for_request(&request, &idle(), program()).expect("a status");
    assert!(control.repo);

    // The same files, in another folder of the same name.
    let git = folder.path().join(".git");
    fs::rename(&git, folder.path().join(".git-before")).expect("moved");
    write_git_folder(&git);

    let answer = gitstatus::status_for_request(&request, &idle(), program());

    assert!(matches!(answer, Err(GitError::Repository)), "{answer:?}");
}
