//! HEIC decoding: the safe helpers on every system, and the decoder of this system where it has one.

use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use pretty_assertions::assert_eq;

use crate::model::ViewKind;

use super::heic::{HEADER, check_size, copy_rows, map_heif, map_hresult, no_decoder, output, read_whole};
use super::testing::{file_of, idle, opened, temp};
use super::{MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE, decode_heic, open};

/// A 16 by 8 picture of one colour, `#3080c0`, made by `heif-enc` from a PNG.
const TINY: &[u8] = include_bytes!("heic_testdata/tiny.heic");

const NO_DECODER_TEXT: &str = if cfg!(windows) {
    "no heic decoder on this system: install the heif and hevc image extensions"
} else if cfg!(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
)) {
    "no heic decoder on this system: install libheif with an hevc plugin"
} else {
    "no heic decoder on this system"
};

/// An AV1 image in a file of the brands `mif1` and `heic`, made by `heif-enc -A`: the system's decoder takes it too.
const AV1: &[u8] = include_bytes!("heic_testdata/av1_mif1_heic.heic");

#[cfg(any(
    windows,
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
))]
#[test]
fn a_file_with_an_item_of_another_codec_is_not_a_heic_image_and_reaches_no_helper() {
    let (_folder, file) = file_of(AV1);

    // A helper that is not there: the refusal comes first, so it is never looked for.
    let error = decode_heic(&file, &idle(), Path::new("/nonexistent/pairpane-helper")).expect_err("refused");

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "the file is not a heic image");
    assert!(super::is_refusal(&error));
}

#[test]
fn the_fixture_is_sniffed_as_a_heic_image() {
    let (_folder, opened) = opened("tiny.heic", TINY);

    assert_eq!(
        opened.kind,
        ViewKind::Image {
            mime: "image/heic".to_owned()
        }
    );
}

#[test]
fn sizes_are_checked_without_overflow() {
    let side = i64::from(MAX_IMAGE_SIDE);

    assert_eq!(check_size(1, 1).expect("one pixel"), (1, 1));
    assert_eq!(check_size(16, 8).expect("a small picture"), (16, 8));
    for (width, height) in [(0, 5), (5, 0), (-1, 5), (5, -1), (i64::MIN, i64::MIN), (0, 0)] {
        let error = check_size(width, height).expect_err("not a size");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{width}x{height}");
    }
    for (width, height) in [(side + 1, 1), (1, side + 1), (i64::MAX, i64::MAX), (i64::MAX, 1)] {
        let error = check_size(width, height).expect_err("too wide or tall");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{width}x{height}");
    }

    // Under the side cap but over the pixel cap.
    let over = i64::try_from(MAX_IMAGE_PIXELS).expect("fits") / side + 1;
    let error = check_size(side, over).expect_err("too many pixels");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);

    // The largest picture that is allowed: the pixel cap, in the longest allowed side.
    let high = i64::try_from(MAX_IMAGE_PIXELS).expect("fits") / side;
    assert_eq!(
        check_size(side, high).expect("the largest"),
        (MAX_IMAGE_SIDE, u32::try_from(high).expect("fits"))
    );
    assert_eq!(check_size(8192, 8192).expect("exactly the cap"), (8192, 8192));
}

#[test]
fn rows_are_copied_without_their_padding() {
    let tight: Vec<u8> = (0..12).collect();
    let mut out = vec![0; 12];
    copy_rows(&mut out, &tight, 4, 3, 4).expect("equal stride");
    assert_eq!(out, tight);

    // Rows of 4 bytes, 6 apart; the last row has no padding after it.
    let padded = [1, 2, 3, 4, 9, 9, 5, 6, 7, 8, 9, 9, 10, 11, 12, 13];
    let mut out = vec![0; 12];
    copy_rows(&mut out, &padded, 4, 3, 6).expect("padded stride");
    assert_eq!(out, [1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13]);

    let mut out = vec![0; 4];
    copy_rows(&mut out, &[1, 2, 3, 4], 4, 1, 100).expect("one row needs no stride");
    assert_eq!(out, [1, 2, 3, 4]);
}

#[test]
fn rows_that_do_not_fit_are_refused_and_never_panic() {
    let src = [0_u8; 16];

    // A stride under the row, a source too short (by one byte), an output of another length.
    assert!(copy_rows(&mut [0; 12], &src, 4, 3, 3).is_err());
    assert!(copy_rows(&mut [0; 12], &src[..11], 4, 3, 4).is_err());
    assert!(copy_rows(&mut [0; 12], &src[..13], 4, 3, 6).is_err());
    assert!(copy_rows(&mut [0; 8], &src, 4, 3, 4).is_err());
    assert!(copy_rows(&mut [0; 16], &src, 4, 3, 4).is_err());

    // Sizes that overflow.
    assert!(copy_rows(&mut [0; 4], &src, 4, usize::MAX, usize::MAX).is_err());
    assert!(copy_rows(&mut [0; 4], &src, usize::MAX, 2, usize::MAX).is_err());
    assert!(copy_rows(&mut [0; 4], &src, 4, 2, usize::MAX).is_err());

    // Nothing to copy.
    assert!(copy_rows(&mut [], &src, 4, 0, 4).is_ok());
    assert!(copy_rows(&mut [0; 4], &src, 4, 0, 4).is_err());
}

#[test]
fn the_output_starts_with_the_size_and_holds_four_bytes_per_pixel() {
    let out = output(3, 2).expect("a buffer");

    assert_eq!(out.len(), HEADER + 3 * 2 * 4);
    assert_eq!(out[..4], 3_u32.to_le_bytes());
    assert_eq!(out[4..8], 2_u32.to_le_bytes());
    assert!(out[HEADER..].iter().all(|byte| *byte == 0));

    let out = output(0x0102, 0x0304).expect("a buffer with a wide header");
    assert_eq!(out[..4], [0x02, 0x01, 0, 0]);
    assert_eq!(out[4..8], [0x04, 0x03, 0, 0]);
}

#[test]
fn a_file_is_read_whole_in_pieces() {
    // More than one piece of MAX_READ.
    let bytes: Vec<u8> = (0..(super::MAX_READ + 1000)).map(|at| (at % 251) as u8).collect();
    let (_folder, file) = file_of(&bytes);

    assert_eq!(read_whole(&file, &idle()).expect("read"), bytes);
}

#[test]
fn a_file_over_the_cap_is_refused_before_it_is_read() {
    let folder = temp();
    let path = folder.path().join("big.heic");
    File::create(&path)
        .expect("create")
        .set_len(MAX_IMAGE_BYTES + 1)
        .expect("a sparse file");
    let opened = open(folder.path(), "big.heic").expect("open");

    let error = read_whole(&opened.file, &idle()).expect_err("too big");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "the image is too big to show here");

    // Also through the public function, whatever the system has (the helper is not reached either way).
    let error = decode_heic(&opened.file, &idle(), Path::new("/nonexistent/pairpane-helper")).expect_err("too big");
    assert!(matches!(
        error.kind(),
        io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
    ));
}

#[test]
fn a_cancelled_read_is_interrupted() {
    let (_folder, file) = file_of(TINY);
    let cancel = AtomicBool::new(false);
    cancel.store(true, Ordering::Relaxed);

    let error = read_whole(&file, &cancel).expect_err("cancelled");

    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

#[test]
fn libheif_results_map_to_the_two_kinds_of_failure() {
    // Unsupported feature with an unsupported codec, plugin loading error and "no matching decoder installed" mean
    // there is no HEVC decoder; the rest (a decoder plugin error too: the decoder failed on the data) is a bad file.
    for (code, subcode) in [(4, 3000), (11, 0), (11, 6001), (3, 6003), (4, 6003)] {
        let error = map_heif(code, subcode);
        assert_eq!(error.kind(), io::ErrorKind::Unsupported, "{code}/{subcode}");
        assert_eq!(error.to_string(), NO_DECODER_TEXT);
    }
    for (code, subcode) in [
        (1, 0),
        (2, 100),
        (3, 0),
        (4, 3001),
        (4, 0),
        (5, 0),
        (6, 0),
        (7, 0),
        (7, 9999),
        (8, 0),
        (-1, -1),
    ] {
        let error = map_heif(code, subcode);
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{code}/{subcode}");
        assert_eq!(error.to_string(), "the image cannot be decoded");
    }
}

#[test]
fn imaging_component_results_map_to_the_two_kinds_of_failure() {
    // WINCODEC_ERR_COMPONENTNOTFOUND and MF_E_TOPO_CODEC_NOT_FOUND.
    for hresult in [0x8898_2F50_u32 as i32, 0xC00D_5212_u32 as i32] {
        let error = map_hresult(hresult);
        assert_eq!(error.kind(), io::ErrorKind::Unsupported, "{hresult:#x}");
        assert_eq!(error.to_string(), NO_DECODER_TEXT);
    }
    // WINCODEC_ERR_BADIMAGE, E_FAIL, E_OUTOFMEMORY, S_OK.
    for hresult in [
        0x8898_2F60_u32 as i32,
        0x8000_4005_u32 as i32,
        0x8007_000E_u32 as i32,
        0,
    ] {
        let error = map_hresult(hresult);
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{hresult:#x}");
        assert_eq!(error.to_string(), "the image cannot be decoded");
    }
}

#[test]
fn the_error_texts_are_fixed() {
    assert_eq!(no_decoder().to_string(), NO_DECODER_TEXT);
    assert_eq!(no_decoder().kind(), io::ErrorKind::Unsupported);
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
#[test]
fn a_missing_libheif_means_no_decoder() {
    use super::heic::libheif::Api;

    assert!(Api::load("libpairpane-missing.so.0").is_none());
}

#[cfg(not(any(
    windows,
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
)))]
#[test]
fn a_system_without_a_decoder_says_so_at_once_and_starts_nothing() {
    let (_folder, file) = file_of(TINY);

    // The helper is not there; it is not looked for.
    let error = decode_heic(&file, &idle(), Path::new("/nonexistent/pairpane-helper")).expect_err("unsupported");

    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert_eq!(error.to_string(), NO_DECODER_TEXT);
}
