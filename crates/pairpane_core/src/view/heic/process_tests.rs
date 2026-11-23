//! The helper process of the HEIC decoding: the answer it writes and the parent's reading of it (on every system), the
//! brand check, and the parent's handling of a helper that crashes, hangs, lies or runs away (with the tools of a Unix
//! system standing in for the helper).

use std::io::{self, Cursor, Read};

use pretty_assertions::assert_eq;

use super::process::{read_answer, run_child, write_answer};
use super::{HEADER, MAX_IMAGE_BYTES, is_hevc_heif};
use crate::view::is_refusal;
use crate::view::testing::ftyp;

/// A 16 by 8 picture of one colour, `#3080c0`, made by `heif-enc` from a PNG.
const TINY: &[u8] = include_bytes!("../heic_testdata/tiny.heic");

/// What the helper writes for `input`, and its exit code.
fn helper_answer(input: impl io::Read) -> (Vec<u8>, i32) {
    let mut output = Vec::new();
    let code = run_child(input, &mut output);

    (output, code)
}

/// An answer that holds a picture of `width` by `height` with `pixels` bytes after its header.
fn answer(width: u32, height: u32, pixels: usize) -> Vec<u8> {
    let mut bytes = vec![0];
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.resize(1 + HEADER + pixels, 7);

    bytes
}

#[test]
fn a_picture_is_answered_with_its_status_and_a_failure_with_a_status_alone() {
    let picture = answer(2, 3, 2 * 3 * 4)[1..].to_vec();

    let mut written = Vec::new();
    write_answer(&mut written, &Ok(picture.clone())).expect("written");
    assert_eq!(written[0], 0);
    assert_eq!(read_answer(&written[..]).expect("a picture"), picture);

    for (error, status, kind) in [
        (super::no_decoder(), 1, io::ErrorKind::Unsupported),
        (super::undecodable(), 2, io::ErrorKind::InvalidData),
        (super::too_big(), 3, io::ErrorKind::InvalidInput),
        (io::Error::other("anything else"), 2, io::ErrorKind::InvalidData),
    ] {
        let mut written = Vec::new();
        write_answer(&mut written, &Err(error)).expect("written");
        assert_eq!(written, [status]);

        let read = read_answer(&written[..]).expect_err("a failure");
        assert_eq!(read.kind(), kind);
        assert!(is_refusal(&read), "the text is ours");
    }
}

#[test]
fn an_answer_that_does_not_say_what_it_is_is_undecodable() {
    let bad = [
        // Nothing, an unknown status, a failure with something after it.
        vec![],
        vec![9],
        vec![1, 0],
        vec![2, 2],
        vec![3, 0, 0],
        // A picture without its header, with a short one, and with the size of nothing.
        vec![0],
        answer(1, 1, 4)[..8].to_vec(),
        answer(0, 5, 0),
        answer(5, 0, 0),
        // A picture a byte short or long, and one over the caps whatever the bytes after it are.
        answer(2, 2, 15),
        answer(2, 2, 17),
        answer(40_000, 1, 40_000 * 4),
        answer(32_767, 32_767, 16),
    ];

    for bytes in bad {
        let error = read_answer(&bytes[..]).expect_err("not an answer");
        assert_eq!(
            error.kind(),
            io::ErrorKind::InvalidData,
            "{:?}",
            &bytes[..bytes.len().min(12)]
        );
        assert!(is_refusal(&error));
    }

    // Exactly as long as its header says is taken, without the status.
    let good = read_answer(&answer(2, 2, 16)[..]).expect("a picture");
    assert_eq!(good.len(), HEADER + 16);
    assert_eq!(good[..4], 2_u32.to_le_bytes());
}

#[test]
fn the_helper_decodes_the_fixture_where_the_system_has_a_decoder() {
    let (output, code) = helper_answer(Cursor::new(TINY));

    assert_eq!(code, 0);
    let picture = match read_answer(&output[..]) {
        Ok(picture) => picture,
        Err(error) if error.kind() == io::ErrorKind::Unsupported => {
            // CI installs a decoder and sets this, so the decode cannot pass there by being skipped.
            assert!(
                std::env::var_os("PAIRPANE_REQUIRE_HEIC_DECODER").is_none(),
                "a heic decoder is required here: {error}"
            );
            eprintln!("no heic decoder here, skipped: {error}");

            return;
        }
        Err(error) => {
            #[cfg(windows)]
            let step = super::wic::failed_at();
            #[cfg(not(windows))]
            let step: Option<(&str, i32)> = None;

            panic!(
                "the fixture did not decode: {error} (failed at {:?})",
                step.map(|(step, code)| format!("{step} {:#010x}", code as u32))
            )
        }
    };

    assert_eq!(picture.len(), HEADER + 16 * 8 * 4);
    assert_eq!(picture[..4], 16_u32.to_le_bytes());
    assert_eq!(picture[4..8], 8_u32.to_le_bytes());
    for pixel in picture[HEADER..].as_chunks::<4>().0 {
        for (got, want) in pixel.iter().zip([0x30_u8, 0x80, 0xC0, 0xFF]) {
            assert!(got.abs_diff(want) <= 16, "{pixel:?} is not #3080c0 and opaque");
        }
    }
}

#[test]
fn garbage_and_a_file_over_the_cap_are_answered_with_a_status_and_never_a_panic() {
    let mut garbage = TINY[..32].to_vec();
    garbage.extend((0..4000_u32).map(|at| (at.wrapping_mul(2_654_435_761) >> 24) as u8));

    // Not a picture (or no decoder at all): undecodable or unsupported.
    for input in [&garbage[..], &TINY[..TINY.len() / 2], &TINY[..8], &[][..]] {
        let (output, code) = helper_answer(Cursor::new(input));

        assert_eq!(code, 0);
        let error = read_answer(&output[..]).expect_err("not a picture");
        assert!(matches!(
            error.kind(),
            io::ErrorKind::InvalidData | io::ErrorKind::Unsupported
        ));
    }

    // One byte more than the cap of a file is read and refused, whatever is after it.
    let over = io::repeat(0).take(MAX_IMAGE_BYTES + 1000);
    let (output, code) = helper_answer(over);
    assert_eq!(code, 0);
    assert_eq!(output, [3]);
}

#[test]
fn only_hevc_coded_heif_is_sent_to_the_decoder() {
    // The fixture: major brand heic, compatible mif1, heic, miaf.
    assert!(is_hevc_heif(TINY));

    for major in [b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx"] {
        assert!(is_hevc_heif(&ftyp(major, &[])), "{major:?}");
    }
    // An HEVC brand among the compatible ones is as good as the major one, and an AVIF compatible one is not.
    assert!(is_hevc_heif(&ftyp(b"mif1", &[b"miaf", b"heic"])));
    assert!(!is_hevc_heif(&ftyp(b"mif1", &[b"avif"])));
    assert!(!is_hevc_heif(&ftyp(b"avif", &[b"mif1", b"miaf"])));
    assert!(!is_hevc_heif(&ftyp(b"mif1", &[])));
    assert!(!is_hevc_heif(&ftyp(b"msf1", &[b"avis"])));
    assert!(!is_hevc_heif(&ftyp(b"isom", &[b"mp41"])));
}

#[test]
fn a_box_that_is_not_a_whole_ftyp_is_refused_and_never_a_panic() {
    let whole = ftyp(b"mif1", &[b"heic"]);
    let size = 20;

    // Truncated inside the brands, inside the head, and nothing.
    for cut in [0, 4, 8, 11, 15, 16, 19] {
        assert!(!is_hevc_heif(&whole[..cut]), "cut at {cut}");
    }
    assert!(is_hevc_heif(&whole[..size]), "exactly the box");

    // A size under the head, to the end of the file (0), a 64-bit size (1) and one past the file.
    for declared in [0_u32, 1, 8, 15, 21, u32::MAX] {
        let mut bytes = whole[..size].to_vec();
        bytes[..4].copy_from_slice(&declared.to_be_bytes());
        assert!(!is_hevc_heif(&bytes), "size {declared}");
    }

    // Another box first, and a head that only looks like one.
    let mut other = whole.clone();
    other[4..8].copy_from_slice(b"moov");
    assert!(!is_hevc_heif(&other));
    assert!(!is_hevc_heif(b"\0\0\0\x10ftypheic"));

    // A huge box is read only as far as its first kilobyte: an HEVC brand after that is not seen.
    let mut huge = ftyp(b"mif1", &[]);
    huge[..4].copy_from_slice(&5000_u32.to_be_bytes());
    huge.resize(5000, 0);
    huge[2000..2004].copy_from_slice(b"heic");
    assert!(!is_hevc_heif(&huge));
}

/// The helper is a program of the system, not the app: these show what the parent does with what it gets. Only fixed
/// programs with explicit arguments are used, never a shell.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod isolation {
    use std::io;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use pretty_assertions::assert_eq;

    use super::{HEADER, TINY, is_refusal};
    use crate::view::heic::decode_isolated;
    use crate::view::heic::process::run;
    use crate::view::testing::{file_of, idle};

    /// The answer `[0, 1, 0, 0, 0, 1, 0, 0, 0, a, b, c, d]` as the argument of `printf`: a picture of one pixel.
    const ONE_PIXEL: &str = "\\000\\001\\000\\000\\000\\001\\000\\000\\000abcd";

    /// The result of the fixture with `program` and `args` as the helper.
    fn through(program: &str, args: &[&str]) -> io::Result<Vec<u8>> {
        let (_folder, file) = file_of(TINY);

        decode_isolated(&file, &idle(), Path::new(program), args)
    }

    #[test]
    fn a_helper_that_fails_or_says_nothing_is_a_file_that_cannot_be_decoded() {
        // Exits with an error, and exits well with no answer at all.
        for program in ["false", "true"] {
            let error = through(program, &[]).expect_err(program);

            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{program}");
            assert_eq!(error.to_string(), "the image cannot be decoded");
            assert!(is_refusal(&error));
        }
    }

    #[test]
    fn the_answer_of_the_helper_is_what_the_parent_reports() {
        let error = through("printf", &["\\001"]).expect_err("no decoder");
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);

        let error = through("printf", &["\\003"]).expect_err("too big");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);

        let error = through("printf", &["\\002"]).expect_err("undecodable");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        // A picture of one pixel (status, 1, 1, four bytes), and ones with a pixel byte short or too many.
        let picture = through("printf", &[ONE_PIXEL]).expect("a picture");
        assert_eq!(picture, [1, 0, 0, 0, 1, 0, 0, 0, b'a', b'b', b'c', b'd']);
        assert_eq!(picture.len(), HEADER + 4);
        for answer in [&ONE_PIXEL[..ONE_PIXEL.len() - 1], &format!("{ONE_PIXEL}e")] {
            let error = through("printf", &[answer]).expect_err(answer);
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{answer}");
        }
    }

    #[test]
    fn a_helper_that_answers_without_end_is_stopped_at_the_cap() {
        let started = Instant::now();

        let error = through("yes", &[]).expect_err("a runaway answer");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(started.elapsed() < Duration::from_secs(20), "stopped, not waited for");
    }

    #[test]
    fn a_helper_that_is_not_there_is_a_failure_without_text_of_ours() {
        let (_folder, file) = file_of(TINY);

        let error =
            decode_isolated(&file, &idle(), Path::new("/nonexistent/pairpane-helper"), &[]).expect_err("no helper");

        assert!(!is_refusal(&error));
        assert_ne!(error.kind(), io::ErrorKind::NotFound, "not mistaken for a missing file");
    }

    #[test]
    fn raising_the_cancel_flag_stops_a_helper_that_runs_on() {
        let (_folder, file) = file_of(TINY);
        let cancel = AtomicBool::new(false);
        let started = Instant::now();

        let error = std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(150));
                cancel.store(true, Ordering::Relaxed);
            });

            decode_isolated(&file, &cancel, Path::new("sleep"), &["30"]).expect_err("cancelled")
        });

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the helper did not hold it up"
        );
    }

    #[test]
    fn a_helper_that_runs_past_the_deadline_is_killed_and_is_a_file_that_cannot_be_decoded() {
        let started = Instant::now();

        let error = run(
            Path::new("sleep"),
            &["30"],
            TINY.to_vec(),
            &idle(),
            Duration::from_millis(300),
        )
        .expect_err("a hung helper");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(is_refusal(&error));
        assert!(started.elapsed() < Duration::from_secs(10), "killed, not waited for");
    }

    #[test]
    fn a_flag_raised_before_the_start_starts_nothing() {
        let (_folder, file) = file_of(TINY);
        let cancel = AtomicBool::new(true);

        let error =
            decode_isolated(&file, &cancel, Path::new("/nonexistent/pairpane-helper"), &[]).expect_err("cancelled");

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }
}
