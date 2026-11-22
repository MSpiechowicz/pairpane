//! 7z archives: listing from the headers alone, what the attributes make of an entry, extraction of chosen entries (solid
//! and not) without decoding what is not wanted, cancel, passwords on content and on headers.

use std::io::{self, Cursor};
use std::sync::atomic::{AtomicBool, Ordering};

use pretty_assertions::assert_eq;

use crate::archive::write::{Entry, Options, SevenZWriter};
use crate::model::{ArchiveFormat, PackLevel};

use super::entries::{EntryType, Flow, Record};
use super::error::{PasswordFault, password_fault};
use super::sevenz::{check_password, confirm_password};
use super::testing::{
    Counting, MOMENT, SevenZ, assert_stop_and_failure, extract, idle, index_of, noise, project, root, sevenz_entry,
};
use super::{Input, Password, build_index, open_source, records};

fn password(text: &str) -> Password {
    Password::new(text.to_owned())
}

/// The records of the 7z archive `file`, the way a listing sees them.
fn listed(file: &dyn super::ReadAt, secret: Option<&Password>) -> io::Result<Vec<Record>> {
    let mut all = Vec::new();

    records(
        ArchiveFormat::SevenZ,
        &Input { file, path: None },
        &idle(),
        u64::MAX,
        secret,
        &mut |record| {
            all.push(record);

            Flow::Continue
        },
    )?;

    Ok(all)
}

fn fault_of(result: io::Result<impl Sized>) -> Option<PasswordFault> {
    password_fault(&result.err().expect("an error"))
}

#[test]
fn a_project_is_indexed_whole_or_solid() {
    for solid in [false, true] {
        let bytes = SevenZ {
            solid,
            ..SevenZ::default()
        }
        .of(&project());
        let index = index_of(&bytes, ArchiveFormat::SevenZ).expect("an index");

        assert_eq!(index.format(), ArchiveFormat::SevenZ);
        assert!(!index.truncated());
        assert_eq!(
            root(&index),
            [("src".to_owned(), true), ("README".to_owned(), false)],
            "solid {solid}"
        );

        let readme = index.find("", "README").and_then(|child| child.entry).expect("README");
        assert_eq!((readme.size, readme.modified), (7, Some(MOMENT)), "solid {solid}");
    }
}

#[test]
fn listing_reads_the_headers_and_decodes_no_data() {
    let big = noise(1024 * 1024);
    let file = Counting::new(SevenZ::default().of(&[("one.bin", &big), ("two.bin", &big), ("dir/", b"")]));

    let all = listed(&file, None).expect("listed");

    assert_eq!(
        all.iter().map(|record| record.size).collect::<Vec<_>>(),
        [1 << 20, 1 << 20, 0]
    );
    assert!(file.read() < 4096, "read {}", file.read());
}

#[test]
fn folders_empty_files_and_encrypted_flags_are_recorded() {
    let bytes = SevenZ::default().of(&[("d/", b""), ("d/empty", b""), ("d/full", b"x")]);

    let all = listed(&bytes, None).expect("listed");

    assert_eq!(
        all.iter()
            .map(|record| (record.name.as_str(), record.kind, record.size, record.encrypted))
            .collect::<Vec<_>>(),
        [
            ("d", EntryType::Dir, 0, false),
            ("d/empty", EntryType::File, 0, false),
            ("d/full", EntryType::File, 1, false)
        ]
    );
}

#[test]
fn attributes_tell_the_type_and_the_mode_and_anti_items_are_left_out() {
    let unix = |mode: u32| (mode << 16) | 0x8000;
    let mut entries = Vec::new();

    for (name, attributes) in [
        ("script", Some(unix(0o100_755))),
        ("setuid", Some(unix(0o104_755))),
        ("link", Some(unix(0o120_777))),
        ("pipe", Some(unix(0o010_644))),
        ("folder", Some(unix(0o040_755))),
        ("reparse", Some(0x400)),
        ("hidden", Some(0x2)),
        ("plain", None),
    ] {
        let mut entry = sevenz_entry(name, b"x");
        entry.has_windows_attributes = attributes.is_some();
        entry.windows_attributes = attributes.unwrap_or(0);
        entries.push((entry, &b"x"[..]));
    }

    let mut anti = sevenz_entry("removed", b"");
    anti.is_anti_item = true;
    entries.insert(1, (anti, &b""[..]));

    let all = listed(&SevenZ::default().write(entries), None).expect("listed");

    assert_eq!(
        all.iter()
            .map(|record| (record.name.as_str(), record.kind, record.mode))
            .collect::<Vec<_>>(),
        [
            ("script", EntryType::File, Some(0o755)),
            ("setuid", EntryType::File, Some(0o4755)),
            ("link", EntryType::Symlink, Some(0o777)),
            ("pipe", EntryType::Special, Some(0o644)),
            ("folder", EntryType::Dir, Some(0o755)),
            ("reparse", EntryType::Symlink, None),
            ("hidden", EntryType::File, None),
            ("plain", EntryType::File, None),
        ]
    );
}

#[test]
fn chosen_entries_come_out_in_order_with_folders_and_empty_files_between() {
    let files: [(&str, &[u8]); 5] = [
        ("a.txt", b"first"),
        ("dir/", b""),
        ("dir/empty", b""),
        ("dir/b.txt", b"second"),
        ("c.txt", b"third"),
    ];

    for solid in [false, true] {
        let bytes = SevenZ {
            solid,
            ..SevenZ::default()
        }
        .of(&files);
        let index = index_of(&bytes, ArchiveFormat::SevenZ).expect("an index");

        let got = extract(&index, &bytes, &[0, 1, 2, 3, 4]).expect("extracted");
        assert_eq!(
            got,
            files
                .iter()
                .map(|(name, content)| (name.trim_end_matches('/').to_owned(), content.to_vec()))
                .collect::<Vec<_>>(),
            "solid {solid}"
        );

        let got = extract(&index, &bytes, &[2, 4]).expect("extracted");
        assert_eq!(
            got,
            [
                ("dir/empty".to_owned(), vec![]),
                ("c.txt".to_owned(), b"third".to_vec())
            ],
            "solid {solid}"
        );
    }
}

#[test]
fn a_block_without_a_wanted_entry_is_never_decoded() {
    let big = noise(2 * 1024 * 1024);
    let file = Counting::new(SevenZ::default().of(&[("one.bin", &big), ("two.bin", &big), ("last.txt", b"wanted")]));
    let index = index_of(&file, ArchiveFormat::SevenZ).expect("an index");
    let before = file.read();

    let got = extract(&index, &file, &[2]).expect("extracted");

    assert_eq!(got, [("last.txt".to_owned(), b"wanted".to_vec())]);
    assert!(file.read() - before < 8192, "read {}", file.read() - before);
}

#[test]
fn a_solid_block_is_decoded_up_to_the_last_wanted_entry_only() {
    let big = noise(2 * 1024 * 1024);
    let file = Counting::new(
        SevenZ {
            solid: true,
            ..SevenZ::default()
        }
        .of(&[("first.txt", b"wanted"), ("one.bin", &big), ("two.bin", &big)]),
    );
    let index = index_of(&file, ArchiveFormat::SevenZ).expect("an index");
    let before = file.read();

    let got = extract(&index, &file, &[0]).expect("extracted");

    assert_eq!(got, [("first.txt".to_owned(), b"wanted".to_vec())]);
    assert!(file.read() - before < 512 * 1024, "read {}", file.read() - before);
}

#[test]
fn an_entry_read_short_is_fine_and_a_stop_or_a_failing_visitor_ends_the_extraction() {
    let bytes = SevenZ {
        solid: true,
        ..SevenZ::default()
    }
    .of(&[("a", b"hello"), ("b", b"world"), ("c", b"!")]);
    let index = index_of(&bytes, ArchiveFormat::SevenZ).expect("an index");
    let mut seen = Vec::new();

    open_source(
        &index,
        &Input {
            file: &bytes,
            path: None,
        },
        &idle(),
        None,
    )
    .expect("a source")
    .extract(&[0, 1], &mut |entry, data| {
        let mut first = [0; 2];
        data.read_exact(&mut first)?;
        seen.push((entry.path.clone(), first));

        Ok(Flow::Continue)
    })
    .expect("extracted");
    assert_eq!(seen, [("a".to_owned(), *b"he"), ("b".to_owned(), *b"wo")]);

    assert_stop_and_failure(&index, &bytes);
}

#[test]
fn an_archive_that_changed_since_it_was_indexed_is_an_error_not_a_guess() {
    let index = index_of(
        &SevenZ::default().of(&[("a", b"1"), ("b", b"22"), ("c", b"3")]),
        ArchiveFormat::SevenZ,
    )
    .expect("an index");
    let changed = SevenZ::default().of(&[("a", b"1"), ("b", b"2")]);

    let error = extract(&index, &changed, &[1, 2]).expect_err("changed");

    assert_eq!(error.to_string(), "the archive cannot be read");
}

#[test]
fn a_raised_cancel_flag_stops_indexing_and_one_raised_midway_stops_the_extraction() {
    let big = noise(1024 * 1024);
    let bytes = SevenZ {
        solid: true,
        ..SevenZ::default()
    }
    .of(&[("a", &big), ("b", &big)]);
    let raised = AtomicBool::new(true);

    let error = build_index(
        ArchiveFormat::SevenZ,
        &Input {
            file: &bytes,
            path: None,
        },
        &raised,
        None,
    )
    .expect_err("cancelled");
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);

    let index = index_of(&bytes, ArchiveFormat::SevenZ).expect("an index");
    let cancel = AtomicBool::new(false);
    let error = open_source(
        &index,
        &Input {
            file: &bytes,
            path: None,
        },
        &cancel,
        None,
    )
    .expect("a source")
    .extract(&[0, 1], &mut |_, data| {
        // `read_to_end` would try again at every `Interrupted`: read by hand.
        let mut chunk = [0; 4096];

        while data.read(&mut chunk)? > 0 {
            cancel.store(true, Ordering::Relaxed);
        }

        Ok(Flow::Continue)
    })
    .expect_err("cancelled");
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

#[test]
fn encrypted_content_is_listed_and_needs_the_password_to_extract() {
    let bytes = SevenZ {
        password: Some("sesame"),
        ..SevenZ::default()
    }
    .of(&[("secret.txt", b"top secret"), ("dir/", b"")]);

    let all = listed(&bytes, None).expect("the headers are plain");
    assert_eq!(
        all.iter().map(|record| record.encrypted).collect::<Vec<_>>(),
        [true, false]
    );

    let index = index_of(&bytes, ArchiveFormat::SevenZ).expect("an index");
    let attempt = |secret: Option<&Password>| {
        let mut content = Vec::new();
        let cancel = idle();

        open_source(
            &index,
            &Input {
                file: &bytes,
                path: None,
            },
            &cancel,
            secret,
        )
        .expect("a source")
        .extract(&[0], &mut |_, data| {
            data.read_to_end(&mut content)?;

            Ok(Flow::Continue)
        })
        .map(|()| content)
    };

    assert_eq!(fault_of(attempt(None)), Some(PasswordFault::Needed));
    assert_eq!(fault_of(attempt(Some(&password("wrong")))), Some(PasswordFault::Wrong));
    assert_eq!(attempt(Some(&password("sesame"))).expect("opened"), b"top secret");

    assert!(check_password(&bytes, &password("sesame")).expect("checked"));
    assert!(!check_password(&bytes, &password("wrong")).expect("checked"));
}

#[test]
fn encrypted_headers_need_the_password_even_to_list() {
    let sealed = SevenZ {
        password: Some("sesame"),
        encrypted_headers: true,
        ..SevenZ::default()
    };

    for solid in [false, true] {
        let bytes = SevenZ { solid, ..sealed }.of(&project());

        assert_eq!(fault_of(listed(&bytes, None)), Some(PasswordFault::Needed));
        assert_eq!(
            fault_of(listed(&bytes, Some(&password("wrong")))),
            Some(PasswordFault::Wrong)
        );
        assert_eq!(
            fault_of(build_index(
                ArchiveFormat::SevenZ,
                &Input {
                    file: &bytes,
                    path: None
                },
                &idle(),
                None
            )),
            Some(PasswordFault::Needed)
        );
        assert_eq!(listed(&bytes, Some(&password("sesame"))).expect("listed").len(), 4);

        assert!(check_password(&bytes, &password("sesame")).expect("checked"));
        assert!(!check_password(&bytes, &password("wrong")).expect("checked"));
    }
}

#[test]
fn a_password_for_an_archive_that_has_none_is_harmless_and_damage_is_not_a_wrong_password() {
    let plain = SevenZ::default().of(&project());

    assert_eq!(listed(&plain, Some(&password("anything"))).expect("listed").len(), 4);
    assert!(check_password(&plain, &password("anything")).expect("checked"));

    let mut damaged = plain.clone();
    let last = damaged.len() - 3;
    damaged[last] ^= 0xFF;
    let error = listed(&damaged, Some(&password("anything"))).expect_err("damaged");

    assert_eq!(password_fault(&error), None);
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn a_file_that_is_not_a_7z_is_an_unreadable_archive() {
    let error = listed(&vec![0x37; 100], None).expect_err("not a 7z");

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(error.to_string(), "the archive cannot be read");
}

#[test]
fn a_stored_7z_entry_that_fits_the_probe_is_read_to_its_end_so_a_wrong_password_is_told() {
    let cancel = idle();
    let options = Options {
        level: PackLevel::Store,
        password: Some(password("sesame")),
        encrypt_names: false,
    };
    let mut writer = SevenZWriter::new(Cursor::new(Vec::new()), &options, &cancel)
        .expect("a 7z writer")
        .with_block(1);
    for (name, content) in [("big.bin", noise(100_000)), ("small.txt", b"top secret".to_vec())] {
        writer
            .add(Entry::file(
                name,
                0o644,
                None,
                content.len() as u64,
                Cursor::new(content),
            ))
            .expect("added");
    }
    let bytes = writer.finish().expect("a 7z").into_inner();

    assert!(check_password(&bytes, &password("sesame")).expect("checked"));
    assert!(!check_password(&bytes, &password("wrong")).expect("checked"));
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
