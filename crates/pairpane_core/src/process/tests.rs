//! The shared child-process plumbing: the environment a child gets, and how a waiting parent ends a child that runs on.
//! Unix tools stand in for the children.

use std::time::Duration;

#[cfg(unix)]
mod unix {
    use std::path::Path;
    use std::process::Stdio;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use pretty_assertions::assert_eq;

    use crate::process::{Wait, command, stop, wait_for};

    /// What `/usr/bin/env` prints when started like a child (so only what the child is given).
    fn environment(kept: &[&str]) -> String {
        let output = command(Path::new("/usr/bin/env"), ["-0"], kept)
            .output()
            .expect("env runs");

        String::from_utf8_lossy(&output.stdout).replace('\0', "\n")
    }

    #[test]
    fn a_child_gets_only_the_variables_that_are_named() {
        let bare = environment(&[]);
        assert_eq!(bare.trim(), "", "nothing of the app's own");

        let kept = environment(&["PATH"]);
        assert!(kept.starts_with("PATH="), "{kept}");
        assert_eq!(kept.lines().count(), 1, "{kept}");
    }

    #[test]
    fn a_child_has_no_input_and_starts_in_the_root() {
        let mut child = command(Path::new("/bin/pwd"), [""; 0], &[]).spawn().expect("pwd runs");
        let mut out = String::new();
        std::io::Read::read_to_string(&mut child.stdout.take().expect("piped"), &mut out).expect("read");
        child.wait().expect("ends");

        assert_eq!(out.trim(), "/");

        // The standard input is closed, so a program that reads it sees the end at once.
        let status = command(Path::new("/bin/cat"), [""; 0], &[])
            .stdout(Stdio::null())
            .status()
            .expect("cat runs");
        assert!(status.success());
    }

    #[test]
    fn a_child_that_ends_is_waited_for_with_its_status() {
        let mut child = command(Path::new("/usr/bin/false"), [""; 0], &[])
            .spawn()
            .expect("false runs");
        let status = wait_for(&mut child, &AtomicBool::new(false), Duration::from_secs(10), || true).expect("ended");
        assert_eq!(status.code(), Some(1));
    }

    #[test]
    fn a_child_that_runs_on_is_given_up_on_at_the_flag_and_at_the_deadline() {
        let started = Instant::now();
        let mut child = command(Path::new("/usr/bin/sleep"), ["30"], &[])
            .spawn()
            .expect("sleep runs");

        let flag = AtomicBool::new(false);
        let waited = std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                flag.store(true, Ordering::Relaxed);
            });

            wait_for(&mut child, &flag, Duration::from_secs(20), || true)
        });
        assert_eq!(waited.expect_err("cancelled"), Wait::Cancelled);

        let waited = wait_for(&mut child, &AtomicBool::new(false), Duration::from_millis(200), || true);
        assert_eq!(waited.expect_err("too long"), Wait::Deadline);

        stop(&mut child);
        assert!(child.try_wait().expect("reaped").is_some());
        assert!(started.elapsed() < Duration::from_secs(10), "stopped, not waited for");
    }

    #[test]
    fn an_output_that_does_not_end_is_a_stall_after_the_grace() {
        let mut child = command(Path::new("/usr/bin/true"), [""; 0], &[])
            .spawn()
            .expect("true runs");

        let waited = wait_for(&mut child, &AtomicBool::new(false), Duration::from_secs(30), || false);

        assert_eq!(waited.expect_err("stalled"), Wait::Stalled);
    }
}

#[test]
fn a_thread_that_does_not_end_is_left_behind_and_one_that_ends_is_joined() {
    use std::time::Instant;

    use pretty_assertions::assert_eq;

    use crate::process::join_within;

    let (stop, wait) = std::sync::mpsc::channel::<()>();
    let stuck = std::thread::spawn(move || {
        let _ = wait.recv();
        5
    });
    let started = Instant::now();

    assert_eq!(join_within(stuck, Duration::from_millis(100)), None);
    assert!(started.elapsed() < Duration::from_secs(5), "not waited for");
    drop(stop);

    assert_eq!(join_within(std::thread::spawn(|| 7), Duration::from_secs(5)), Some(7));
}
