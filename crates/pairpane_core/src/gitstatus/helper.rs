//! The parent's end of the helper that reads a status (the child's is `child`). What the parent does:
//!
//! 1. [`prepare`]: the folder is resolved and the repository it is in is found and looked at (`locate`, `inspect`): what a
//!    repository names, what its files are and how big, and where they are, so that it can say `repo: false` with no process
//!    started, and say which folders the helper may read (the [`Layout`](super::layout::Layout)).
//! 2. [`run`]: starts the app's own executable with [`CHILD_ARG`](super::child::CHILD_ARG) (through `process::spawn`, with
//!    the cleared environment of `process::command`, as the thumbnail helper is), gives it the request on its standard
//!    input and reads its answer from its standard output, at most [`MAX_ANSWER`](super::request::MAX_ANSWER) bytes, which it
//!    then checks as it decodes it (`request::decode_answer`).
//! 3. At the deadline, [`GRACE`] after the time limit, the child is killed and the answer is [`GitError::TimedOut`]: what
//!    the status reads is bounded by that whatever gix does (a read of a pipe, a chain of deltas that spends the processor, a
//!    mount that does not answer). The app's exit flag ends the wait and kills the child as well, and a child never outlives
//!    the app (`PR_SET_PDEATHSIG` on Linux, the kill-on-close limit of the job on Windows). A child that ends without an
//!    answer, or with an error, is [`GitError::Repository`], except one that the processor-time limit ended, which is a
//!    time out.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use super::child::CHILD_ARG;
use super::request::{MAX_ANSWER, Request, decode_answer};
use super::scan::not_a_repository;
use super::{GitError, Limits, locate};
use crate::model::GitStatus;
use crate::process::{self, GRACE as REAP, Started, Wait};

/// How long a status is waited for after its time limit, before the child is killed (it also has the time to start, to read
/// the repository's first bytes and to answer).
pub(super) const GRACE: Duration = Duration::from_millis(500);

/// The environment variables the helper is given on top of the ones every child gets: it is the app's own executable, which
/// needs these to find the libraries it was started with (an AppImage sets `LD_LIBRARY_PATH` for the ones it bundles).
const KEPT_ENV: &[&str] = &["LD_LIBRARY_PATH"];

/// The committed memory the helper may have on Windows (the Job Object's limit), in bytes: what gix may hold for one status
/// (the rules, the index, the trees, objects up to 16 MiB each) with room to spare.
#[cfg(windows)]
const MEMORY: usize = 1024 * 1024 * 1024;

/// The status of the direct children of `folder` as the helper `program` (the app's own executable) reads it; see
/// [`super::status`].
pub(super) fn status(
    folder: &Path,
    limits: Limits,
    cancel: &AtomicBool,
    program: &Path,
) -> Result<GitStatus, GitError> {
    // The user's home folder is where the search for a repository ends (it is the last folder looked in).
    let home = std::env::home_dir().and_then(|home| gix::path::realpath(home).ok());

    let Some(request) = prepare(folder, home, limits, cancel)? else {
        return Ok(not_a_repository());
    };

    run(program, &request, cancel)
}

/// Finds and looks at the repository the folder `folder` is in: the request for the helper, or `None` when the folder is in
/// none, or in one that must not be read. `ceiling` is the folder the search must not go above (`None`: the filesystem root).
pub(super) fn prepare(
    folder: &Path,
    ceiling: Option<PathBuf>,
    limits: Limits,
    cancel: &AtomicBool,
) -> Result<Option<Request>, GitError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(GitError::Cancelled);
    }

    // Links are resolved first: the work tree has to be found as a parent of where the folder really is.
    let real = gix::path::realpath(folder).map_err(|error| {
        tracing::debug!(%error, "git status: the folder cannot be resolved");

        GitError::Folder(if error.is_not_found() {
            io::ErrorKind::NotFound
        } else {
            io::ErrorKind::Other
        })
    })?;
    match std::fs::metadata(&real) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(GitError::Folder(io::ErrorKind::NotADirectory)),
        Err(error) => return Err(GitError::Folder(error.kind())),
    }

    // The files of the repository are looked at, and what they point to, before anything reads one.
    let Some(layout) = locate::find(&real, ceiling.as_deref())? else {
        return Ok(None);
    };

    Ok(Some(Request {
        folder: real,
        limits,
        layout,
    }))
}

/// The command that starts the helper `program`: the cleared environment and the pipes of `process::command`, and the
/// standard input a pipe too.
fn command(program: &Path) -> std::process::Command {
    let mut command = process::command(program, [CHILD_ARG], KEPT_ENV);
    command.stdin(std::process::Stdio::piped());

    command
}

/// Starts `program` as the helper for `request` and returns what it answers (see the module).
pub(super) fn run(program: &Path, request: &Request, cancel: &AtomicBool) -> Result<GitStatus, GitError> {
    let bytes = request.encode().map_err(|error| {
        tracing::debug!(%error, "git status: the request cannot be written");

        GitError::Folder(io::ErrorKind::Other)
    })?;

    #[cfg(target_os = "linux")]
    {
        static NOTED: std::sync::Once = std::sync::Once::new();

        crate::view::heic::sandbox::note_kernel_once(&NOTED, "git status");
    }

    let mut child = process::spawn(&mut command(program)).map_err(|error| {
        tracing::debug!(%error, "git status: the helper did not start");

        GitError::Repository
    })?;

    // The job is made before a byte is sent, so the helper has nothing to run that is not in it. It is kept until the child
    // is reaped below; closing it ends whatever is still in it.
    #[cfg(windows)]
    let _job = match crate::view::heic::job::contain(&child, MEMORY) {
        Ok(job) => job,
        Err(error) => {
            tracing::debug!(%error, "git status: the helper cannot be contained");
            process::stop(&mut child);

            return Err(GitError::Repository);
        }
    };

    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        process::stop(&mut child);

        return Err(GitError::Repository);
    };

    let answer = supervise(&mut child, stdin, stdout, bytes, cancel, request.limits.time + GRACE);

    // Whatever happened above, the child is stopped and reaped here; harmless for one that has ended.
    process::stop(&mut child);

    answer
}

/// Feeds the child, waits for its answer until `deadline`, and decodes it.
fn supervise<C: Started>(
    child: &mut C,
    mut stdin: impl Write + Send + 'static,
    stdout: impl Read + Send + 'static,
    request: Vec<u8>,
    cancel: &AtomicBool,
    deadline: Duration,
) -> Result<GitStatus, GitError> {
    // The writer ends when everything is written (the pipe is closed with it) or when the child is gone. It is never
    // waited for: a child that does not read is killed, which ends the writer too.
    thread::Builder::new()
        .name("git-helper-stdin".to_owned())
        .spawn(move || {
            let _ = stdin.write_all(&request);
        })
        .map_err(|_| GitError::Repository)?;
    let reader = thread::Builder::new()
        .name("git-helper-stdout".to_owned())
        .spawn(move || read_answer(stdout))
        .map_err(|_| GitError::Repository)?;

    let waited = process::wait_for(child, cancel, deadline, || reader.is_finished());

    // The child is stopped in every case, and its output closes with it, which ends the reader. If it does not end (the pipe
    // is held open by something else) it is left behind.
    process::stop(child);
    let answer = process::join_within(reader, REAP);

    match waited {
        Ok(status) if status.success() => {}
        Ok(status) => return Err(after_a_signal(&status)),
        Err(Wait::Cancelled) => return Err(GitError::Cancelled),
        Err(Wait::Deadline) => {
            tracing::debug!("git status: the helper took too long");

            return Err(GitError::TimedOut);
        }
        Err(Wait::Stalled | Wait::Lost) => return Err(GitError::Repository),
    }

    match answer {
        Some(Ok(bytes)) => decode_answer(&bytes),
        _ => Err(GitError::Repository),
    }
}

/// What a helper that ended with an error and no answer of its own is: a time out when the processor-time limit ended it
/// (`SIGXCPU`, 24 on Linux and on macOS), and an unreadable repository otherwise (a crash, or the memory limit).
fn after_a_signal(status: &std::process::ExitStatus) -> GitError {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if status.signal() == Some(24) {
            return GitError::TimedOut;
        }
    }
    tracing::debug!(%status, "git status: the helper ended with an error");

    GitError::Repository
}

/// The answer from `input`: at most [`MAX_ANSWER`] bytes, and one more is read to tell a longer one.
fn read_answer(input: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    input.take(MAX_ANSWER + 1).read_to_end(&mut bytes)?;

    if bytes.len() as u64 > MAX_ANSWER {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the answer is too long"));
    }

    Ok(bytes)
}

#[cfg(all(test, unix))]
mod tests {
    use std::process::{Child, Stdio};
    use std::time::Instant;

    use super::*;

    fn start(program: &str, args: &[&str]) -> Child {
        let mut command = process::command(Path::new(program), args, &[]);
        command.stdin(Stdio::piped());

        process::spawn(&mut command).expect("a program of the system starts")
    }

    /// `supervise` over a program of the system that is told nothing and answers nothing (it is waited for, not read).
    fn supervised(
        program: &str,
        args: &[&str],
        cancel: &AtomicBool,
        deadline: Duration,
    ) -> (Result<GitStatus, GitError>, Child) {
        let mut child = start(program, args);
        let (stdin, stdout) = (child.stdin.take().expect("stdin"), child.stdout.take().expect("stdout"));
        let answer = supervise(&mut child, stdin, stdout, Vec::new(), cancel, deadline);

        (answer, child)
    }

    #[test]
    fn a_child_that_does_not_answer_is_killed_at_the_deadline_and_the_answer_is_the_time_limit() {
        let started = Instant::now();

        let (answer, mut child) = supervised("sleep", &["30"], &AtomicBool::new(false), Duration::from_millis(300));

        assert!(matches!(answer, Err(GitError::TimedOut)), "{answer:?}");
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
        // Killed, and `process::stop` (which `run` calls) reaps it: it is not left running.
        process::stop(&mut child);
        assert!(child.try_wait().expect("asked").is_some());
    }

    #[test]
    fn the_exit_flag_ends_the_wait_and_the_answer_is_cancelled() {
        let flag = AtomicBool::new(false);

        let (answer, mut child) = std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                flag.store(true, Ordering::Relaxed);
            });

            supervised("sleep", &["30"], &flag, Duration::from_secs(20))
        });
        process::stop(&mut child);

        assert!(matches!(answer, Err(GitError::Cancelled)), "{answer:?}");
    }

    #[test]
    fn a_child_that_ends_with_an_error_and_a_child_that_says_nothing_are_a_repository_that_cannot_be_read() {
        for program in ["false", "true"] {
            let (answer, _) = supervised(program, &[], &AtomicBool::new(false), Duration::from_secs(10));

            assert!(matches!(answer, Err(GitError::Repository)), "{program}: {answer:?}");
        }
    }

    #[test]
    fn an_answer_is_read_up_to_its_cap_and_one_byte_more_and_no_further() {
        // An answer that never ends is not read to its end.
        assert!(read_answer(io::repeat(1)).is_err());

        let exact = vec![0_u8; MAX_ANSWER as usize];
        assert_eq!(read_answer(&exact[..]).expect("at the cap").len() as u64, MAX_ANSWER);
    }
}
