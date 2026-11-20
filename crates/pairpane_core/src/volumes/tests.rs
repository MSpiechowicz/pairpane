use std::fs;
use std::path::Path;

use pretty_assertions::assert_eq;

use super::*;
use crate::model::VolumeKind::{Drive, Home, Network, Removable, System};

fn found(name: &str, kind: VolumeKind, path: &str) -> Found {
    Found {
        volume: Volume {
            name: name.to_owned(),
            label: None,
            path: path.to_owned(),
            kind,
            free: None,
        },
        measure: true,
    }
}

/// A temporary folder and its path with every link resolved (on macOS the temporary folder is under the link
/// `/var`), as [`measure`] only asks a path that holds none.
fn link_free_temp() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = fs::canonicalize(dir.path()).expect("canonical");

    (dir, path.to_string_lossy().into_owned())
}

fn root(name: &str) -> String {
    let root = if cfg!(windows) { "C:\\" } else { "/" };

    Path::new(root).join(name).to_string_lossy().into_owned()
}

#[test]
fn order_puts_the_kinds_in_menu_order_and_keeps_the_order_within_a_kind() {
    let list = vec![
        found("net", Network, "/n"),
        found("b", Drive, "/b"),
        found("usb", Removable, "/u"),
        found("home", Home, "/h"),
        found("a", Drive, "/a"),
        found("sys", System, "/"),
    ];

    let names: Vec<_> = order(list).into_iter().map(|found| found.volume.name).collect();

    assert_eq!(names, ["sys", "home", "b", "a", "usb", "net"]);
}

#[test]
fn order_drops_a_second_volume_at_the_same_path_and_limits_the_list() {
    let list = vec![found("mount", Drive, "/home/me"), found("Home", Home, "/home/me")];
    let ordered = order(list);
    assert_eq!(ordered.len(), 1);
    assert_eq!(
        ordered[0].volume.kind, Home,
        "the home folder wins over the mount at the same place"
    );

    let many = (0..MAX_VOLUMES + 20)
        .map(|n| found("d", Drive, &format!("/mnt/{n}")))
        .collect();
    assert_eq!(order(many).len(), MAX_VOLUMES);
}

#[test]
fn list_offers_the_home_folder_and_only_paths_a_pane_may_open() {
    let dir = tempfile::tempdir().expect("temp dir");

    let found = list(Some(dir.path()));

    let home = found.iter().find(|found| found.volume.kind == Home).expect("home");
    assert_eq!(home.volume.name, "Home");
    assert_eq!(Path::new(&home.volume.path), dir.path());
    assert!(
        found
            .iter()
            .all(|found| check_folder(Path::new(&found.volume.path)).is_ok())
    );
    assert_eq!(found.iter().filter(|found| found.volume.kind == System).count(), 1);
}

#[test]
fn list_without_a_home_folder_has_no_home_entry() {
    assert!(list(None).iter().all(|found| found.volume.kind != Home));
}

#[test]
fn a_home_folder_that_a_pane_could_not_open_is_not_offered() {
    let found = list(Some(Path::new("relative/home")));

    assert!(found.iter().all(|found| found.volume.kind != Home));
}

#[test]
fn measure_asks_the_volumes_that_may_be_asked_in_list_order() {
    let (_dir, path) = link_free_temp();
    let list = vec![found("a", Drive, &path), found("b", Removable, &path)];
    let mut asked = Vec::new();

    measure(&list, |at, bytes| {
        asked.push((at.to_owned(), bytes));
        true
    });

    assert_eq!(asked.len(), 2);
    assert!(asked.iter().all(|(at, bytes)| *at == path && *bytes > 0));
}

#[test]
fn measure_never_asks_a_network_volume_a_volume_marked_not_to_be_asked_or_one_that_has_its_free_space() {
    let (_dir, path) = link_free_temp();
    let mut network = found("net", Network, &path);
    let mut marked = found("marked", Drive, &path);
    let mut known = found("known", Drive, &path);
    // Even a network volume wrongly marked as askable is left alone.
    network.measure = true;
    marked.measure = false;
    known.volume.free = Some(5);

    measure(&[network, marked, known], |_, _| panic!("nothing may be asked"));
}

#[cfg(unix)]
#[test]
fn measure_skips_a_path_that_goes_through_a_link_and_asks_the_same_folder_without_one() {
    let (dir, real) = link_free_temp();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    let through_link = format!("{}/link", dir.path().to_string_lossy());
    let list = vec![
        found("home", Home, &through_link),
        found("with_slash", Home, &format!("{through_link}/")),
        found("real", Home, &real),
    ];
    let mut asked = Vec::new();

    measure(&list, |at, _| {
        asked.push(at.to_owned());
        true
    });

    assert_eq!(asked, [real], "only the path without a link");
}

#[cfg(unix)]
#[test]
fn measure_skips_a_path_with_a_link_in_the_middle() {
    let (dir, real) = link_free_temp();
    fs::create_dir(Path::new(&real).join("sub")).expect("sub");
    std::os::unix::fs::symlink(&real, dir.path().join("link")).expect("symlink");
    let list = vec![found(
        "home",
        Home,
        &format!("{}/link/sub", dir.path().to_string_lossy()),
    )];

    measure(&list, |_, _| panic!("a path through a link must not be asked"));
}

#[test]
fn measure_skips_a_path_that_cannot_be_resolved() {
    let list = vec![found("gone", Home, &root("pairpane-no-such-home"))];

    measure(&list, |_, _| panic!("nothing may be asked"));
}

#[test]
fn measure_skips_a_volume_that_cannot_be_read_and_stops_when_asked_to() {
    let (_dir, path) = link_free_temp();
    let list = vec![
        found("gone", Drive, &root("pairpane-no-such-volume")),
        found("a", Drive, &path),
        found("b", Drive, &path),
    ];
    let mut calls = 0;

    measure(&list, |_, _| {
        calls += 1;
        false
    });

    assert_eq!(
        calls, 1,
        "the unreadable volume is skipped and the first answer ends it"
    );
}

#[test]
fn the_mac_list_has_the_system_disk_and_the_folders_of_volumes_that_are_for_people() {
    let entries: Vec<(String, bool)> = [
        ("Macintosh HD", false),
        ("USB", true),
        (".timemachine", true),
        ("com.apple.TimeMachine.localsnapshots", true),
        ("Backup", true),
        ("notes.txt", false),
        ("", true),
    ]
    .into_iter()
    .map(|(name, folder)| (name.to_owned(), folder))
    .collect();

    let list = macos::volumes(&entries);

    let shown: Vec<_> = list
        .iter()
        .map(|found| {
            (
                found.volume.name.as_str(),
                found.volume.kind,
                found.volume.path.as_str(),
            )
        })
        .collect();
    assert_eq!(
        shown,
        vec![
            ("System", System, "/"),
            ("Backup", Drive, "/Volumes/Backup"),
            ("USB", Drive, "/Volumes/USB"),
        ]
    );
    let measured: Vec<_> = list.iter().map(|found| found.measure).collect();
    assert_eq!(
        measured,
        [true, false, false],
        "only the system disk: a volume in /Volumes may be a network share"
    );
}

#[test]
fn mac_names_are_filtered_by_their_prefix() {
    assert!(macos::is_listed("USB"));
    assert!(macos::is_listed("com.example.app"));
    assert!(macos::is_listed("My .Drive"));
    assert!(!macos::is_listed(".hidden"));
    assert!(!macos::is_listed("com.apple.TimeMachine.localsnapshots"));
    assert!(!macos::is_listed(""));
}

#[test]
fn a_windows_drive_is_named_by_its_letter_and_its_label_goes_to_the_second_line() {
    let drive = windows::volume("C:\\", "Windows", false, 123, Some("C:"));

    assert_eq!(drive.volume.name, "C:");
    assert_eq!(drive.volume.label.as_deref(), Some("Windows"));
    assert_eq!(drive.volume.path, "C:\\");
    assert_eq!(drive.volume.kind, System);
    assert_eq!(drive.volume.free, Some(123), "the OS list brings the free space");
    assert!(!drive.measure, "so it is not asked again");
}

#[test]
fn a_windows_volume_is_removable_a_drive_or_the_system_disk_and_an_empty_label_is_none() {
    let usb = windows::volume("E:\\", "", true, 1, Some("C:"));
    assert_eq!((usb.volume.kind, usb.volume.label), (Removable, None));

    let data = windows::volume("D:\\", "  ", false, 1, Some("C:"));
    assert_eq!((data.volume.kind, data.volume.label), (Drive, None));

    let lower = windows::volume("c:\\", "x", false, 1, Some("C:"));
    assert_eq!(lower.volume.kind, System, "drive letters ignore case");

    let unknown = windows::volume("C:\\", "x", false, 1, None);
    assert_eq!(unknown.volume.kind, Drive);
}

#[test]
fn a_windows_volume_mounted_in_a_folder_is_named_by_the_folder() {
    let mounted = windows::volume("C:\\Mounts\\Data\\", "Data", false, 1, Some("C:"));

    assert_eq!(mounted.volume.name, "Data");
    assert_eq!(mounted.volume.path, "C:\\Mounts\\Data\\");
    assert_eq!(mounted.volume.kind, Drive);
}
