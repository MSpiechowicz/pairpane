//! Running the OpenSSH client for what it prints, and finding it.
//!
//! The program is a fixed binary started with an explicit argument list: no shell is ever involved, so no text from the
//! dialog or the configuration is read as a command line. It is started the way every child of the core is
//! ([`pairpane_core::process`]: no input, no window), what it writes to its error output is dropped (it may hold host
//! names and paths; nothing of it is kept or logged), what it prints is read up to a limit, and it is stopped when it
//! takes too long. It leads a group of its own (a job on Windows), which is ended with it: `ssh -G` runs the commands of a
//! `Match exec` of the user's configuration, and one that outlives the limit does not outlive the call.
//!
//! The environment is the app's own, so that the answer is the one the user's own `ssh` gets (a `Match exec` that tests a
//! variable, a `${XDG_RUNTIME_DIR}` in a path, the `SHELL` that runs a command): cleaned for an AppImage (its own
//! variables and the folders of its mount are taken out, as for every program the app starts for the user), and without
//! the variables that change how a program is loaded (`LD_*`, `DYLD_*`).

use std::ffi::OsStr;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use pairpane_core::process::{self, Group, Wait};
use pairpane_core::shell::inherit_cleaned_env;

use crate::error::fail;

/// The most that is read of what the program prints (a full `ssh -G` is a few kilobytes).
pub(super) const MAX_OUTPUT: usize = 256 * 1024;

/// How long the program may take.
pub(super) const LIMIT: Duration = Duration::from_secs(5);

/// Whether the variable `name` is one that changes how a program is loaded (`LD_PRELOAD`, `DYLD_INSERT_LIBRARIES`): not
/// given to the program, whatever the app's environment holds.
fn loads_code(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| name.starts_with("LD_") || name.starts_with("DYLD_"))
}

/// The name of the program in a folder of `PATH`.
const NAME: &str = if cfg!(windows) { "ssh.exe" } else { "ssh" };

/// The well-known places of the client, in the order they are looked at: where an operating system or a package
/// manager puts it.
fn well_known() -> Vec<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("SystemRoot")
            .map(|root| PathBuf::from(root).join("System32").join("OpenSSH").join(NAME))
            .into_iter()
            .collect();
    }

    [
        "/usr/bin/ssh",
        "/usr/local/bin/ssh",
        "/opt/homebrew/bin/ssh",
        "/run/current-system/sw/bin/ssh",
    ]
    .map(PathBuf::from)
    .to_vec()
}

/// The OpenSSH client of this machine: the first of the well-known places that has one, then the first `ssh` in `PATH`
/// (only in its folders that are given by an absolute path). `None` when there is none.
pub(crate) fn find_ssh() -> Option<PathBuf> {
    well_known().into_iter().find(|path| path.is_file()).or_else(|| {
        let path = std::env::var_os("PATH")?;

        std::env::split_paths(&path)
            .filter(|folder| folder.is_absolute())
            .map(|folder| folder.join(NAME))
            .find(|candidate| candidate.is_file())
    })
}

/// Runs `program` with `args` and gives what it printed. Blocks the calling thread for `limit` at the most, so it is
/// not for an async task.
///
/// Errors (fixed messages): the program could not be started (`Other`), took longer than `limit` (`TimedOut`), printed
/// more than [`MAX_OUTPUT`] (`InvalidData`), or ended with a failure (`InvalidInput`: for `ssh -G` this is a
/// configuration it cannot read).
pub(super) fn run<I, S>(program: &Path, args: I, limit: Duration) -> io::Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = process::command(program, args, &[]);

    inherit_cleaned_env(&mut command, loads_code);

    let mut group = Group::start(&mut command).map_err(|_| not_started())?;
    let stdout = group.child_mut().stdout.take().ok_or_else(not_started)?;
    let too_long = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&too_long);

    // The reading has a thread of its own: a program that prints nothing and never ends must not hold this one.
    let reader = thread::Builder::new()
        .name("pairpane-ssh-output".to_owned())
        .spawn(move || {
            let mut bytes = Vec::new();

            stdout.take(MAX_OUTPUT as u64 + 1).read_to_end(&mut bytes)?;
            flag.store(bytes.len() > MAX_OUTPUT, Ordering::Release);

            Ok::<_, io::Error>(bytes)
        })
        .map_err(|_| not_started())?;

    // Raised by the reader itself, so that a program that prints without end is not waited for.
    let waited = process::wait_for(&mut group, &too_long, limit, || reader.is_finished());

    // The group is ended whatever the way out (so is the pipe the reader holds), and the child is reaped.
    process::stop(&mut group);

    let bytes = process::join_within(reader, process::GRACE);

    if too_long.load(Ordering::Acquire) {
        return Err(fail(io::ErrorKind::InvalidData, "ssh's answer is too long"));
    }

    let status = match waited {
        Ok(status) => status,
        Err(Wait::Deadline) => return Err(too_slow()),
        Err(_) => return Err(unreadable()),
    };
    let Some(Ok(bytes)) = bytes else {
        return Err(unreadable());
    };

    if !status.success() {
        return Err(fail(
            io::ErrorKind::InvalidInput,
            "ssh could not read its configuration for this host",
        ));
    }

    Ok(bytes)
}

fn not_started() -> io::Error {
    fail(io::ErrorKind::Other, "ssh could not be started")
}

fn unreadable() -> io::Error {
    fail(io::ErrorKind::Other, "ssh's answer could not be read")
}

fn too_slow() -> io::Error {
    fail(io::ErrorKind::TimedOut, "ssh did not answer in time")
}

#[cfg(all(test, unix))]
mod tests {
    use std::time::Instant;

    use super::*;

    /// A standard tool of the system, by its file name.
    fn tool(name: &str) -> Option<PathBuf> {
        ["/bin", "/usr/bin"]
            .iter()
            .map(|folder| Path::new(folder).join(name))
            .find(|path| path.is_file())
    }

    #[test]
    fn what_the_program_prints_comes_back_and_the_arguments_are_not_read_by_a_shell() {
        let Some(echo) = tool("echo") else {
            return;
        };
        let text = "$(touch nothing) `x` ; a  b";
        let printed = run(&echo, ["-n", text], LIMIT).expect("ran");

        assert_eq!(String::from_utf8(printed).expect("text"), text);
    }

    #[test]
    fn the_variables_that_change_how_a_program_is_loaded_are_the_only_ones_left_out() {
        for name in [
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "DYLD_INSERT_LIBRARIES",
            "DYLD_LIBRARY_PATH",
        ] {
            assert!(loads_code(OsStr::new(name)), "{name}");
        }

        for name in [
            "PATH",
            "SHELL",
            "LANG",
            "LC_ALL",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
            "OLD_LD_X",
            "LDFLAGS",
        ] {
            assert!(!loads_code(OsStr::new(name)), "{name}");
        }
    }

    #[test]
    fn the_program_is_given_the_environment_of_the_app_and_not_what_loads_code() {
        let Some(env) = tool("env") else {
            return;
        };
        let printed = String::from_utf8(run(&env, ["-0"], LIMIT).expect("ran")).expect("text");
        let given: Vec<&str> = printed
            .split('\0')
            .filter_map(|entry| entry.split_once('=').map(|(name, _)| name))
            .collect();

        // Cargo sets these for a test: they are not among the few variables a stripped environment would keep.
        assert!(given.contains(&"CARGO_MANIFEST_DIR"), "{given:?}");

        for (name, _) in std::env::vars_os() {
            if let Some(name) = name.to_str().filter(|name| !name.contains('=')) {
                assert_eq!(given.contains(&name), !loads_code(OsStr::new(name)), "{name}");
            }
        }
    }

    #[test]
    fn a_program_that_fails_is_a_configuration_that_cannot_be_read() {
        let Some(program) = tool("false") else {
            return;
        };
        let error = run(&program, ["x"; 0], LIMIT).expect_err("failed");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "ssh could not read its configuration for this host");
    }

    #[test]
    fn a_program_that_does_not_end_is_stopped_at_the_limit() {
        let Some(sleep) = tool("sleep") else {
            return;
        };
        let started = Instant::now();
        let error = run(&sleep, ["30"], Duration::from_millis(200)).expect_err("too slow");

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), "ssh did not answer in time");
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    }

    #[test]
    fn a_program_that_prints_without_end_is_stopped_at_the_cap() {
        let Some(yes) = tool("yes") else {
            return;
        };
        let started = Instant::now();
        let error = run(&yes, ["x"; 0], LIMIT).expect_err("too long");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(started.elapsed() < LIMIT, "{:?}", started.elapsed());
    }

    /// Whether the process `pid` is running (a process that has ended and waits to be reaped is not).
    fn running(pid: i32) -> bool {
        let Some(pid) = rustix::process::Pid::from_raw(pid) else {
            return false;
        };

        if rustix::process::test_kill_process(pid).is_err() {
            return false;
        }

        // On Linux a process that was killed may stay a zombie (`Z`, or `X` once it is dead) until its parent reaps it,
        // and one that was reaped between the two looks is gone (where there is a `/proc` at all).
        match std::fs::read_to_string(format!("/proc/{}/stat", pid.as_raw_nonzero())) {
            Ok(stat) => stat
                .rsplit(')')
                .next()
                .is_some_and(|rest| !rest.trim_start().starts_with(['Z', 'X'])),
            Err(_) => !Path::new("/proc/self").exists(),
        }
    }

    #[test]
    fn a_command_of_the_configuration_that_outlives_the_limit_does_not_outlive_the_call() {
        let Some(ssh) = crate::tests::ssh_program() else {
            return;
        };
        let dir = tempfile::tempdir().expect("temp dir");
        let pid_file = dir.path().join("pid");
        let config = dir.path().join("config");

        // The text of `ssh_config`: the command is run by OpenSSH when it works out which blocks apply.
        std::fs::write(
            &config,
            format!(
                "Match exec \"echo $$ > {}; exec sleep 30\"\n  HostName x\n",
                pid_file.display()
            ),
        )
        .expect("config");

        let started = Instant::now();
        let error = run(
            &ssh,
            [OsStr::new("-G"), OsStr::new("-F"), config.as_os_str(), OsStr::new("h")],
            Duration::from_secs(2),
        )
        .expect_err("too slow");

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());

        let pid: i32 = std::fs::read_to_string(&pid_file)
            .expect("the command ran")
            .trim()
            .parse()
            .expect("a process number");
        let until = Instant::now() + Duration::from_secs(5);

        while running(pid) && Instant::now() < until {
            thread::sleep(Duration::from_millis(20));
        }

        assert!(!running(pid), "the command is still running");
    }

    #[test]
    fn a_program_that_is_not_there_does_not_start() {
        let error = run(Path::new("/nonexistent/ssh"), ["x"; 0], LIMIT).expect_err("missing");

        assert_eq!(error.to_string(), "ssh could not be started");
    }
}
