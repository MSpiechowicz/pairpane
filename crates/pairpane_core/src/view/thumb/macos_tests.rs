//! The thumbnail helper's sandbox on macOS, tried for real: the test starts the test program itself, which confines
//! itself as the helper does and then reports what it can still do. (Confining the process of the test would confine the
//! other tests with it.)
#![allow(unsafe_code, reason = "`fcntl` in the check that a descriptor was closed; see there")]

use std::fs::File;
use std::net::TcpListener;
use std::os::fd::{AsRawFd, IntoRawFd};
use std::process::Command;

use image::ImageFormat::Png;
use rustix::process::{Resource, getrlimit};

use super::child::thumbnail;
use super::fixtures::{encode, is_color, size_of};
use super::macos::confine;
use crate::view::heic::HEADER;
use crate::view::testing::run_probe;

/// Set in the test program that plays the helper.
const PROBE: &str = "PAIRPANE_THUMBNAIL_SANDBOX_PROBE";

/// The name of the test that is the probe when [`PROBE`] is set.
const TEST: &str = "view::thumb::macos_tests::the_helper_confines_itself";

/// What the confined process reports, one line.
fn probe() -> String {
    // A file in a folder of the user's (the temporary folder is one), a descriptor of it that the parent leaked (inheritable,
    // and no object of this program owns it) and one that this program owns (close-on-exec, as the standard library opens
    // everything): what the helper could reach if it were not confined, and what it must not close behind its owner.
    let folder = tempfile::tempdir().expect("a folder");
    let path = folder.path().join("secret");
    std::fs::write(&path, b"secret").expect("a file");
    let owned = File::open(&path).expect("an open file");
    let owned_fd = owned.as_raw_fd();
    let leaked = File::open(&path).expect("an open file").into_raw_fd();
    // SAFETY: `fcntl` with `F_SETFD` takes the descriptor and the flags by value; `leaked` is open and owned by no object.
    assert_eq!(unsafe { libc::fcntl(leaked, libc::F_SETFD, 0) }, 0, "made inheritable");
    let picture = encode(Png, 200, 100);
    // Used once before the sandbox, as the helper's standard streams are.
    std::io::Write::flush(&mut std::io::stdout()).expect("flushed");

    confine().expect("the helper confines itself");

    // SAFETY: `fcntl` with `F_GETFD` takes the descriptor by value and only asks about it.
    let closed = unsafe { libc::fcntl(leaked, libc::F_GETFD) } == -1;
    // SAFETY: as above.
    let kept = unsafe { libc::fcntl(owned_fd, libc::F_GETFD) } != -1;
    let opens = File::open(&path).is_ok();
    let lists = std::fs::read_dir(folder.path()).is_ok();
    let creates = File::create(folder.path().join("new")).is_ok();
    let listens = TcpListener::bind("127.0.0.1:0").is_ok();
    let starts = Command::new("/usr/bin/true").status().is_ok();
    let decodes =
        thumbnail(&picture, 64).is_ok_and(|answer| size_of(&answer) == (64, 32) && is_color(&answer[HEADER..], 0));
    let limits = [Resource::Core, Resource::Cpu, Resource::Nofile].map(|resource| getrlimit(resource).current);

    // The owner of the descriptor that was kept closes it: no "IO Safety violation".
    drop(owned);

    format!(
        "PROBE closed={closed} kept={kept} opens={opens} lists={lists} creates={creates} listens={listens} starts={starts} \
         decodes={decodes} limits={limits:?}"
    )
}

#[test]
fn the_helper_confines_itself() {
    if std::env::var_os(PROBE).is_some() {
        println!("{}", probe());
        // The test harness would make calls the sandbox refuses on its way out.
        std::process::exit(0);
    }

    let probe = run_probe(PROBE, TEST);
    let report = probe.report.as_str();

    assert!(probe.success, "{}", probe.output);
    assert!(
        report.contains("closed=true"),
        "a descriptor of the app is closed: {report}"
    );
    assert!(
        report.contains("kept=true"),
        "a descriptor the program owns is not closed behind its owner: {report}"
    );
    assert!(report.contains("opens=false"), "no file is opened: {report}");
    assert!(report.contains("lists=false"), "no folder is listed: {report}");
    assert!(report.contains("creates=false"), "no file is created: {report}");
    assert!(report.contains("listens=false"), "no socket is made: {report}");
    assert!(report.contains("starts=false"), "no program is started: {report}");
    assert!(report.contains("limits=[Some(0), Some(60), Some(64)]"), "{report}");
    // And what the helper is for still works in the sandbox.
    assert!(report.contains("decodes=true"), "a picture is still decoded: {report}");
}
