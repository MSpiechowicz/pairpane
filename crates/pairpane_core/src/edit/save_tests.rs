//! Saving from the editor: that the bytes are exact, that a changed file is not overwritten by mistake, how the file
//! is replaced, and that nothing is left behind or written through a link.

use std::fs::{self, File};
use std::io::ErrorKind;
use std::time::{Duration, SystemTime};

use pretty_assertions::assert_eq;

use crate::model::{EditOpened, EditSaveOptions, EditSaved, LineEnding};

use super::testing::{document, names, options, put, read, save_now, temp};
use super::{MAX_EDIT, MAX_SAVE_TEXT, open, save};

fn saved(result: std::io::Result<EditSaved>) -> (u64, String) {
    match result.expect("save") {
        EditSaved::Saved { size, revision } => (size, revision),
        other => panic!("not saved: {other:?}"),
    }
}

#[test]
fn the_text_is_written_in_the_chosen_encoding_mark_and_line_break() {
    let folder = temp();
    put(folder.path(), "a.txt", b"old");
    let revision = document(folder.path(), "a.txt").revision;

    let chosen = EditSaveOptions {
        encoding: "UTF-16LE".to_owned(),
        bom: true,
        line_ending: LineEnding::Crlf,
        ..options(&revision)
    };
    let (size, _) = saved(save(folder.path(), "a.txt", "a\nb", &chosen));

    let expected = [0xFF, 0xFE, b'a', 0, b'\r', 0, b'\n', 0, b'b', 0];
    assert_eq!(read(folder.path(), "a.txt"), expected);
    assert_eq!(size, expected.len() as u64);
}

#[test]
fn a_file_opened_and_saved_unchanged_is_the_same_bytes() {
    let folder = temp();
    let original = [&[0xEF, 0xBB, 0xBF][..], "zażółć\r\nżółw\r\n".as_bytes()].concat();
    put(folder.path(), "a.txt", &original);

    let document = document(folder.path(), "a.txt");
    let again = EditSaveOptions {
        encoding: document.encoding.clone(),
        bom: document.bom,
        line_ending: document.line_ending,
        ..options(&document.revision)
    };
    saved(save(folder.path(), "a.txt", &document.text, &again));

    assert_eq!(read(folder.path(), "a.txt"), original);
}

#[test]
fn the_revision_that_comes_back_is_the_one_the_next_open_sees() {
    let folder = temp();
    put(folder.path(), "a.txt", b"one");
    let revision = document(folder.path(), "a.txt").revision;

    let (size, after) = saved(save(folder.path(), "a.txt", "two words", &options(&revision)));

    assert_eq!(size, 9);
    assert_eq!(document(folder.path(), "a.txt").revision, after);

    // So the next save from the editor goes through without a change being reported.
    saved(save(folder.path(), "a.txt", "three", &options(&after)));
    assert_eq!(read(folder.path(), "a.txt"), b"three");
}

#[test]
fn a_file_that_changed_since_it_was_read_is_not_overwritten_unless_forced() {
    let folder = temp();
    put(folder.path(), "a.txt", b"one");
    let revision = document(folder.path(), "a.txt").revision;
    put(folder.path(), "a.txt", b"changed elsewhere");

    let answer = save(folder.path(), "a.txt", "mine", &options(&revision)).expect("save");
    assert_eq!(answer, EditSaved::Changed { gone: false });
    assert_eq!(read(folder.path(), "a.txt"), b"changed elsewhere");

    let forced = EditSaveOptions {
        force: true,
        ..options(&revision)
    };
    saved(save(folder.path(), "a.txt", "mine", &forced));
    assert_eq!(read(folder.path(), "a.txt"), b"mine");
}

#[test]
fn a_change_of_the_time_alone_is_a_change() {
    let folder = temp();
    put(folder.path(), "a.txt", b"same size");
    let revision = document(folder.path(), "a.txt").revision;
    let file = File::options()
        .write(true)
        .open(folder.path().join("a.txt"))
        .expect("open");
    file.set_modified(SystemTime::now() + Duration::from_secs(3600))
        .expect("touch");
    drop(file);

    let answer = save(folder.path(), "a.txt", "mine", &options(&revision)).expect("save");

    assert_eq!(answer, EditSaved::Changed { gone: false });
    assert_eq!(read(folder.path(), "a.txt"), b"same size");
}

#[test]
fn a_file_that_is_gone_is_told_and_made_again_only_when_forced() {
    let folder = temp();
    put(folder.path(), "a.txt", b"one");
    let revision = document(folder.path(), "a.txt").revision;
    fs::remove_file(folder.path().join("a.txt")).expect("remove");

    let answer = save(folder.path(), "a.txt", "mine", &options(&revision)).expect("save");
    assert_eq!(answer, EditSaved::Changed { gone: true });
    assert_eq!(names(folder.path()), Vec::<String>::new());

    let forced = EditSaveOptions {
        force: true,
        ..options(&revision)
    };
    let (size, after) = saved(save(folder.path(), "a.txt", "mine", &forced));

    assert_eq!(read(folder.path(), "a.txt"), b"mine");
    assert_eq!(size, 4);
    assert_eq!(document(folder.path(), "a.txt").revision, after);
}

#[test]
fn a_text_that_cannot_be_saved_is_refused_before_the_disk_is_touched() {
    let folder = temp();
    put(folder.path(), "a.txt", b"safe");
    let revision = document(folder.path(), "a.txt").revision;

    let huge = "a".repeat(MAX_SAVE_TEXT + 1);
    let too_large = save(folder.path(), "a.txt", &huge, &options(&revision)).expect_err("refused");
    assert_eq!(too_large.to_string(), "the text is too large to save");

    let latin2 = EditSaveOptions {
        encoding: "windows-1250".to_owned(),
        ..options(&revision)
    };
    let unmappable = save(folder.path(), "a.txt", "sekret 日本語", &latin2).expect_err("refused");
    assert_eq!(unmappable.kind(), ErrorKind::InvalidInput);
    assert_eq!(
        unmappable.to_string(),
        "some characters cannot be saved in this encoding"
    );

    let marked = EditSaveOptions { bom: true, ..latin2 };
    assert_eq!(
        save(folder.path(), "a.txt", "abc", &marked)
            .expect_err("refused")
            .to_string(),
        "a byte order mark belongs to UTF-8 and UTF-16 only"
    );

    assert_eq!(read(folder.path(), "a.txt"), b"safe");
    assert_eq!(names(folder.path()), ["a.txt"]);
}

#[test]
fn the_biggest_file_in_a_one_byte_encoding_can_be_saved_again() {
    // Every `€` is one byte in windows-1252 and three in UTF-8: the editor's text is three times the file.
    let folder = temp();
    let original = vec![0x80_u8; MAX_EDIT];
    put(folder.path(), "a.txt", &original);

    let EditOpened::Text { document } = open(folder.path(), "a.txt", Some("windows-1252")).expect("open") else {
        panic!("not text");
    };
    assert_eq!(document.encoding, "windows-1252");
    assert_eq!(document.text.len(), 3 * MAX_EDIT);
    let again = EditSaveOptions {
        encoding: document.encoding.clone(),
        ..options(&document.revision)
    };
    saved(save(folder.path(), "a.txt", &document.text, &again));

    assert_eq!(read(folder.path(), "a.txt"), original);
}

#[test]
fn a_folder_and_a_bad_name_are_refused() {
    let folder = temp();
    fs::create_dir(folder.path().join("inner")).expect("folder");

    let in_folder = save(folder.path(), "inner", "x", &options("")).expect_err("refused");
    assert_eq!(in_folder.to_string(), "the editor saves files, not folders");

    for name in ["", "..", "a/b"] {
        let error = save(folder.path(), name, "x", &options("")).expect_err("refused");
        assert_eq!(error.kind(), ErrorKind::InvalidInput, "{name:?}");
    }
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    use std::io::ErrorKind;
    use std::path::Path;

    use pretty_assertions::assert_eq;

    use super::{EditSaveOptions, document, fs, names, options, put, read, save, save_now, saved, temp};
    use crate::edit::disk::{Strategy, has_xattrs, strategy};
    use crate::edit::testing::is_root;

    fn inode(folder: &Path, name: &str) -> u64 {
        fs::metadata(folder.join(name)).expect("metadata").ino()
    }

    /// Removes the extended attributes of a file made for a test, where the system lets it (macOS gives a fresh file a
    /// `com.apple.provenance` one, which it may not let go), and tells whether a save would replace it by a new file:
    /// not one that still has an attribute, which a new file would lose, so it is written in place.
    fn renames_here(folder: &Path, name: &str) -> bool {
        let path = folder.join(name);

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let mut list = vec![0_u8; 8192];
            if let Ok(length) = rustix::fs::listxattr(&path, &mut list[..]) {
                for attribute in list[..length].split(|byte| *byte == 0).filter(|name| !name.is_empty()) {
                    let _ = rustix::fs::removexattr(&path, attribute);
                }
            }
        }

        !has_xattrs(&fs::File::open(path).expect("open"))
    }

    fn mode(folder: &Path, name: &str) -> u32 {
        fs::metadata(folder.join(name)).expect("metadata").permissions().mode() & 0o7777
    }

    #[test]
    fn a_plain_file_is_replaced_by_a_new_one_with_its_permissions_and_nothing_is_left_behind() {
        let folder = temp();
        put(folder.path(), "a.txt", b"old");
        fs::set_permissions(folder.path().join("a.txt"), fs::Permissions::from_mode(0o640)).expect("chmod");
        let renames = renames_here(folder.path(), "a.txt");
        let before = inode(folder.path(), "a.txt");

        save_now(folder.path(), "a.txt", "new text");

        assert_eq!(read(folder.path(), "a.txt"), b"new text");
        if renames {
            assert_ne!(inode(folder.path(), "a.txt"), before);
        }
        assert_eq!(mode(folder.path(), "a.txt"), 0o640);
        assert_eq!(names(folder.path()), ["a.txt"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_file_with_an_extended_attribute_is_written_in_place_and_keeps_it() {
        let folder = temp();
        put(folder.path(), "a.txt", b"old");
        let file = fs::File::open(folder.path().join("a.txt")).expect("open");
        // A filesystem that keeps no user attributes (tmpfs on some kernels) cannot show it: nothing to test there.
        if rustix::fs::fsetxattr(&file, "user.pairpane", b"1", rustix::fs::XattrFlags::empty()).is_err() {
            return;
        }
        drop(file);
        let before = inode(folder.path(), "a.txt");

        save_now(folder.path(), "a.txt", "new text");

        let kept = fs::File::open(folder.path().join("a.txt")).expect("open");
        let mut value = [0_u8; 8];
        let length = rustix::fs::fgetxattr(&kept, "user.pairpane", &mut value).expect("the attribute is kept");
        assert_eq!(&value[..length], b"1");
        assert_eq!(inode(folder.path(), "a.txt"), before);
        assert_eq!(read(folder.path(), "a.txt"), b"new text");
        assert_eq!(names(folder.path()), ["a.txt"]);
    }

    #[test]
    fn a_new_file_does_not_carry_the_setuid_and_setgid_bits() {
        let folder = temp();
        put(folder.path(), "a.txt", b"old");
        fs::set_permissions(folder.path().join("a.txt"), fs::Permissions::from_mode(0o6755)).expect("chmod");
        let renames = renames_here(folder.path(), "a.txt");
        let before = inode(folder.path(), "a.txt");

        save_now(folder.path(), "a.txt", "new text");

        if renames {
            assert_ne!(inode(folder.path(), "a.txt"), before, "replaced by a new file");
        }
        assert_eq!(mode(folder.path(), "a.txt"), 0o755);
    }

    #[test]
    fn a_file_with_another_name_is_written_in_place_so_both_names_see_the_text() {
        let folder = temp();
        put(folder.path(), "a.txt", b"a much longer old text");
        fs::hard_link(folder.path().join("a.txt"), folder.path().join("b.txt")).expect("link");
        let before = inode(folder.path(), "a.txt");

        save_now(folder.path(), "a.txt", "short");

        assert_eq!(inode(folder.path(), "a.txt"), before);
        assert_eq!(read(folder.path(), "a.txt"), b"short");
        assert_eq!(read(folder.path(), "b.txt"), b"short");
        assert_eq!(names(folder.path()), ["a.txt", "b.txt"]);
    }

    #[test]
    fn a_folder_that_takes_no_new_file_still_saves_in_place() {
        if is_root() {
            return;
        }

        let folder = temp();
        put(folder.path(), "a.txt", b"old");
        let before = inode(folder.path(), "a.txt");
        let revision = document(folder.path(), "a.txt").revision;
        fs::set_permissions(folder.path(), fs::Permissions::from_mode(0o555)).expect("chmod");

        let result = save(folder.path(), "a.txt", "new", &options(&revision));
        fs::set_permissions(folder.path(), fs::Permissions::from_mode(0o755)).expect("chmod");

        saved(result);
        assert_eq!(read(folder.path(), "a.txt"), b"new");
        assert_eq!(inode(folder.path(), "a.txt"), before);
        assert_eq!(names(folder.path()), ["a.txt"]);
    }

    #[test]
    fn a_file_the_user_cannot_write_is_refused_and_untouched() {
        if is_root() {
            return;
        }

        let folder = temp();
        put(folder.path(), "a.txt", b"fixed");
        fs::set_permissions(folder.path().join("a.txt"), fs::Permissions::from_mode(0o444)).expect("chmod");
        let revision = document(folder.path(), "a.txt").revision;

        let error = save(folder.path(), "a.txt", "new", &options(&revision)).expect_err("refused");

        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "the file is read-only");
        assert_eq!(read(folder.path(), "a.txt"), b"fixed");
        assert_eq!(names(folder.path()), ["a.txt"]);
    }

    #[test]
    fn a_file_swapped_for_a_link_is_refused_and_what_it_leads_to_is_untouched() {
        let folder = temp();
        let elsewhere = temp();
        put(folder.path(), "a.txt", b"mine");
        put(elsewhere.path(), "target.txt", b"not yours");
        let revision = document(folder.path(), "a.txt").revision;
        fs::remove_file(folder.path().join("a.txt")).expect("remove");
        symlink(elsewhere.path().join("target.txt"), folder.path().join("a.txt")).expect("link");

        let normal = save(folder.path(), "a.txt", "new", &options(&revision)).expect_err("refused");
        let forced = EditSaveOptions {
            force: true,
            ..options(&revision)
        };
        let again = save(folder.path(), "a.txt", "new", &forced).expect_err("refused");

        for error in [normal, again] {
            assert_eq!(error.kind(), ErrorKind::InvalidInput);
            assert_eq!(error.to_string(), "the editor saves regular files, not links");
        }
        assert_eq!(read(elsewhere.path(), "target.txt"), b"not yours");
        assert_eq!(names(elsewhere.path()), ["target.txt"]);
    }

    #[test]
    fn the_file_is_replaced_only_when_it_is_the_users_own_with_one_name_and_a_group_that_stays() {
        let rename = Strategy::Rename;
        let in_place = Strategy::InPlace;

        // nlink, owner, group, me, group of the new file, extended attributes
        assert_eq!(strategy(1, 1000, 100, 1000, 100, false), rename);
        assert_eq!(strategy(2, 1000, 100, 1000, 100, false), in_place, "another name");
        assert_eq!(strategy(1, 0, 0, 1000, 100, false), in_place, "another owner");
        assert_eq!(
            strategy(1, 1000, 100, 1000, 200, false),
            in_place,
            "a group a new file would not keep"
        );
        assert_eq!(
            strategy(1, 1000, 100, 1000, 100, true),
            in_place,
            "ACLs and labels a new file would not have"
        );
    }
}
