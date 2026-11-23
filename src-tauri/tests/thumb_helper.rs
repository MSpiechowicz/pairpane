//! The real executable as the thumbnail helper: started with `--pairpane-thumbnail` and a side it answers on its standard
//! output without opening a window or starting anything else of the app, confined on Linux before it reads a byte, and
//! the thumbnails the core crate makes with it, from pictures encoded here, a bomb and files that are damaged.

use std::io::Cursor;
use std::path::Path;
use std::process::Child;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use pairpane_core::view::is_refusal;
use pairpane_core::view::thumb::{CHILD_ARG, Source, helper_command, last_failure, make, open, read};

mod common;

use common::finish;

/// The app's own executable, which is the helper.
fn program() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_pairpane"))
}

/// A 16 by 8 picture of one colour, `#3080c0`, made by `heif-enc` (the fixture of the core crate's tests).
const TINY_HEIC: &[u8] = include_bytes!("../../crates/pairpane_core/src/view/heic_testdata/tiny.heic");

/// The colour of the pictures.
const COLOR: [u8; 4] = [0x30, 0x80, 0xC0, 0xFF];

/// `width` by `height` pixels of [`COLOR`] in `format`, as a picture without alpha (which every format here can hold).
fn encode(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
    let [red, green, blue, _] = COLOR;
    let pixels = RgbImage::from_pixel(width, height, Rgb([red, green, blue]));

    let mut out = Cursor::new(Vec::new());
    DynamicImage::from(pixels).write_to(&mut out, format).expect("encoded");

    out.into_inner()
}

/// The CRC-32 of the PNG chunks.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }

    !crc
}

/// A PNG that claims `width` by `height` pixels of 8-bit RGBA and has no pixels: a decompression bomb.
fn bomb(width: u32, height: u32) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunks: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();

    let mut header = width.to_be_bytes().to_vec();
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunks.push((b"IHDR", header));
    chunks.push((b"IDAT", vec![0x78, 0x01]));
    chunks.push((b"IEND", vec![]));

    for (name, data) in chunks {
        let mut chunk = name.to_vec();
        chunk.extend_from_slice(&data);

        png.extend_from_slice(&u32::try_from(data.len()).expect("small").to_be_bytes());
        png.extend_from_slice(&chunk);
        png.extend_from_slice(&crc32(&chunk).to_be_bytes());
    }

    png
}

/// A folder holding `bytes` as `name`, and the file opened to make a thumbnail of it.
fn source(bytes: &[u8]) -> (tempfile::TempDir, Source) {
    let folder = tempfile::tempdir().expect("a folder");
    std::fs::write(folder.path().join("picture"), bytes).expect("a file");
    let source = open(folder.path(), "picture").expect("a picture");

    (folder, source)
}

fn idle() -> AtomicBool {
    AtomicBool::new(false)
}

/// The thumbnail of `bytes` at `size`, through the helper.
fn thumbnail(bytes: &[u8], size: u32) -> std::io::Result<Vec<u8>> {
    let (_folder, source) = source(bytes);

    let bytes = read(&source, &idle())?;
    let made = make(&source, bytes, size, &idle(), program());

    // Where the helper is started in a sandbox that did not start, what failed (the call and the system's error) is told
    // here: it is in the output of the test that fails.
    if let Err(error) = &made {
        eprintln!(
            "no thumbnail: {error:?}; the last failure of the sandbox: {:?}",
            last_failure()
        );
    }

    made
}

fn size_of(answer: &[u8]) -> (u32, u32) {
    (
        u32::from_le_bytes(answer[0..4].try_into().expect("width")),
        u32::from_le_bytes(answer[4..8].try_into().expect("height")),
    )
}

fn is_color(rgba: &[u8], tolerance: u8) -> bool {
    rgba.as_chunks::<4>().0.iter().all(|pixel| {
        let near = |at: usize| pixel[at].abs_diff(COLOR[at]) <= tolerance;

        (0..4).all(near)
    })
}

/// The formats that get a thumbnail, with how far a decoded colour may be from the one that was encoded (a JPEG is not
/// exact).
const FORMATS: [(ImageFormat, u8); 5] = [
    (ImageFormat::Png, 0),
    (ImageFormat::Jpeg, 8),
    (ImageFormat::Gif, 0),
    (ImageFormat::WebP, 0),
    (ImageFormat::Bmp, 0),
];

#[test]
fn every_format_gets_a_thumbnail_from_the_helper() {
    for (format, tolerance) in FORMATS {
        let thumbnail = thumbnail(&encode(format, 300, 150), 100).expect("a thumbnail");

        assert_eq!(size_of(&thumbnail), (100, 50), "{format:?}");
        assert_eq!(thumbnail.len(), 8 + 100 * 50 * 4, "{format:?}");
        assert!(is_color(&thumbnail[8..], tolerance), "{format:?}");
    }
}

#[test]
fn big_pictures_of_every_format_are_decoded_under_the_helper_s_confinement() {
    // Millions of pixels: the memory the decoders ask for grows, moves and is given back, which the allow-list
    // seccomp filter of Linux must let through (and nothing else).
    for (format, tolerance) in FORMATS {
        let thumbnail = thumbnail(&encode(format, 1600, 1200), 256).expect("a thumbnail");

        assert_eq!(size_of(&thumbnail), (256, 192), "{format:?}");
        assert!(is_color(&thumbnail[8..], tolerance), "{format:?}");
    }
}

#[test]
fn the_size_is_clamped_and_a_small_picture_is_not_made_bigger() {
    let png = encode(ImageFormat::Png, 1000, 1000);

    assert_eq!(size_of(&thumbnail(&png, 1).expect("a thumbnail")), (32, 32));
    assert_eq!(size_of(&thumbnail(&png, 10_000).expect("a thumbnail")), (512, 512));
    assert_eq!(
        size_of(&thumbnail(&encode(ImageFormat::Png, 20, 10), 256).expect("a thumbnail")),
        (20, 10)
    );
}

#[test]
fn a_picture_that_claims_a_huge_size_is_too_big_at_once() {
    let started = Instant::now();

    for bytes in [bomb(100_000, 100_000), bomb(30_000, 30_000), bomb(32_768, 1)] {
        let error = thumbnail(&bytes, 64).expect_err("too big");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput, "{error}");
        assert!(is_refusal(&error));
        assert_eq!(error.to_string(), "the image is too big to show here");
    }

    // The helper allocated nothing for them (it would have taken seconds, or died by its limit on memory).
    assert!(started.elapsed() < Duration::from_secs(15));
}

#[test]
fn a_truncated_or_garbled_file_cannot_be_decoded() {
    for format in [
        ImageFormat::Png,
        ImageFormat::Jpeg,
        ImageFormat::Gif,
        ImageFormat::WebP,
        ImageFormat::Bmp,
    ] {
        let whole = encode(format, 64, 64);
        let mut garbled = whole[..24].to_vec();
        garbled.extend((0..4000_u32).map(|at| (at.wrapping_mul(2_654_435_761) >> 24) as u8));

        for bytes in [&whole[..whole.len() / 2], &whole[..32], &garbled[..]] {
            let error = thumbnail(bytes, 64).expect_err("undecodable");

            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData, "{format:?}: {error}");
            assert!(is_refusal(&error));
            assert_eq!(error.to_string(), "the image cannot be decoded");
        }
    }
}

#[test]
fn what_is_not_a_picture_is_refused_before_any_helper_is_started() {
    let folder = tempfile::tempdir().expect("a folder");
    std::fs::write(folder.path().join("a.png"), b"not a png after all").expect("a file");
    std::fs::write(folder.path().join("a.txt"), b"hello").expect("a file");

    for name in ["a.png", "a.txt"] {
        let error = open(folder.path(), name).expect_err(name);

        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
        assert!(is_refusal(&error));
    }
}

/// HEIC is the one picture that has no thumbnail on Windows: its codec there is the Imaging Component, which runs with the
/// user's whole token, and a thumbnail is made without a click.
#[cfg(windows)]
#[test]
fn a_heic_picture_has_no_thumbnail_on_windows() {
    let folder = tempfile::tempdir().expect("a folder");
    std::fs::write(folder.path().join("picture.heic"), TINY_HEIC).expect("a file");

    let error = open(folder.path(), "picture.heic").expect_err("refused at once");

    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
    assert!(is_refusal(&error));
}

#[cfg(not(windows))]
#[test]
fn a_heic_picture_is_decoded_by_the_heic_helper_where_the_system_has_a_decoder() {
    let (_folder, source) = source(TINY_HEIC);
    assert!(source.is_heic());

    match read(&source, &idle()).and_then(|bytes| make(&source, bytes, 64, &idle(), program())) {
        Ok(thumbnail) => {
            // 16 by 8 fits in 64: not made bigger.
            assert_eq!(size_of(&thumbnail), (16, 8));
            assert_eq!(thumbnail.len(), 8 + 16 * 8 * 4);
            assert!(is_color(&thumbnail[8..], 16));
        }
        Err(error) => {
            // CI installs a decoder and sets this, so the decode cannot pass there by being skipped.
            assert!(
                std::env::var_os("PAIRPANE_REQUIRE_HEIC_DECODER").is_none(),
                "a heic decoder is required here: {error}"
            );
            assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
            assert!(is_refusal(&error));
            eprintln!("no heic decoder here, the thumbnail of a heic picture was not checked");
        }
    }
}

/// The helper, started as the app starts it and waiting for its input.
fn start(args: &[&str]) -> Child {
    // Through the app's own `spawn`, as every start in the app is, so that tests that run at once on Windows do not race the
    // pipes of the helper in its container.
    pairpane_core::process::spawn(&mut helper_command(program(), args)).expect("the helper starts")
}

#[test]
fn the_helper_answers_with_a_status_and_the_picture_and_exits_well() {
    let (answer, exited_well) = finish(start(&[CHILD_ARG, "64"]), &encode(ImageFormat::Png, 200, 100));

    assert!(exited_well);
    assert_eq!(answer[0], 0);
    assert_eq!(size_of(&answer[1..]), (64, 32));
    assert_eq!(answer.len(), 1 + 8 + 64 * 32 * 4);

    // 1: not a picture, 2: undecodable, 3: too big; the status stands alone.
    for (input, status) in [
        (b"hello".to_vec(), 1),
        (encode(ImageFormat::Png, 200, 100)[..40].to_vec(), 2),
        (bomb(100_000, 100_000), 3),
    ] {
        let (answer, exited_well) = finish(start(&[CHILD_ARG, "64"]), &input);

        assert!(exited_well);
        assert_eq!(answer, [status]);
    }
}

#[test]
fn a_helper_without_a_usable_side_answers_nothing_and_fails() {
    for args in [
        vec![CHILD_ARG],
        vec![CHILD_ARG, "wide"],
        vec![CHILD_ARG, "-5"],
        vec![CHILD_ARG, ""],
    ] {
        let (answer, exited_well) = finish(start(&args), &encode(ImageFormat::Png, 20, 10));

        assert!(answer.is_empty(), "{args:?}");
        assert!(!exited_well, "{args:?}");
    }
}

#[test]
fn the_helper_never_writes_more_than_the_side_allows() {
    // A side of 32 is asked of a 4000 by 3000 picture: 32 by 24, whatever the picture.
    let (answer, _) = finish(start(&[CHILD_ARG, "32"]), &encode(ImageFormat::Png, 4000, 3000));

    assert_eq!(size_of(&answer[1..]), (32, 24));
    assert_eq!(answer.len(), 1 + 8 + 32 * 24 * 4);
}

/// The helper is confined before it reads a byte (it has no codec to load first), under the smaller address space of the
/// thumbnails, and decodes under that.
#[cfg(target_os = "linux")]
#[test]
fn the_helper_confines_itself_before_it_reads_and_decodes_under_the_limits() {
    let child = start(&[CHILD_ARG, "64"]);
    let proc = format!("/proc/{}", child.id());
    let text = common::wait_until_confined(&proc);

    common::assert_limited(&proc, &text);

    // The address space is the mapped size when it started and 1.5 GiB more: the limit is far under the 3 GiB of the HEIC
    // helper's room, and over what the picture needs.
    let space: u64 = text
        .lines()
        .find(|line| line.starts_with("Max address space"))
        .and_then(|line| line.split_whitespace().nth(3))
        .and_then(|value| value.parse().ok())
        .expect("a limit on the address space");
    assert!(space < 3 * 1024 * 1024 * 1024, "{space}");
    assert!(space >= 1536 * 1024 * 1024, "{space}");

    let (answer, exited_well) = finish(child, &encode(ImageFormat::Png, 200, 100));
    assert!(exited_well);
    assert_eq!(size_of(&answer[1..]), (64, 32));
}

/// Nothing of the app reaches the helper: it reads its input and writes its answer through its standard streams only,
/// and the file system is closed to it, so a picture cannot make it open another file (the probe is `/proc/self`, which
/// every process may read until it gives up the file system).
#[cfg(target_os = "linux")]
#[test]
fn the_helper_has_no_descriptor_beyond_the_standard_streams_when_it_starts_to_read() {
    let child = start(&[CHILD_ARG, "64"]);
    let proc = format!("/proc/{}", child.id());
    common::wait_until_confined(&proc);

    // After Landlock the helper cannot list its descriptors, but we can: stdin, stdout and stderr (the null device).
    let open = common::open_descriptors(&proc);

    let (_, exited_well) = finish(child, &encode(ImageFormat::Png, 8, 8));
    assert!(exited_well);
    assert_eq!(open, [0, 1, 2], "{open:?}");
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code, reason = "`fcntl`, to leak a descriptor to the helper; see the test")]
unsafe extern "C" {
    fn fcntl(fd: i32, command: i32, ...) -> i32;
}

/// The descriptors of the process `pid`, by `lsof` (a tool of the system, started with fixed arguments and no shell). The
/// answer is checked to be one: `lsof` that tells nothing would make every test of what is closed pass.
#[cfg(target_os = "macos")]
fn descriptors_of(pid: u32) -> Vec<u32> {
    let output = std::process::Command::new("/usr/sbin/lsof")
        .args(["-a", "-p", &pid.to_string(), "-F", "pf"])
        .output()
        .expect("lsof runs");
    let text = String::from_utf8_lossy(&output.stdout);

    assert!(
        text.lines().any(|line| line == format!("p{pid}")),
        "lsof tells about the process {pid}: {text} {}",
        String::from_utf8_lossy(&output.stderr)
    );

    text.lines()
        .filter_map(|line| line.strip_prefix('f')?.parse().ok())
        .collect()
}

/// The helper closes the descriptors it inherits and keeps what is its own: a descriptor of this test program that is
/// not close-on-exec (as one that a library of the app may leak) is in a process this program starts, which a program that
/// is not sandboxed shows, and is gone from the helper once it has confined itself, which is before it reads the picture;
/// the picture is still decoded in the sandbox.
#[cfg(target_os = "macos")]
#[allow(unsafe_code, reason = "`fcntl` makes a descriptor inheritable; see the test")]
#[test]
fn the_helper_closes_the_descriptors_it_inherits_and_still_decodes_in_its_sandbox() {
    use std::os::fd::IntoRawFd;

    // Opened close-on-exec (as the standard library opens everything), then made inheritable and given up by its owner:
    // the descriptor is open in this program and in every process it starts, and nothing owns it.
    let leaked = std::fs::File::open("/dev/null").expect("a file").into_raw_fd();
    // SAFETY: `fcntl` with `F_SETFD` takes the descriptor and the flags by value; `leaked` is open and owned by nothing.
    assert_eq!(unsafe { fcntl(leaked, 2, 0) }, 0, "the descriptor is made inheritable");
    let leaked = u32::try_from(leaked).expect("a descriptor");

    // The control: a program that is not sandboxed, started the same way, holds it. Without this a helper that does not
    // show it would prove nothing.
    let mut control = pairpane_core::process::spawn(std::process::Command::new("/bin/sleep").arg("30"))
        .expect("a program of the system starts");
    let held = descriptors_of(control.id());
    let _ = control.kill();
    let _ = control.wait();
    assert!(
        held.contains(&leaked),
        "the descriptor reaches a process that is started: {held:?}"
    );

    let child = start(&[CHILD_ARG, "64"]);
    let confined = |open: &[u32]| [0, 1, 2].iter().all(|fd| open.contains(fd)) && !open.contains(&leaked);
    let mut open_in_helper = descriptors_of(child.id());
    let patience = Instant::now();
    while !confined(&open_in_helper) && patience.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(100));
        open_in_helper = descriptors_of(child.id());
    }

    let (answer, exited_well) = finish(child, &encode(ImageFormat::Png, 200, 100));

    assert!(
        confined(&open_in_helper),
        "the helper has its standard streams and not the descriptor that was leaked to it: {open_in_helper:?}"
    );
    assert!(exited_well);
    assert_eq!(size_of(&answer[1..]), (64, 32));
}
