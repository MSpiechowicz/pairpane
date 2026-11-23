//! The AppContainer of the thumbnail helper, tried for real: the test starts the test program itself in the container
//! (`appcontainer::start`, as the helper is started), and the program, finding itself in a container, reports what it
//! can still do. The probe is a test of the test program; nothing of it is in the app.
#![allow(unsafe_code, reason = "the token query of `in_app_container`; see there")]

use std::ffi::c_void;
use std::fs::File;
use std::io::Read;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::Command;
use std::time::Duration;

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TokenIsAppContainer};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::MEMORY;
use super::appcontainer::start;
use crate::process::Started;

/// The name of the test that is the probe in the container.
const TEST: &str = "view::thumb::appcontainer_tests::the_helper_runs_in_its_container";

/// Whether this process has the token of an AppContainer.
fn in_app_container() -> bool {
    let mut token = std::ptr::null_mut();

    // SAFETY: the pseudo handle of this process is always valid; `token` is a place for the process's token, set on success
    // and closed below.
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) };
    if opened == 0 {
        return false;
    }

    let mut value = 0_u32;
    let mut size = 0_u32;

    // SAFETY: the token is live and has the right to be asked; `value` is the four bytes the class `TokenIsAppContainer`
    // gives (a `DWORD`) and `size` the place for the length.
    let asked = unsafe {
        GetTokenInformation(
            token,
            TokenIsAppContainer,
            (&raw mut value).cast::<c_void>(),
            size_of::<u32>() as u32,
            &raw mut size,
        )
    };
    // SAFETY: the token was opened above and is closed here once.
    unsafe { CloseHandle(token) };

    asked != 0 && value == 1
}

/// The text of `bytes` as hexadecimal, so that a path with a space in it is one argument of the command line.
fn hex(text: &str) -> String {
    text.bytes().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> String {
    let bytes: Vec<u8> = (0..text.len() / 2)
        .filter_map(|at| u8::from_str_radix(&text[2 * at..2 * at + 2], 16).ok())
        .collect();

    String::from_utf8_lossy(&bytes).into_owned()
}

/// What the process in the container reports, one line: its last two arguments name a file of the user's and the port of
/// a listener of the app on the loopback.
fn probe() -> String {
    let arguments: Vec<String> = std::env::args().collect();
    let secret = unhex(&arguments[arguments.len() - 2]);
    let port: u16 = arguments[arguments.len() - 1].parse().expect("a port");

    let opens = File::open(secret).is_ok();
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let connects = TcpStream::connect_timeout(&address, Duration::from_secs(3)).is_ok();
    // A program of the system that any user may run, by its whole path (the helper has no search path).
    let root = std::env::var_os("SystemRoot").expect("the system folder");
    let program = std::path::Path::new(&root).join("System32").join("hostname.exe");
    let starts = Command::new(program).status().is_ok();

    format!("PROBE container=true opens={opens} connects={connects} starts={starts}")
}

#[test]
fn the_helper_runs_in_its_container() {
    if in_app_container() {
        println!("{}", probe());
        // The test harness would make calls the container refuses on its way out.
        std::process::exit(0);
    }

    // A file the user may read, and a listener the user may connect to: what the container is to be refused.
    let folder = tempfile::tempdir().expect("a folder");
    let secret = folder.path().join("secret.txt");
    std::fs::write(&secret, b"secret").expect("a file");
    assert!(File::open(&secret).is_ok(), "the user can read it");
    let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
    let port = listener.local_addr().expect("an address").port().to_string();
    let secret = hex(&secret.to_string_lossy());

    let program = std::env::current_exe().expect("the test program");
    let (mut helper, stdin, mut stdout) = start(
        &program,
        &["--exact", TEST, "--nocapture", "--test-threads=1", &secret, &port],
        MEMORY,
    )
    .expect("the test program starts in its container");
    drop(stdin);

    let mut text = String::new();
    stdout.read_to_string(&mut text).expect("the report is read");
    let status = helper.wait().expect("the probe ends");

    assert!(status.success(), "{status:?} {text}");
    let report = text
        .lines()
        .find_map(|line| line.split_once("PROBE "))
        .map_or("", |(_, report)| report);
    assert!(report.contains("container=true"), "it is in an AppContainer: {report}");
    assert!(
        report.contains("opens=false"),
        "no file of the user is opened: {report}"
    );
    assert!(
        report.contains("connects=false"),
        "the loopback cannot be reached: {report}"
    );
    assert!(report.contains("starts=false"), "no other program is started: {report}");
}
