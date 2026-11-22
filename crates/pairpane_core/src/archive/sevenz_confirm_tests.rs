//! Confirming a 7z password before an archive is changed: the chosen entry is read to its end, and a cancel ends it.

use std::io::{self, Cursor};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use pretty_assertions::assert_eq;

use crate::archive::write::{Entry, Options, SevenZWriter};
use crate::model::PackLevel;

use super::Password;
use super::file::ReadAt;
use super::sevenz::{check_password, confirm_password};
use super::testing::{SevenZ, idle, noise, project};

fn password(text: &str) -> Password {
    Password::new(text.to_owned())
}

/// A stored, encrypted 7z with one entry of `length` bytes.
fn stored_secret(length: usize) -> Vec<u8> {
    let cancel = idle();
    let options = Options {
        level: PackLevel::Store,
        password: Some(password("sesame")),
        encrypt_names: false,
    };
    let content = noise(length);
    let mut writer = SevenZWriter::new(Cursor::new(Vec::new()), &options, &cancel).expect("a 7z writer");
    writer
        .add(Entry::file(
            "big.bin",
            0o644,
            None,
            content.len() as u64,
            Cursor::new(content),
        ))
        .expect("added");

    writer.finish().expect("a 7z").into_inner()
}

#[test]
fn the_confirmation_reads_a_stored_entry_longer_than_the_probe_to_its_end_and_tells_a_wrong_password() {
    let bytes = stored_secret(128 * 1024);

    // The cheap check stops at 64 KiB, before the checksum that would tell.
    assert!(check_password(&bytes, &password("wrong")).expect("checked"));

    assert!(!confirm_password(&bytes, &idle(), &password("wrong")).expect("confirmed"));
    assert!(confirm_password(&bytes, &idle(), &password("sesame")).expect("confirmed"));
}

#[test]
fn the_confirmation_accepts_any_password_for_a_7z_with_nothing_encrypted_and_stops_for_a_cancel() {
    let plain = SevenZ::default().of(&project());
    assert!(confirm_password(&plain, &idle(), &password("anything")).expect("confirmed"));

    let cancel = AtomicBool::new(true);
    let error = confirm_password(&stored_secret(1024), &cancel, &password("sesame")).expect_err("cancelled");
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

/// The bytes of a 7z that raise `cancel` after `reads` reads of them.
struct CancelAfter<'a> {
    bytes: Vec<u8>,
    reads: AtomicU64,
    after: u64,
    cancel: &'a AtomicBool,
}

impl ReadAt for CancelAfter<'_> {
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        let count = self.bytes.read_at(buffer, offset)?;
        self.cancel.fetch_or(
            self.reads.fetch_add(1, Ordering::Relaxed) >= self.after,
            Ordering::Relaxed,
        );

        Ok(count)
    }

    fn size(&self) -> io::Result<u64> {
        self.bytes.size()
    }
}

#[test]
fn a_cancel_raised_while_the_password_is_confirmed_ends_it_promptly_and_is_not_a_wrong_password() {
    let cancel = AtomicBool::new(false);
    let file = CancelAfter {
        bytes: stored_secret(8 * 1024 * 1024),
        reads: AtomicU64::new(0),
        after: 50,
        cancel: &cancel,
    };

    let started = std::time::Instant::now();
    let error = confirm_password(&file, &cancel, &password("sesame")).expect_err("cancelled");

    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "it must not spin on the cancel"
    );
}
