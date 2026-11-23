//! What the tests of the two helper processes (`heic_helper.rs`, `thumb_helper.rs`) share: feeding a helper and reading its
//! answer, and looking at how a helper has confined itself (Linux).
#![allow(dead_code, reason = "each test program uses only a part of this")]

use std::io::{Read, Write};
use std::process::Child;

/// What `child` answers to `input`: what it wrote to the standard output, and whether it exited with 0.
pub fn finish(mut child: Child, input: &[u8]) -> (Vec<u8>, bool) {
    // The inputs here are small enough for the pipe, so writing everything first cannot block. A helper that refuses
    // the input early closes the pipe, which is not a failure of the test.
    let mut stdin = child.stdin.take().expect("stdin");
    let _ = stdin.write_all(input);
    drop(stdin);

    let mut answer = Vec::new();
    child
        .stdout
        .take()
        .expect("stdout")
        .read_to_end(&mut answer)
        .expect("the answer is read");
    let status = child.wait().expect("the helper ends");

    (answer, status.success())
}

/// The limits of the process at `proc` (`/proc/<pid>`) once its address space is limited and every thread has the seccomp
/// filter, which are the last steps of the confinement; the helper then waits for its input.
#[cfg(target_os = "linux")]
pub fn wait_until_confined(proc: &str) -> String {
    let limits = || std::fs::read_to_string(format!("{proc}/limits")).unwrap_or_default();
    let limited = |text: &str| {
        text.lines()
            .any(|line| line.starts_with("Max address space") && !line.contains("unlimited"))
    };
    let filtered = || {
        let Ok(tasks) = std::fs::read_dir(format!("{proc}/task")) else {
            return false;
        };

        tasks.filter_map(Result::ok).all(|task| {
            std::fs::read_to_string(task.path().join("status")).is_ok_and(|status| status.contains("Seccomp:\t2"))
        })
    };

    for _ in 0..500 {
        if limited(&limits()) && filtered() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(limited(&limits()), "the helper limits its address space: {}", limits());
    assert!(filtered(), "every thread of the helper has the seccomp filter");

    limits()
}

/// Asserts that the process at `proc`, whose limits are `limits` (from [`wait_until_confined`]), has the limits every
/// helper sets for itself and can never gain privileges.
#[cfg(target_os = "linux")]
pub fn assert_limited(proc: &str, limits: &str) {
    for line in [
        "Max cpu time              60 ",
        "Max core file size        0 ",
        "Max open files            64 ",
    ] {
        assert!(limits.contains(line), "{limits}");
    }

    let status = std::fs::read_to_string(format!("{proc}/status")).expect("status");
    assert!(status.contains("NoNewPrivs:\t1"), "{status}");
}

/// The descriptors the process at `proc` holds, in order.
#[cfg(target_os = "linux")]
pub fn open_descriptors(proc: &str) -> Vec<u32> {
    let mut open: Vec<u32> = std::fs::read_dir(format!("{proc}/fd"))
        .expect("the descriptors")
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    open.sort_unstable();

    open
}
