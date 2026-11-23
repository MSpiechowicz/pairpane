//! Which attribute words mean "the data is not on this computer", and that an ordinary file is not one.

use super::placeholder::{is_dataless, is_offline, is_placeholder};

#[test]
fn each_of_the_three_windows_attributes_of_a_placeholder_is_one() {
    assert!(is_offline(0x1000), "offline");
    assert!(is_offline(0x4_0000), "recall on open");
    assert!(is_offline(0x40_0000), "recall on data access");
    assert!(is_offline(0x20 | 0x40_0000 | 0x2), "among others");
}

#[test]
fn the_other_windows_attributes_are_not() {
    // Read-only, hidden, system, directory, archive, normal, temporary, sparse, reparse point, compressed, encrypted.
    for attributes in [0, 0x1, 0x2, 0x4, 0x10, 0x20, 0x80, 0x100, 0x200, 0x400, 0x800, 0x4000] {
        assert!(!is_offline(attributes), "{attributes:#x}");
    }
}

#[test]
fn only_the_dataless_flag_of_macos_is_a_placeholder() {
    assert!(is_dataless(0x4000_0000));
    assert!(is_dataless(0x4000_0000 | 0x8000));

    // `UF_NODUMP`, `UF_HIDDEN`, `SF_ARCHIVED`, `SF_IMMUTABLE`.
    for flags in [0, 0x1, 0x8000, 0x1_0000, 0x2_0000] {
        assert!(!is_dataless(flags), "{flags:#x}");
    }
}

#[test]
fn an_ordinary_file_is_not_a_placeholder() {
    let folder = tempfile::tempdir().expect("folder");
    let file = folder.path().join("a.txt");
    std::fs::write(&file, b"abc").expect("file");

    assert!(!is_placeholder(&std::fs::metadata(&file).expect("metadata")));
}
