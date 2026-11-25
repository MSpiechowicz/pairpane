//! The device list: what it keeps, what it refuses to read and how it is written.

use std::fs;
use std::path::Path;

use iroh::{EndpointId, SecretKey};
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::{DEVICES_FILE_NAME, DeviceStore, MAX_ADDRS, MAX_DEVICES, MAX_NAME_CHARS, validate_name};
use crate::error::Error;

fn id() -> EndpointId {
    SecretKey::generate().public()
}

fn open(dir: &TempDir) -> DeviceStore {
    DeviceStore::open(dir.path()).unwrap()
}

fn file(dir: &Path) -> std::path::PathBuf {
    dir.join(DEVICES_FILE_NAME)
}

#[test]
fn a_missing_file_is_an_empty_list() {
    let dir = TempDir::new().unwrap();

    assert!(open(&dir).list().is_empty());
    assert!(!file(dir.path()).exists());
}

#[test]
fn devices_survive_a_restart_as_they_were_written() {
    let dir = TempDir::new().unwrap();
    let (first, second) = (id(), id());

    let mut store = open(&dir);
    store.add(first, "Laptop", 1_700_000_000_000).unwrap();
    store.add(second, "  Desktop ", 1_700_000_000_500).unwrap();

    let reopened = open(&dir);
    assert_eq!(reopened.list(), store.list());
    assert_eq!(reopened.list()[0].name, "Laptop");
    assert_eq!(reopened.list()[1].name, "Desktop");
    assert_eq!(reopened.list()[1].paired_at, 1_700_000_000_500);
    assert!(reopened.is_paired(&first) && reopened.is_paired(&second));
    assert!(!reopened.is_paired(&id()));
}

#[test]
fn the_file_holds_the_node_id_as_hex_text() {
    let dir = TempDir::new().unwrap();
    let node_id = id();
    open(&dir).add(node_id, "Laptop", 5).unwrap();

    let text = fs::read_to_string(file(dir.path())).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();

    assert_eq!(json["devices"][0]["node_id"], node_id.to_string());
    assert_eq!(json["devices"][0]["name"], "Laptop");
    assert_eq!(json["devices"][0]["paired_at"], 5);
}

#[test]
fn adding_a_paired_device_again_replaces_it_in_place() {
    let dir = TempDir::new().unwrap();
    let (first, second) = (id(), id());
    let mut store = open(&dir);
    store.add(first, "Old name", 1).unwrap();
    store.add(second, "Other", 2).unwrap();

    store.add(first, "New name", 3).unwrap();

    assert_eq!(store.list().len(), 2);
    assert_eq!(store.list()[0].node_id, first);
    assert_eq!(store.list()[0].name, "New name");
    assert_eq!(store.list()[0].paired_at, 3);
}

#[test]
fn rename_and_remove_change_the_file() {
    let dir = TempDir::new().unwrap();
    let (first, second) = (id(), id());
    let mut store = open(&dir);
    store.add(first, "One", 1).unwrap();
    store.add(second, "Two", 2).unwrap();

    store.rename(&second, "Second").unwrap();
    store.remove(&first).unwrap();

    let reopened = open(&dir);
    assert_eq!(reopened.list().len(), 1);
    assert_eq!(reopened.list()[0].name, "Second");
    assert!(matches!(store.rename(&first, "Gone"), Err(Error::UnknownDevice)));
    assert!(matches!(store.remove(&first), Err(Error::UnknownDevice)));
}

#[test]
fn names_are_trimmed_and_limited() {
    assert_eq!(validate_name("  Laptop of Ann ").unwrap(), "Laptop of Ann");
    assert_eq!(
        validate_name(&"é".repeat(MAX_NAME_CHARS)).unwrap().chars().count(),
        MAX_NAME_CHARS
    );

    for bad in [
        "",
        "   ",
        "tab\there",
        "new\nline",
        "bell\u{7}",
        &"x".repeat(MAX_NAME_CHARS + 1),
    ] {
        assert!(matches!(validate_name(bad), Err(Error::InvalidName)), "{bad:?}");
    }
}

#[test]
fn a_bad_name_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let node_id = id();
    let mut store = open(&dir);

    assert!(matches!(store.add(node_id, "", 1), Err(Error::InvalidName)));
    store.add(node_id, "Fine", 1).unwrap();
    assert!(matches!(store.rename(&node_id, "\n"), Err(Error::InvalidName)));

    assert_eq!(open(&dir).list()[0].name, "Fine");
}

#[test]
fn the_list_has_a_limit() {
    let dir = TempDir::new().unwrap();
    let mut store = open(&dir);
    for index in 0..MAX_DEVICES {
        store.add(id(), &format!("Device {index}"), 1).unwrap();
    }

    assert!(matches!(store.add(id(), "One too many", 1), Err(Error::TooManyDevices)));
    assert_eq!(open(&dir).list().len(), MAX_DEVICES);
}

#[test]
fn a_garbage_file_is_refused() {
    for content in [&b"not json"[..], b"", b"[]", b"{\"version\":1}", b"\xff\xfe\x00"] {
        let dir = TempDir::new().unwrap();
        fs::write(file(dir.path()), content).unwrap();

        assert!(
            matches!(DeviceStore::open(dir.path()), Err(Error::CorruptDevices)),
            "{content:?}"
        );
    }
}

#[test]
fn an_oversized_file_is_refused() {
    let dir = TempDir::new().unwrap();
    let mut text = String::from("{\"version\":1,\"devices\":[],\"padding\":\"");
    text.push_str(&"a".repeat(2 * 1024 * 1024));
    text.push_str("\"}");
    fs::write(file(dir.path()), text).unwrap();

    assert!(matches!(DeviceStore::open(dir.path()), Err(Error::CorruptDevices)));
}

#[test]
fn unknown_fields_a_wrong_version_and_bad_records_are_refused() {
    let node_id = id();
    let record =
        |extra: &str, name: &str| format!("{{\"node_id\":\"{node_id}\",\"name\":\"{name}\",\"paired_at\":1{extra}}}");
    let cases = [
        format!("{{\"version\":1,\"devices\":[{}]}}", record(",\"extra\":true", "A")),
        format!("{{\"version\":2,\"devices\":[{}]}}", record("", "A")),
        format!("{{\"version\":1,\"devices\":[{}],\"extra\":1}}", record("", "A")),
        format!("{{\"version\":1,\"devices\":[{}]}}", record("", " padded ")),
        format!("{{\"version\":1,\"devices\":[{}]}}", record("", "")),
        format!("{{\"version\":1,\"devices\":[{0},{0}]}}", record("", "A")),
        "{\"version\":1,\"devices\":[{\"node_id\":\"abcd\",\"name\":\"A\",\"paired_at\":1}]}".to_owned(),
        format!(
            "{{\"version\":1,\"devices\":[{}]}}",
            record("", "A").replace("\"paired_at\":1", "\"paired_at\":-1")
        ),
    ];

    for text in cases {
        let dir = TempDir::new().unwrap();
        fs::write(file(dir.path()), &text).unwrap();

        assert!(
            matches!(DeviceStore::open(dir.path()), Err(Error::CorruptDevices)),
            "{text}"
        );
    }
}

#[test]
fn a_write_leaves_no_temporary_file_and_replaces_an_older_one() {
    let dir = TempDir::new().unwrap();
    let mut store = open(&dir);
    store.add(id(), "One", 1).unwrap();
    // What a crash in the middle of an earlier write leaves behind.
    fs::write(dir.path().join(format!(".{DEVICES_FILE_NAME}.tmp")), b"half a wri").unwrap();

    store.add(id(), "Two", 2).unwrap();

    let names: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![DEVICES_FILE_NAME.to_owned()]);
    assert_eq!(open(&dir).list().len(), 2);
}

#[test]
fn a_failed_write_changes_neither_the_file_nor_the_list() {
    let dir = TempDir::new().unwrap();
    let mut store = open(&dir);
    store.add(id(), "One", 1).unwrap();
    let before = fs::read(file(dir.path())).unwrap();
    // A rename of a file over a folder fails, so the write cannot be finished.
    fs::remove_file(file(dir.path())).unwrap();
    fs::create_dir(file(dir.path())).unwrap();

    assert!(matches!(store.add(id(), "Two", 2), Err(Error::Io(_))));

    assert_eq!(store.list().len(), 1);
    assert!(!dir.path().join(format!(".{DEVICES_FILE_NAME}.tmp")).exists());
    assert!(!before.is_empty());
}

#[cfg(unix)]
#[test]
fn the_file_and_the_folder_are_private() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let folder = dir.path().join("link");
    let mut store = DeviceStore::open(&folder).unwrap();
    store.add(id(), "One", 1).unwrap();

    assert_eq!(
        fs::metadata(folder.join(DEVICES_FILE_NAME))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(fs::metadata(&folder).unwrap().permissions().mode() & 0o777, 0o700);
}

fn addr(last: u8) -> std::net::SocketAddr {
    std::net::SocketAddr::from(([100, 64, 0, last], 47620))
}

#[test]
fn addresses_are_kept_newest_first_without_repeats_and_survive_a_restart() {
    let dir = TempDir::new().unwrap();
    let node_id = id();
    let mut store = open(&dir);
    store
        .add_with_addrs(node_id, "Laptop", 1, &[addr(1), addr(2), addr(1)])
        .unwrap();

    assert!(store.remember_addrs(&node_id, &[addr(3)]).unwrap());
    assert!(store.remember_addrs(&node_id, &[addr(2), addr(4)]).unwrap());

    let expected = vec![addr(2), addr(4), addr(3), addr(1)];
    assert_eq!(store.get(&node_id).unwrap().addrs, expected);
    assert_eq!(open(&dir).get(&node_id).unwrap().addrs, expected);
}

#[test]
fn only_the_newest_eight_addresses_are_kept() {
    let dir = TempDir::new().unwrap();
    let node_id = id();
    let mut store = open(&dir);
    store.add(node_id, "Laptop", 1).unwrap();

    for last in 1..=12 {
        store.remember_addrs(&node_id, &[addr(last)]).unwrap();
    }
    store
        .remember_addrs(&node_id, &(20..40).map(addr).collect::<Vec<_>>())
        .unwrap();

    let kept = open(&dir).get(&node_id).unwrap().addrs.clone();
    assert_eq!(kept.len(), MAX_ADDRS);
    assert_eq!(kept[0], addr(20));
    assert_eq!(kept[MAX_ADDRS - 1], addr(27));
}

#[test]
fn nothing_is_written_when_the_addresses_are_the_newest_already() {
    let dir = TempDir::new().unwrap();
    let node_id = id();
    let mut store = open(&dir);
    store.add_with_addrs(node_id, "Laptop", 1, &[addr(1), addr(2)]).unwrap();
    let written = fs::metadata(file(dir.path())).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));

    assert!(!store.remember_addrs(&node_id, &[addr(2), addr(1)]).unwrap());
    assert!(!store.remember_addrs(&node_id, &[addr(1)]).unwrap());
    assert!(!store.remember_addrs(&node_id, &[]).unwrap());
    assert!(
        !store.remember_addrs(&id(), &[addr(9)]).unwrap(),
        "a device that is not paired is ignored"
    );

    assert_eq!(fs::metadata(file(dir.path())).unwrap().modified().unwrap(), written);
}

#[test]
fn pairing_a_device_again_keeps_the_addresses_it_had() {
    let dir = TempDir::new().unwrap();
    let node_id = id();
    let mut store = open(&dir);
    store.add_with_addrs(node_id, "Laptop", 1, &[addr(1)]).unwrap();

    store.add(node_id, "Laptop", 2).unwrap();
    assert_eq!(store.get(&node_id).unwrap().addrs, vec![addr(1)]);

    store.add_with_addrs(node_id, "Laptop", 3, &[addr(2)]).unwrap();
    assert_eq!(store.get(&node_id).unwrap().addrs, vec![addr(2), addr(1)]);
}

#[test]
fn a_file_from_before_addresses_were_kept_still_loads() {
    let dir = TempDir::new().unwrap();
    let node_id = id();
    let text =
        format!("{{\"version\":1,\"devices\":[{{\"node_id\":\"{node_id}\",\"name\":\"Old\",\"paired_at\":7}}]}}");
    fs::write(file(dir.path()), text).unwrap();

    let mut store = open(&dir);

    assert_eq!(store.get(&node_id).unwrap().name, "Old");
    assert!(store.get(&node_id).unwrap().addrs.is_empty());
    assert!(store.remember_addrs(&node_id, &[addr(1)]).unwrap());
}

#[test]
fn a_file_with_too_many_or_repeated_addresses_keeps_the_first_eight() {
    let dir = TempDir::new().unwrap();
    let node_id = id();
    let addrs: Vec<String> = (1..=12)
        .chain([1, 2])
        .map(|last| format!("\"{}\"", addr(last)))
        .collect();
    let text = format!(
        "{{\"version\":1,\"devices\":[{{\"node_id\":\"{node_id}\",\"name\":\"A\",\"paired_at\":1,\"addrs\":[{}]}}]}}",
        addrs.join(",")
    );
    fs::write(file(dir.path()), text).unwrap();

    let store = open(&dir);

    assert_eq!(
        store.get(&node_id).unwrap().addrs,
        (1..=8).map(addr).collect::<Vec<_>>()
    );
}
