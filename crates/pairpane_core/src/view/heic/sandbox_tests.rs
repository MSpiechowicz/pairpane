//! The helper's confinement, tried for real: the test starts the test program itself, which confines itself as the
//! helper does and then reports what it can still do. (Confining the process of the test would confine the other
//! tests with it.)

use std::fs::File;
use std::net::{TcpListener, UdpSocket};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::Command;

use rustix::process::{Resource, getrlimit};

use super::kernel::{io_uring_setup_errno, refused_errnos};
use super::sandbox::{confine, limit};

/// Set in the test program that plays the helper.
const PROBE: &str = "PAIRPANE_SANDBOX_PROBE";

/// The name of the test that is the probe when [`PROBE`] is set.
const TEST: &str = "view::heic::sandbox_tests::the_helper_confines_itself";

/// What the confined process reports, one line.
fn probe() -> String {
    limit().expect("the limits are set");

    // A listener on a path socket and a UDP socket made before the confinement: what the helper could reach if it could
    // make a socket.
    let folder = tempfile::tempdir().expect("a folder");
    let path = folder.path().join("socket");
    let _listener = UnixListener::bind(&path).expect("a path socket");
    let _udp = UdpSocket::bind("127.0.0.1:0").expect("a UDP socket");

    // The flag belongs to the thread, which the test runs on.
    let no_new_privs = std::fs::read_to_string("/proc/thread-self/status")
        .expect("status")
        .lines()
        .any(|line| line.starts_with("NoNewPrivs:") && line.ends_with('1'));
    confine().expect("the helper confines itself");

    let limits = [Resource::Core, Resource::Cpu, Resource::Nofile].map(|resource| getrlimit(resource).current);
    let space = getrlimit(Resource::As).current.expect("a limit on the address space");
    let opens = File::open("/proc/self/status").is_ok();
    let listens = TcpListener::bind("127.0.0.1:0").is_ok();
    let connects = UnixStream::connect(&path).is_ok();
    let sends = UdpSocket::bind("127.0.0.1:0").is_ok();
    let starts = Command::new("/bin/true").status().is_ok();
    // The refusals are the filter's, not some other failure.
    let socket_errno = UdpSocket::bind("127.0.0.1:0")
        .err()
        .and_then(|error| error.raw_os_error());
    let ring_errno = io_uring_setup_errno();
    let threads = std::thread::spawn(|| 7).join().is_ok_and(|value| value == 7);
    let refused = refused_errnos();

    format!(
        "PROBE no_new_privs={no_new_privs} limits={limits:?} space={} opens={opens} listens={listens} \
         connects={connects} sends={sends} starts={starts} socket_errno={} ring_errno={ring_errno} threads={threads} \
         refused={refused:?}",
        space > 3 << 30,
        socket_errno.unwrap_or(0),
    )
}

#[test]
fn the_helper_confines_itself() {
    if std::env::var_os(PROBE).is_some() {
        println!("{}", probe());

        return;
    }

    let output = Command::new(std::env::current_exe().expect("the test program"))
        .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
        .env(PROBE, "1")
        .output()
        .expect("the probe starts");
    let text = String::from_utf8_lossy(&output.stdout);
    // The report follows the name of the test on its line.
    let report = text
        .lines()
        .find_map(|line| line.split_once("PROBE "))
        .map_or("", |(_, report)| report);

    assert!(
        output.status.success(),
        "{:?} {text} {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(report.contains("no_new_privs=true"), "{report}");
    assert!(report.contains("limits=[Some(0), Some(60), Some(64)]"), "{report}");
    assert!(report.contains("space=true"), "{report}");

    // The seccomp filter needs no Landlock: no socket of any kind (a path socket and UDP, which Landlock may leave open)
    // and no new program, on every kernel.
    assert!(
        report.contains("connects=false"),
        "no path socket can be reached: {report}"
    );
    assert!(report.contains("sends=false"), "no UDP socket can be made: {report}");
    assert!(report.contains("starts=false"), "no program can be started: {report}");
    let eperm = libc::EPERM;
    assert!(
        report.contains(&format!("socket_errno={eperm} ")),
        "the filter refuses sockets: {report}"
    );
    assert!(
        report.contains(&format!("ring_errno={eperm} ")),
        "the filter refuses io_uring: {report}"
    );
    assert!(
        report.contains("threads=true"),
        "threads can still be started: {report}"
    );
    // fork (clone without CLONE_THREAD), clone3, System V shared memory, a signal to and a look at the limits of the
    // parent.
    let refused = format!("refused=[{eperm}, {}, {eperm}, {eperm}, {eperm}]", libc::ENOSYS);
    assert!(report.contains(&refused), "the filter refuses them: {report}");

    // The kernel needs Landlock (Linux 5.13 and a kernel that has it switched on) for the rest.
    if report.contains("opens=true") {
        eprintln!("no Landlock on this kernel, the file system was not closed: {report}");

        return;
    }
    assert!(report.contains("opens=false"), "no file can be opened: {report}");

    // The network rights are Landlock's fourth ABI, Linux 6.7.
    if kernel_at_least(6, 7) {
        assert!(report.contains("listens=false"), "no port can be bound: {report}");
    }
}

/// Whether the running kernel is `major.minor` or newer.
fn kernel_at_least(major: u32, minor: u32) -> bool {
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let mut numbers = release
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|part| part.parse::<u32>().ok());

    (numbers.next(), numbers.next()) >= (Some(major), Some(minor))
}
