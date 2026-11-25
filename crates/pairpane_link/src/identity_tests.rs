//! The device key stores. None of these touches the OS keychain.

use pretty_assertions::assert_eq;

use super::{KEY_FILE_NAME, KeyStore, MemoryStore, StoreKind, load_or_create};

#[test]
fn a_key_is_made_once_and_kept() {
    let store = MemoryStore::new();
    assert!(store.load().unwrap().is_none());

    let first = load_or_create(&store).unwrap();
    let second = load_or_create(&store).unwrap();

    assert_eq!(first.to_bytes(), second.to_bytes());
    assert_eq!(store.kind(), StoreKind::Memory);
}

#[test]
fn two_stores_make_two_identities() {
    let one = load_or_create(&MemoryStore::new()).unwrap();
    let other = load_or_create(&MemoryStore::new()).unwrap();

    assert!(one.public() != other.public());
}

#[cfg(unix)]
mod file {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::super::FileStore;
    use super::*;
    use crate::error::Error;

    fn mode(path: &std::path::Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn the_key_file_is_private_and_the_key_comes_back() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(&dir.path().join("link"));

        let key = load_or_create(&store).unwrap();

        assert_eq!(store.kind(), StoreKind::File);
        assert_eq!(mode(&dir.path().join("link").join(KEY_FILE_NAME)), 0o600);
        assert_eq!(mode(&dir.path().join("link")), 0o700);
        assert_eq!(
            FileStore::new(&dir.path().join("link"))
                .load()
                .unwrap()
                .unwrap()
                .to_bytes(),
            key.to_bytes()
        );
    }

    #[test]
    fn a_file_that_group_or_other_can_access_is_refused() {
        for insecure in [0o644, 0o640, 0o604, 0o660, 0o666, 0o700 | 0o040] {
            let dir = TempDir::new().unwrap();
            let store = FileStore::new(dir.path());
            load_or_create(&store).unwrap();
            let path = dir.path().join(KEY_FILE_NAME);

            fs::set_permissions(&path, fs::Permissions::from_mode(insecure)).unwrap();

            assert!(matches!(store.load(), Err(Error::InsecureKeyFile)), "mode {insecure:o}");
            assert!(
                matches!(load_or_create(&store), Err(Error::InsecureKeyFile)),
                "mode {insecure:o}"
            );
        }
    }

    #[test]
    fn an_owner_only_mode_is_accepted() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path());
        load_or_create(&store).unwrap();

        for owner_only in [0o600, 0o400] {
            fs::set_permissions(dir.path().join(KEY_FILE_NAME), fs::Permissions::from_mode(owner_only)).unwrap();

            assert!(store.load().unwrap().is_some(), "mode {owner_only:o}");
        }
    }

    #[test]
    fn a_link_in_place_of_the_file_is_refused() {
        let dir = TempDir::new().unwrap();
        let elsewhere = dir.path().join("elsewhere");
        fs::write(&elsewhere, [7u8; 32]).unwrap();
        fs::set_permissions(&elsewhere, fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.path().join(KEY_FILE_NAME)).unwrap();

        assert!(matches!(FileStore::new(dir.path()).load(), Err(Error::InsecureKeyFile)));
    }

    #[test]
    fn something_that_is_not_a_regular_file_is_refused_when_it_is_open() {
        // What a name swapped for another kind of file after the first check would be: the open descriptor is judged.
        let dir = TempDir::new().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();

        let result = super::super::read_key_file(fs::File::open(dir.path()).unwrap());

        assert!(matches!(result, Err(Error::InsecureKeyFile)), "{result:?}");
    }

    #[test]
    fn a_file_of_the_wrong_size_is_refused() {
        for bytes in [&[1u8; 31][..], &[1u8; 33], &[]] {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join(KEY_FILE_NAME);
            fs::write(&path, bytes).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

            assert!(
                matches!(FileStore::new(dir.path()).load(), Err(Error::CorruptKey)),
                "{} bytes",
                bytes.len()
            );
        }
    }

    #[test]
    fn saving_again_replaces_the_key_whole_and_leaves_no_temporary_file() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path());
        let first = load_or_create(&store).unwrap();
        // What a crash in the middle of an earlier write leaves behind.
        fs::write(dir.path().join(format!(".{KEY_FILE_NAME}.tmp")), b"half").unwrap();
        let second = iroh::SecretKey::generate();

        store.save(&second).unwrap();

        assert!(first.public() != second.public());
        assert_eq!(store.load().unwrap().unwrap().to_bytes(), second.to_bytes());
        assert_eq!(mode(&dir.path().join(KEY_FILE_NAME)), 0o600);
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from(KEY_FILE_NAME)]);
    }

    #[test]
    fn a_missing_file_is_no_key() {
        let dir = TempDir::new().unwrap();

        assert!(FileStore::new(dir.path()).load().unwrap().is_none());
    }
}
