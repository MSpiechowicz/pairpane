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

    use crate::process::{Exit, Started, Wait, command, limit_for, stop, wait_for, wait_for_exit};

    /// What `/usr/bin/env` prints when started like a child (so only what the child is given).
    fn environment(kept: &[&str]) -> String {
        let output = command(Path::new("/usr/bin/env"), ["-0"], kept)
            .output()
            .expect("env runs");

        String::from_utf8_lossy(&output.stdout).replace('\0', "\n")
    }

    #[test]
    fn the_hard_limit_on_processor_time_is_a_second_above_the_soft_one_and_the_others_are_equal() {
        use rustix::process::Resource;

        // The signal of the soft limit (`SIGXCPU`) is what ends a process, and not the `SIGKILL` of a hard limit that is the same.
        let cpu = limit_for(Resource::Cpu, 6);
        assert_eq!((cpu.current, cpu.maximum), (Some(6), Some(7)));

        for resource in [Resource::Nofile, Resource::Core, Resource::Data] {
            let limit = limit_for(resource, 6);

            assert_eq!((limit.current, limit.maximum), (Some(6), Some(6)));
        }
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
        let mut child = command(Path::new("/bin/sleep"), ["30"], &[])
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
    fn a_child_without_a_reader_may_run_longer_than_the_grace() {
        let mut child = command(Path::new("/bin/sleep"), ["3"], &[])
            .spawn()
            .expect("sleep runs");

        let waited = wait_for_exit(&mut child, &AtomicBool::new(false), Duration::from_secs(10), || Ok(()));

        assert!(waited.expect("ended").success());
    }

    #[test]
    fn a_failed_check_gives_up_with_its_error_and_the_flag_and_the_deadline_still_do() {
        let mut child = command(Path::new("/bin/sleep"), ["30"], &[])
            .spawn()
            .expect("sleep runs");
        let cancel = AtomicBool::new(false);

        let waited = wait_for_exit(&mut child, &cancel, Duration::from_secs(20), || {
            Err(std::io::Error::from(std::io::ErrorKind::StorageFull))
        });
        let Err(Exit::Watch(error)) = waited else {
            panic!("the check ends the wait");
        };
        assert_eq!(error.kind(), std::io::ErrorKind::StorageFull);

        let waited = wait_for_exit(&mut child, &cancel, Duration::from_millis(100), || Ok(()));
        assert!(matches!(waited, Err(Exit::Gave(Wait::Deadline))));

        stop(&mut child);
    }

    #[test]
    fn a_group_leads_its_own_group_and_is_ended_when_it_is_dropped_or_stopped() {
        use rustix::process::{Pid, getpgid, test_kill_process};

        use crate::process::Group;

        let mut group = Group::start(&mut command(Path::new("/bin/sleep"), ["30"], &[])).expect("sleep runs");
        let pid = Pid::from_child(group.child_mut());

        assert_eq!(
            getpgid(Some(pid)).expect("a group"),
            pid,
            "the child leads a group of its own"
        );

        // Not ended yet; a wait that gives up leaves it running until it is stopped.
        let waited = wait_for(&mut group, &AtomicBool::new(false), Duration::from_millis(100), || true);
        assert_eq!(waited.expect_err("too long"), Wait::Deadline);
        assert!(test_kill_process(pid).is_ok());

        stop(&mut group);
        assert!(Started::try_wait(&mut group).expect("reaped").is_some());

        // Dropped, it is ended and reaped.
        let mut group = Group::start(&mut command(Path::new("/bin/sleep"), ["30"], &[])).expect("sleep runs");
        let pid = Pid::from_child(group.child_mut());

        drop(group);
        assert!(test_kill_process(pid).is_err(), "the child was reaped");
    }

    /// The processes of the group `pgid` that are running (the entries of `/proc` that have not ended; one that waits to be
    /// reaped does not count).
    #[cfg(target_os = "linux")]
    fn members(pgid: i32) -> Vec<i32> {
        std::fs::read_dir("/proc")
            .expect("proc")
            .filter_map(|entry| {
                let pid: i32 = entry.ok()?.file_name().to_str()?.parse().ok()?;
                let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
                // After the name (which may hold anything): state, parent, group.
                let mut fields = stat.rsplit_once(')')?.1.split_whitespace();
                let state = fields.next()?;
                let group: i32 = fields.nth(1)?.parse().ok()?;

                (group == pgid && state != "Z").then_some(pid)
            })
            .collect()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_wait_ends_what_the_child_left_in_its_group_before_it_reaps_the_child() {
        use rustix::process::{Pid, Signal, kill_process};

        use crate::process::Group;

        let Some(xargs) = ["/usr/bin/xargs", "/bin/xargs"]
            .iter()
            .map(Path::new)
            .find(|path| path.is_file())
        else {
            return;
        };
        // With no input `xargs` (GNU) runs the program once, as a child of its own, in the group.
        let mut group = Group::start(&mut command(xargs, ["/bin/sleep", "30"], &[])).expect("xargs runs");
        let pid = Pid::from_child(group.child_mut());
        let pgid = pid.as_raw_nonzero().get();
        let until = Instant::now() + Duration::from_secs(5);

        while members(pgid).len() < 2 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(members(pgid).len(), 2, "the leader and the program it started");

        // The leader ends by itself and leaves its child behind, as a program that is killed does.
        kill_process(pid, Signal::KILL).expect("signalled");
        Started::wait(&mut group).expect("waited");

        let until = Instant::now() + Duration::from_secs(5);

        while !members(pgid).is_empty() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(members(pgid), Vec::<i32>::new(), "the group was ended by the wait");
    }

    #[test]
    fn a_group_whose_child_ends_gives_its_status_and_can_be_stopped_again() {
        use crate::process::Group;

        let mut group = Group::start(&mut command(Path::new("/usr/bin/false"), [""; 0], &[])).expect("false runs");
        let status = wait_for(&mut group, &AtomicBool::new(false), Duration::from_secs(10), || true).expect("ended");

        assert_eq!(status.code(), Some(1));

        // Reaped: a stop afterwards signals nothing and is harmless.
        stop(&mut group);
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

#[cfg(windows)]
#[test]
fn a_child_starts_in_the_system_folder_and_not_in_the_apps_own() {
    let command = super::command(std::path::Path::new("tool.exe"), [""; 0], &[]);
    let system = std::env::var_os("SystemRoot");

    assert_eq!(command.get_current_dir(), system.as_deref().map(std::path::Path::new));
}
