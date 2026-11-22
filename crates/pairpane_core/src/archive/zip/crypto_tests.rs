//! Encrypted entries: ZipCrypto and WinZip AES (AE-1 and AE-2, three strengths) round trips, the vectors of another
//! writer, missing and wrong passwords, damaged data, and the cheap password check.

use std::io;

use pretty_assertions::assert_eq;

use crate::archive::testing::{Member, zip_members, zip_of};
use crate::archive::{Password, PasswordFault, check_password, password_fault};

use super::testing::{DEFLATE, STORED, aes, failure, index_of, read_entries, read_only, traditional};
use super::vectors::{self, CONTENT, PASSWORD};

const TEXT: &[u8] = b"the quick brown fox jumps over the lazy dog, again and again and again";

fn fault(error: &io::Error) -> Option<PasswordFault> {
    password_fault(error)
}

fn accepts(bytes: &Vec<u8>, password: &str) -> bool {
    let index = index_of(bytes).expect("an index");

    check_password(&index, bytes, &Password::new(password.to_owned())).expect("a verdict")
}

#[test]
fn zipcrypto_entries_are_read_with_the_password() {
    for (method, descriptor) in [(STORED, false), (DEFLATE, false), (DEFLATE, true), (STORED, true)] {
        let bytes = zip_members(&[traditional("a", TEXT, method, "pw", descriptor)]);

        assert_eq!(
            read_only(&bytes, Some("pw")).expect("read"),
            TEXT,
            "{method} {descriptor}"
        );
    }
}

#[test]
fn zipcrypto_asks_for_a_password_and_says_when_it_is_wrong() {
    let bytes = zip_members(&[traditional("a", TEXT, DEFLATE, "right", false)]);

    assert_eq!(fault(&failure(&bytes, None)), Some(PasswordFault::Needed));

    // Whatever the check byte lets through, the data that follows does not check out and is a wrong password too.
    for wrong in ["", "wrong", "Right", "right ", "another", "12345"] {
        assert_eq!(
            fault(&failure(&bytes, Some(wrong))),
            Some(PasswordFault::Wrong),
            "{wrong:?}"
        );
    }
}

#[test]
fn aes_entries_of_every_strength_and_version_are_read() {
    for strength in 1..=3 {
        for version in [1, 2] {
            for method in [STORED, DEFLATE] {
                let bytes = zip_members(&[aes("a", TEXT, method, "pw", strength, version)]);

                assert_eq!(
                    read_only(&bytes, Some("pw")).expect("read"),
                    TEXT,
                    "strength {strength} AE-{version} method {method}"
                );
            }
        }
    }
}

#[test]
fn aes_asks_for_a_password_and_says_when_it_is_wrong() {
    let bytes = zip_members(&[aes("a", TEXT, DEFLATE, "right", 3, 2)]);

    assert_eq!(fault(&failure(&bytes, None)), Some(PasswordFault::Needed));
    assert_eq!(fault(&failure(&bytes, Some("wrong"))), Some(PasswordFault::Wrong));
    assert_eq!(fault(&failure(&bytes, Some(""))), Some(PasswordFault::Wrong));
}

#[test]
fn ae1_keeps_the_checksum_and_ae2_does_not() {
    let mut wrong_crc_ae1 = aes("a", TEXT, DEFLATE, "pw", 3, 1);
    wrong_crc_ae1.crc ^= 1;
    let error = failure(&zip_members(&[wrong_crc_ae1]), Some("pw"));
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(fault(&error), None);

    // AE-2 stores no CRC: whatever the directory says is not checked.
    let mut noisy_crc_ae2 = aes("a", TEXT, DEFLATE, "pw", 3, 2);
    assert_eq!(noisy_crc_ae2.crc, 0);
    noisy_crc_ae2.crc = 0xDEAD_BEEF;
    assert_eq!(
        read_only(&zip_members(&[noisy_crc_ae2]), Some("pw")).expect("read"),
        TEXT
    );
}

#[test]
fn damaged_aes_data_is_damage_and_not_a_wrong_password() {
    let mut member = aes("a", TEXT, STORED, "pw", 2, 2);
    // A byte of the ciphertext (after the 12-byte salt and the verifier).
    member.data[16 + 2 + 5] ^= 0x40;
    let error = failure(&zip_members(&[member]), Some("pw"));

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(fault(&error), None);
    assert_eq!(error.to_string(), "an entry's data is damaged");

    // The tag itself damaged.
    let mut member = aes("a", TEXT, DEFLATE, "pw", 2, 2);
    *member.data.last_mut().expect("a tag") ^= 1;
    let error = failure(&zip_members(&[member]), Some("pw"));
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(fault(&error), None);

    // Data cut short of its tag.
    let mut member = aes("a", TEXT, DEFLATE, "pw", 2, 2);
    member.data.truncate(16 + 2 + 3);
    assert_eq!(
        failure(&zip_members(&[member]), Some("pw")).kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn the_vectors_of_another_writer_are_read() {
    for (name, bytes) in [
        ("zipcrypto", vectors::zipcrypto()),
        ("aes128", vectors::aes128()),
        ("aes256", vectors::aes256()),
    ] {
        assert_eq!(read_only(&bytes, Some(PASSWORD)).expect("read"), CONTENT, "{name}");
        assert_eq!(fault(&failure(&bytes, None)), Some(PasswordFault::Needed), "{name}");
        assert_eq!(
            fault(&failure(&bytes, Some("nope"))),
            Some(PasswordFault::Wrong),
            "{name}"
        );
        assert!(accepts(&bytes, PASSWORD), "{name}");
        assert!(!accepts(&bytes, "nope"), "{name}");
    }
}

#[test]
fn the_password_check_looks_at_the_first_encrypted_file_and_nothing_else() {
    let plain = zip_of(&[("a", b"1")]);
    assert!(accepts(&plain, "anything"));

    let mixed = zip_members(&[
        Member::stored("plain", b"1"),
        Member::stored("folder/", b""),
        aes("secret", TEXT, DEFLATE, "pw", 1, 2),
    ]);
    assert!(accepts(&mixed, "pw"));
    assert!(!accepts(&mixed, "other"));

    let old = zip_members(&[traditional("secret", TEXT, DEFLATE, "pw", true)]);
    assert!(accepts(&old, "pw"));
    assert!(!accepts(&old, "other"));
}

#[test]
fn an_encrypted_entry_does_not_stop_the_others() {
    let bytes = zip_members(&[
        Member::stored("plain", b"1"),
        aes("secret", TEXT, DEFLATE, "pw", 3, 2),
        Member::stored("after", b"2"),
    ]);

    let got = read_entries(&bytes, None, &[]).expect("a source");
    assert_eq!(got[0].1.as_ref().expect("plain"), b"1");
    assert_eq!(
        fault(got[1].1.as_ref().expect_err("needs it")),
        Some(PasswordFault::Needed)
    );
    assert_eq!(got[2].1.as_ref().expect("after"), b"2");

    let got = read_entries(&bytes, Some("pw"), &[]).expect("a source");
    assert_eq!(got[1].1.as_ref().expect("secret"), TEXT);
}
