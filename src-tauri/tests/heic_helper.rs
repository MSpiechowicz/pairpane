//! The real executable as the HEIC decoding helper: started with `--pairpane-decode-heic` it answers on its standard
//! output without opening a window or starting anything else of the app.

#![allow(
    unsafe_code,
    reason = "`set_var` and `fcntl` in the test of what the helper inherits; see there"
)]

#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::Child;

use pairpane_core::view::{HEIC_CHILD_ARG, heic_helper_command};

mod common;

use common::finish;

/// A 16 by 8 picture of one colour, `#3080c0`, made by `heif-enc` (the fixture of the core crate's tests).
const TINY: &[u8] = include_bytes!("../../crates/pairpane_core/src/view/heic_testdata/tiny.heic");

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn fcntl(fd: i32, command: i32, ...) -> i32;
}

/// `F_SETFD` of `fcntl`: sets the descriptor flags (0 clears close-on-exec).
#[cfg(target_os = "linux")]
const F_SETFD: i32 = 2;

/// The helper, started as the app starts it (`heic_helper_command`) and waiting for its input.
fn start() -> Child {
    heic_helper_command(Path::new(env!("CARGO_BIN_EXE_pairpane")), &[HEIC_CHILD_ARG])
        .spawn()
        .expect("the helper starts")
}

/// The answer of a helper to `input`.
fn ask(input: &[u8]) -> (Vec<u8>, bool) {
    finish(start(), input)
}

/// Asserts that `answer` is the picture of the fixture. `false` when the system has no decoder (the answer is then
/// only the status of that).
fn is_the_picture(answer: &[u8]) -> bool {
    if answer == [1] {
        // CI installs a decoder and sets this, so the decode cannot pass there by being skipped.
        assert!(
            std::env::var_os("PAIRPANE_REQUIRE_HEIC_DECODER").is_none(),
            "a heic decoder is required here"
        );
        eprintln!("no heic decoder here, the helper's picture was not checked");

        return false;
    }

    // Status 0, the header and nothing else: 16 by 8 pixels of RGBA.
    assert_eq!(answer.len(), 1 + 8 + 16 * 8 * 4);
    assert_eq!(answer[0], 0);
    assert_eq!(answer[1..5], 16_u32.to_le_bytes());
    assert_eq!(answer[5..9], 8_u32.to_le_bytes());
    let near_blue = |pixel: &[u8; 4]| {
        pixel
            .iter()
            .zip([0x30_u8, 0x80, 0xC0, 0xFF])
            .all(|(got, want)| got.abs_diff(want) <= 16)
    };
    assert!(
        answer[9..].as_chunks::<4>().0.iter().all(near_blue),
        "the helper's picture is not #3080c0 and opaque"
    );

    true
}

#[test]
fn the_helper_decodes_the_fixture_where_the_system_has_a_decoder() {
    let (answer, exited_well) = ask(TINY);

    assert!(exited_well, "the helper exits with 0 after any answer");
    is_the_picture(&answer);
}

/// The helper is confined before it reads a byte (see `sandbox.rs` of the core crate), and the picture is still
/// decoded under that: the codecs are loaded before the file system is given up, and need no file after.
#[cfg(target_os = "linux")]
#[test]
fn the_helper_confines_itself_before_it_reads_and_decodes_under_the_limits() {
    let child = start();
    let proc = format!("/proc/{}", child.id());
    let text = common::wait_until_confined(&proc);

    common::assert_limited(&proc, &text);

    let (answer, exited_well) = finish(child, TINY);
    assert!(exited_well);
    if is_the_picture(&answer) {
        eprintln!("decoded under the helper's limits and Landlock");
    }
}

#[test]
fn the_helper_answers_a_file_it_cannot_decode_with_a_status_and_nothing_else() {
    let mut garbage = TINY[..32].to_vec();
    garbage.extend((0..4000_u32).map(|at| (at.wrapping_mul(2_654_435_761) >> 24) as u8));

    for input in [&garbage[..], &TINY[..TINY.len() / 2], &[]] {
        let (answer, exited_well) = ask(input);

        assert!(exited_well);
        // No decoder here (1) or not a picture (2), and the status stands alone.
        assert!(answer == [1] || answer == [2], "{answer:?}");
    }
}

/// The helper gets no descriptor of the app beyond the standard streams (the app has one open that is not close-on-exec)
/// and none of the app's environment (a sentinel in it), and still decodes the picture.
#[cfg(target_os = "linux")]
#[test]
fn the_helper_inherits_no_descriptor_and_no_environment() {
    const SENTINEL: &str = "PAIRPANE_TEST_SECRET_SENTINEL";

    // SAFETY: the name is one no other test or library of this program reads, and `Command` (which reads the
    // environment when it starts a helper) takes the standard library's lock that `set_var` takes too.
    unsafe { std::env::set_var(SENTINEL, "do-not-leak") };

    // A descriptor the standard library would have marked close-on-exec, made inheritable: the helper holds it unless it
    // closes it (it is closed again when `leaked` goes out of scope).
    let leaked = std::fs::File::open("/dev/null").expect("a file");
    // SAFETY: `fcntl` with `F_SETFD` takes the descriptor and the flags by value; `leaked` is open and owned here.
    let set = unsafe { fcntl(leaked.as_raw_fd(), F_SETFD, 0) };
    assert_eq!(set, 0, "the descriptor is made inheritable");

    let child = start();
    let proc = format!("/proc/{}", child.id());
    common::wait_until_confined(&proc);

    let open = common::open_descriptors(&proc);
    assert_eq!(open, [0, 1, 2], "the helper holds only its standard streams");

    let environment = std::fs::read(format!("{proc}/environ")).expect("the environment");
    assert!(
        !String::from_utf8_lossy(&environment).contains(SENTINEL),
        "the helper has none of the app's environment"
    );

    let (answer, exited_well) = finish(child, TINY);
    assert!(exited_well);
    is_the_picture(&answer);
}
