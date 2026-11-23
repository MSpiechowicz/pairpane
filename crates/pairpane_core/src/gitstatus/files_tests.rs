//! What a look at a file of a repository finds (see [`super::files`]).

use super::files::*;
use std::fs;
use std::path::Path;

/// The cap of the rules files in these tests.
const RULES_CAP: u64 = 100;

#[test]
fn a_regular_file_within_the_cap_is_plain_and_one_over_it_is_large() {
    let folder = tempfile::tempdir().expect("a folder");
    fs::write(folder.path().join("small"), b"12345").expect("a file");

    assert_eq!(seen(&folder.path().join("small"), 5), Seen::Plain(()));
    assert_eq!(seen(&folder.path().join("small"), 4), Seen::Large);
    assert_eq!(seen(&folder.path().join("none"), 4), Seen::Missing);
    assert_eq!(read(&folder.path().join("small"), 5), Seen::Plain(b"12345".to_vec()));
    assert_eq!(read(&folder.path().join("small"), 4), Seen::Large);
    assert_eq!(read(&folder.path().join("none"), 4), Seen::Missing);
}

#[test]
fn a_folder_is_odd_and_a_file_is_read_below_real_folders_only() {
    let folder = tempfile::tempdir().expect("a folder");
    fs::create_dir(folder.path().join("sub")).expect("a folder");
    fs::write(folder.path().join("sub/file"), b"x").expect("a file");

    assert_eq!(seen(&folder.path().join("sub"), 10), Seen::Odd);
    assert_eq!(read_below(folder.path(), "sub/file", 10), Seen::Plain(b"x".to_vec()));
    assert_eq!(read_below(folder.path(), "sub/none", 10), Seen::Missing);
    assert_eq!(read_below(folder.path(), "none/file", 10), Seen::Missing);
    assert_eq!(read_below(folder.path(), "sub/file/deeper", 10), Seen::Odd);
}

#[cfg(unix)]
#[test]
fn pipes_devices_and_links_are_odd_and_nothing_waits_for_them() {
    let folder = tempfile::tempdir().expect("a folder");
    crate::view::testing::fifo(folder.path(), "pipe");
    std::os::unix::fs::symlink("/dev/zero", folder.path().join("zero")).expect("a link");
    fs::create_dir(folder.path().join("real")).expect("a folder");
    std::os::unix::fs::symlink(folder.path().join("real"), folder.path().join("via")).expect("a link");
    fs::write(folder.path().join("real/file"), b"x").expect("a file");
    std::os::unix::fs::symlink(folder.path().join("real/file"), folder.path().join("to-file")).expect("a link");

    for name in ["pipe", "zero", "to-file"] {
        assert_eq!(seen(&folder.path().join(name), 10), Seen::Odd, "{name}");
        assert_eq!(read(&folder.path().join(name), 10), Seen::Odd, "{name}");
    }
    assert_eq!(read(Path::new("/dev/zero"), 10), Seen::Odd);
    assert_eq!(read_below(folder.path(), "via/file", 10), Seen::Odd);
}

/// A file of procfs says it is regular and has the size 0, and what it reads has no end: the size of `lstat` is no size.
#[cfg(target_os = "linux")]
#[test]
fn a_file_of_a_pseudo_file_system_is_odd_whatever_size_it_says() {
    for path in [
        "/proc/self/pagemap",
        "/proc/self/status",
        "/proc/self/maps",
        "/proc/cpuinfo",
    ] {
        let metadata = fs::symlink_metadata(path).expect("there");
        assert!(
            metadata.file_type().is_file() && metadata.len() == 0,
            "{path}: the size is told as 0"
        );

        assert_eq!(seen(Path::new(path), 1 << 20), Seen::Odd, "{path}");
        assert_eq!(read(Path::new(path), 1 << 20), Seen::Odd, "{path}");
        assert_eq!(rules_file(Path::new(path), 1 << 20), Rules::Odd, "{path}");
    }
}

#[test]
fn a_real_folder_is_plain_and_a_file_or_nothing_is_missing() {
    let folder = tempfile::tempdir().expect("a folder");
    fs::create_dir(folder.path().join("sub")).expect("a folder");
    fs::write(folder.path().join("file"), b"x").expect("a file");

    assert_eq!(folder_of(&folder.path().join("sub")), Seen::Plain(()));
    assert_eq!(folder_of(&folder.path().join("file")), Seen::Missing);
    assert_eq!(folder_of(&folder.path().join("none")), Seen::Missing);
    assert_eq!(folder_of(&folder.path().join("file/deeper")), Seen::Missing);
}

fn folder_of(path: &Path) -> Seen<()> {
    folder(path)
}

#[cfg(unix)]
#[test]
fn a_link_for_a_folder_is_odd() {
    let folder = tempfile::tempdir().expect("a folder");
    let root = gix::path::realpath(folder.path()).expect("its real path");
    fs::create_dir(root.join("sub")).expect("a folder");
    std::os::unix::fs::symlink(root.join("sub"), root.join("to-folder")).expect("a link");
    std::os::unix::fs::symlink(root.join("gone"), root.join("dangling")).expect("a link");

    assert_eq!(folder_of(&root.join("to-folder")), Seen::Odd);
    assert_eq!(folder_of(&root.join("dangling")), Seen::Odd);
}

#[test]
fn a_rules_file_is_its_size_unless_it_is_a_pipe_a_device_or_too_big() {
    let folder = tempfile::tempdir().expect("a folder");
    fs::write(folder.path().join("small"), b"*.log\n").expect("rules");
    fs::write(folder.path().join("big"), vec![b'#'; RULES_CAP as usize + 1]).expect("rules");
    fs::create_dir(folder.path().join("folder")).expect("a folder");

    assert_eq!(rules_file(&folder.path().join("small"), RULES_CAP), Rules::Bytes(6));
    assert_eq!(rules_file(&folder.path().join("none"), RULES_CAP), Rules::Bytes(0));
    assert_eq!(rules_file(&folder.path().join("folder"), RULES_CAP), Rules::Bytes(0));
    assert_eq!(rules_file(&folder.path().join("big"), RULES_CAP), Rules::Large);
    #[cfg(unix)]
    {
        crate::view::testing::fifo(folder.path(), "pipe");
        std::os::unix::fs::symlink(folder.path().join("small"), folder.path().join("link")).expect("a link");

        assert_eq!(rules_file(&folder.path().join("pipe"), RULES_CAP), Rules::Odd);
        // A link is not followed by what reads rules, so it is no rules.
        assert_eq!(rules_file(&folder.path().join("link"), RULES_CAP), Rules::Bytes(0));
    }
}
