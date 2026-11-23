//! The item types a HEIF file may list before it reaches a codec, and the walk's way with boxes that are not what they
//! say: a refusal, never a panic.

use super::items::only_hevc_items;

/// A 16 by 8 picture of one colour, `#3080c0`, made by `heif-enc` from a PNG (one `hvc1` item).
const TINY: &[u8] = include_bytes!("../heic_testdata/tiny.heic");

/// The same container with an AV1 image (`av01`) in a file of the brands `mif1` and `heic`, made by `heif-enc -A`.
const AV1: &[u8] = include_bytes!("../heic_testdata/av1_mif1_heic.heic");

/// A box of `kind` with `content`.
pub(in crate::view) fn boxed(kind: &[u8; 4], content: &[u8]) -> Vec<u8> {
    let mut bytes = u32::try_from(8 + content.len()).expect("small").to_be_bytes().to_vec();
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(content);

    bytes
}

/// An `infe` of this version for an item of type `kind` (ID 1, not protected).
fn infe(version: u8, kind: &[u8; 4]) -> Vec<u8> {
    let mut content = vec![version, 0, 0, 0];
    match version {
        3 => content.extend_from_slice(&[0, 0, 0, 1]),
        _ => content.extend_from_slice(&[0, 1]),
    }
    content.extend_from_slice(&[0, 0]);
    content.extend_from_slice(kind);
    content.push(0);

    boxed(b"infe", &content)
}

/// An `iinf` (version 0, the count of its entries as the entries say) of `entries`.
fn iinf(entries: &[Vec<u8>]) -> Vec<u8> {
    let mut content = vec![0, 0, 0, 0];
    content.extend_from_slice(&u16::try_from(entries.len()).unwrap_or(u16::MAX).to_be_bytes());
    for entry in entries {
        content.extend_from_slice(entry);
    }

    boxed(b"iinf", &content)
}

/// A file of an `ftyp` and a `meta` that holds `children`, in the way of a HEIC.
fn file_with(children: &[Vec<u8>]) -> Vec<u8> {
    let mut meta = vec![0, 0, 0, 0];
    for child in children {
        meta.extend_from_slice(child);
    }

    let mut bytes = boxed(b"ftyp", b"heic\0\0\0\0mif1heic");
    bytes.extend_from_slice(&boxed(b"meta", &meta));

    bytes
}

/// A file whose items are of the types `kinds` (as `infe` version 2).
fn file_of_items(kinds: &[&[u8; 4]]) -> Vec<u8> {
    let entries: Vec<_> = kinds.iter().map(|kind| infe(2, kind)).collect();

    file_with(&[iinf(&entries)])
}

#[test]
fn the_fixture_with_one_hevc_item_is_taken_and_the_av1_one_is_refused() {
    assert!(only_hevc_items(TINY));
    assert!(!only_hevc_items(AV1), "an AV1 image in a file of the brand heic");
}

#[test]
fn only_the_listed_item_types_are_taken() {
    for kind in [
        b"hvc1", b"hvt1", b"grid", b"iden", b"iovl", b"tmap", b"Exif", b"mime", b"uri ",
    ] {
        assert!(only_hevc_items(&file_of_items(&[kind])), "{kind:?}");
    }
    assert!(only_hevc_items(&file_of_items(&[b"hvc1", b"hvt1", b"grid", b"Exif"])));
    // A file with no item at all is left to the decoder, which finds no picture in it.
    assert!(only_hevc_items(&file_of_items(&[])));

    for kind in [
        b"av01",
        b"unci",
        b"j2k1",
        b"jpeg",
        b"vvc1",
        b"avc1",
        b"abcd",
        b"\0\0\0\0",
        b"HVC1",
    ] {
        assert!(!only_hevc_items(&file_of_items(&[kind])), "{kind:?}");
        // One item that is not allowed spoils the file, wherever it is.
        assert!(!only_hevc_items(&file_of_items(&[b"hvc1", kind])), "{kind:?} after");
        assert!(!only_hevc_items(&file_of_items(&[kind, b"hvc1"])), "{kind:?} before");
    }
}

#[test]
fn an_infe_has_to_be_version_2_or_3() {
    let with = |entry: Vec<u8>| file_with(&[iinf(&[entry])]);

    assert!(only_hevc_items(&with(infe(2, b"hvc1"))));
    assert!(only_hevc_items(&with(infe(3, b"hvc1"))));
    assert!(!only_hevc_items(&with(infe(3, b"av01"))));
    for version in [0, 1, 4, 255] {
        assert!(!only_hevc_items(&with(infe(version, b"hvc1"))), "version {version}");
    }

    // Too short for the type, whatever the version says.
    assert!(!only_hevc_items(&with(boxed(
        b"infe",
        &[2, 0, 0, 0, 0, 1, 0, 0, b'h', b'v']
    ))));
    assert!(!only_hevc_items(&with(boxed(
        b"infe",
        &[3, 0, 0, 0, 0, 0, 0, 1, 0, 0, b'h', b'v', b'c']
    ))));
    assert!(!only_hevc_items(&with(boxed(b"infe", &[]))));
    // Something in the list that is not an infe.
    assert!(!only_hevc_items(&with(boxed(b"free", &[0; 16]))));
}

#[test]
fn the_count_of_an_iinf_is_not_trusted_and_a_version_1_iinf_has_a_wide_one() {
    // Declares no entry and holds an AV1 one.
    let mut lying = iinf(&[infe(2, b"av01")]);
    lying[13] = 0;
    assert!(!only_hevc_items(&file_with(&[lying])));

    // Version 1: a 32-bit count.
    let mut wide = boxed(b"iinf", &[1, 0, 0, 0, 0, 0, 0, 1]);
    wide.extend_from_slice(&infe(2, b"hvc1"));
    let length = u32::try_from(wide.len()).expect("small").to_be_bytes();
    wide[..4].copy_from_slice(&length);
    assert!(only_hevc_items(&file_with(&[wide])));
}

#[test]
fn a_file_without_what_it_needs_or_with_two_of_it_is_refused() {
    let one = iinf(&[infe(2, b"hvc1")]);

    assert!(only_hevc_items(&file_with(std::slice::from_ref(&one))));

    // No meta, no iinf in the meta, two iinf (which one is read is not for a reader to guess), two meta.
    assert!(!only_hevc_items(&boxed(b"ftyp", b"heic\0\0\0\0mif1heic")));
    assert!(!only_hevc_items(&file_with(&[boxed(b"hdlr", &[0; 24])])));
    assert!(!only_hevc_items(&file_with(&[one.clone(), iinf(&[infe(2, b"av01")])])));
    let mut twice = file_with(std::slice::from_ref(&one));
    twice.extend_from_slice(&file_with(&[iinf(&[infe(2, b"av01")])]));
    assert!(!only_hevc_items(&twice));
    // An iinf that is not a direct child of the meta is not looked at.
    assert!(!only_hevc_items(&file_with(&[boxed(b"iprp", &one)])));
    assert!(!only_hevc_items(&[]));
}

#[test]
fn boxes_that_are_not_what_they_say_are_refused_and_never_a_panic() {
    let whole = file_with(&[iinf(&[infe(2, b"hvc1")])]);
    assert!(only_hevc_items(&whole));

    // Every cut of the file: none panics, and a cut inside the meta (after the 24 bytes of ftyp) is refused.
    for cut in 0..whole.len() {
        assert!(!only_hevc_items(&whole[..cut]), "cut at {cut}");
    }
    for cut in 0..TINY.len().min(400) {
        let _ = only_hevc_items(&TINY[..cut]);
        let _ = only_hevc_items(&AV1[..cut.min(AV1.len())]);
    }

    // Sizes: under the head and past the end, the 64-bit size of 1 (huge, under its head, past the end, one that fits)
    // and the size of 0 (to the end).
    let meta_at = 24;
    for declared in [7_u32, 8, 9, 100, u32::MAX] {
        let mut bytes = whole.clone();
        bytes[meta_at..meta_at + 4].copy_from_slice(&declared.to_be_bytes());
        assert!(!only_hevc_items(&bytes), "meta of {declared}");
    }

    let mut large = whole[..meta_at].to_vec();
    large.extend_from_slice(&1_u32.to_be_bytes());
    large.extend_from_slice(b"meta");
    let content = &whole[meta_at + 8..];
    large.extend_from_slice(&(16 + content.len() as u64).to_be_bytes());
    large.extend_from_slice(content);
    assert!(only_hevc_items(&large), "a 64-bit size that fits");
    for declared in [0_u64, 15, 16 + content.len() as u64 + 1, u64::MAX, 1 << 63] {
        let mut bytes = large.clone();
        bytes[meta_at + 8..meta_at + 16].copy_from_slice(&declared.to_be_bytes());
        assert!(!only_hevc_items(&bytes), "64-bit size {declared}");
    }

    let mut to_end = whole.clone();
    to_end[meta_at..meta_at + 4].copy_from_slice(&0_u32.to_be_bytes());
    assert!(only_hevc_items(&to_end), "a meta to the end of the file");
    // A box inside the meta that claims more than the meta holds.
    let mut inner = whole.clone();
    inner[meta_at + 12..meta_at + 16].copy_from_slice(&1000_u32.to_be_bytes());
    assert!(!only_hevc_items(&inner));
}

#[test]
fn a_box_after_the_meta_that_is_cut_short_does_not_refuse_the_file() {
    let mut bytes = file_with(&[iinf(&[infe(2, b"hvc1")])]);

    // A truncated mdat, as a file that was not downloaded in full has: the decoder says what it makes of it.
    bytes.extend_from_slice(&1_000_u32.to_be_bytes());
    bytes.extend_from_slice(b"mdat");
    bytes.extend_from_slice(&[0; 20]);

    assert!(only_hevc_items(&bytes));
}

#[test]
fn a_walk_has_a_bound() {
    let many = |count: usize| {
        let entries = vec![infe(2, b"hvc1"); count];

        file_with(&[iinf(&entries)])
    };

    assert!(only_hevc_items(&many(4000)));
    assert!(!only_hevc_items(&many(5000)));

    // So many boxes at the top that the walk gives up before the meta.
    let mut bytes = Vec::new();
    for _ in 0..5000 {
        bytes.extend_from_slice(&boxed(b"free", &[]));
    }
    bytes.extend_from_slice(&file_of_items(&[b"hvc1"]));
    assert!(!only_hevc_items(&bytes));
}

#[test]
fn bytes_that_are_noise_are_refused_and_never_a_panic() {
    let mut state = 0x1234_5678_u32;
    let mut next = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);

        (state >> 24) as u8
    };

    for length in [0, 1, 7, 8, 16, 64, 500, 4000] {
        for _ in 0..20 {
            let noise: Vec<u8> = (0..length).map(|_| next()).collect();
            assert!(!only_hevc_items(&noise));
        }
    }

    // The fixture with one byte changed at a time: whatever the walk makes of it, it ends.
    for at in 0..TINY.len().min(200) {
        let mut bytes = TINY.to_vec();
        bytes[at] ^= 0xFF;
        let _ = only_hevc_items(&bytes);
    }
}
