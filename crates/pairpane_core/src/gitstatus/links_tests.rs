use std::os::unix::fs::symlink;

use super::*;

#[test]
fn dots_are_taken_out_by_reading_the_path() {
    for (text, flat) in [
        ("/a/b/../c", "/a/c"),
        ("/a/./b", "/a/b"),
        ("/../a", "/a"),
        ("/a/../..", "/"),
        ("a/../../b", "../b"),
        ("/net/../srv", "/srv"),
    ] {
        assert_eq!(normalise(Path::new(text)), PathBuf::from(flat), "{text}");
    }
}

#[test]
fn an_automounted_folder_is_found_whatever_dots_lead_to_it() {
    for text in [
        "/net/host/x",
        "/srv/../net/host",
        "/srv/x/../../Network/Servers",
        "/./net",
        "/System/Volumes/Data/net/host",
        "/home/../System/Volumes/Data/Network/x",
    ] {
        assert!(is_automount(Path::new(text)), "{text}");
        assert!(!is_local(Path::new(text)), "{text}");
    }
    for text in ["/srv/net/x", "/net2"] {
        assert!(!is_automount(Path::new(text)), "{text}");
    }
    // On macOS names are compared without regard to case, so `/network` is `/Network`.
    assert_eq!(is_automount(Path::new("/network")), cfg!(target_os = "macos"));
    // `/home` is an automount point on macOS (autofs), and an ordinary folder elsewhere.
    for text in ["/home/x", "/srv/../home", "/System/Volumes/Data/home"] {
        assert_eq!(is_automount(Path::new(text)), cfg!(target_os = "macos"), "{text}");
    }
}

#[test]
fn a_link_may_be_followed_unless_a_dots_comes_straight_after_it() {
    let folder = crate::gitstatus::testing::tempdir();
    let root = gix::path::realpath(folder.path()).expect("its real path");
    fs::create_dir_all(root.join("real/inner")).expect("folders");
    symlink("real", root.join("via")).expect("a link");
    symlink("inner", root.join("real/to-inner")).expect("a link");

    for (path, physical, followable) in [
        ("real/inner", true, true),
        ("via/inner", false, true),
        // A real folder of what the link leads to, and back out of it.
        ("via/inner/..", false, true),
        ("real/inner/../x", true, true),
        // Straight after a link: the system goes up from the link's target, gix from the link.
        ("via/..", false, false),
        ("via/../real", false, false),
        ("real/to-inner/..", false, false),
        // Not there, then a dots.
        ("via/none/..", false, false),
    ] {
        let path = root.join(path);

        assert_eq!(is_physical(&path), physical, "{path:?}");
        assert_eq!(is_followable(&path), followable, "{path:?}");
    }
    assert!(!is_followable(Path::new("relative/path")));
}

#[test]
fn a_hop_to_an_automounted_folder_is_found_even_when_the_dots_after_it_lead_back() {
    // Read lexically `/net/../srv` is `/srv`, but the system goes through `/net` to get to the root again.
    assert!(!is_local(Path::new("/net/../srv")));
}

#[test]
fn a_link_that_leads_to_an_automounted_folder_is_refused_at_any_hop() {
    let folder = crate::gitstatus::testing::tempdir();
    let root = gix::path::realpath(folder.path()).expect("its real path");
    symlink("/net/host/export", root.join("direct")).expect("a link");
    symlink(root.join("direct"), root.join("second")).expect("a link");
    symlink("second", root.join("third")).expect("a link");
    fs::create_dir(root.join("real")).expect("a folder");
    symlink("/System/Volumes/Data/Network", root.join("real/inside")).expect("a link");

    for name in ["direct", "second", "third", "third/deeper", "real/inside/x"] {
        assert!(!is_local(&root.join(name)), "{name}");
        assert_eq!(
            resolve(&root.join(name)).expect_err(name).kind(),
            io::ErrorKind::InvalidInput,
            "{name}"
        );
    }
}

#[test]
fn a_link_to_a_local_place_is_followed_to_where_it_leads() {
    let folder = crate::gitstatus::testing::tempdir();
    let root = gix::path::realpath(folder.path()).expect("its real path");
    fs::create_dir_all(root.join("a/b")).expect("folders");
    fs::write(root.join("a/b/file"), b"x").expect("a file");
    symlink("a/b", root.join("one")).expect("a link");
    symlink(root.join("one"), root.join("two")).expect("a link");
    symlink("..", root.join("a/b/up")).expect("a link");

    assert_eq!(
        resolve(&root.join("two/file")).expect("resolved"),
        root.join("a/b/file")
    );
    // `..` goes up from where the link leads: `one/..` is `a`, and not the folder that holds `one`.
    assert_eq!(resolve(&root.join("one/..")).expect("resolved"), root.join("a"));
    assert_eq!(resolve(&root.join("a/b/up/b")).expect("resolved"), root.join("a/b"));
    assert!(is_local(&root.join("two")));
}

#[test]
fn a_path_that_leads_nowhere_is_local_and_a_loop_of_links_is_not() {
    let folder = crate::gitstatus::testing::tempdir();
    let root = gix::path::realpath(folder.path()).expect("its real path");
    fs::write(root.join("file"), b"x").expect("a file");
    symlink("loop-b", root.join("loop-a")).expect("a link");
    symlink("loop-a", root.join("loop-b")).expect("a link");
    symlink("gone", root.join("dangling")).expect("a link");

    assert_eq!(
        resolve(&root.join("none/deeper")).expect_err("missing").kind(),
        io::ErrorKind::NotFound
    );
    assert!(is_local(&root.join("none/deeper")));
    assert!(
        is_local(&root.join("file/deeper")),
        "a file is no folder: nothing is there"
    );
    assert!(is_local(&root.join("dangling")));
    assert!(!is_local(&root.join("loop-a")));
    assert!(
        !is_local(Path::new("relative/path")),
        "a path that is not absolute cannot be checked"
    );
}

#[test]
fn a_path_is_physical_when_no_link_is_on_it_and_dots_follow_real_folders_only() {
    let folder = crate::gitstatus::testing::tempdir();
    let root = gix::path::realpath(folder.path()).expect("its real path");
    fs::create_dir_all(root.join("a/b")).expect("folders");
    fs::write(root.join("a/file"), b"x").expect("a file");
    symlink("a/b", root.join("link")).expect("a link");

    for physical in ["a/b", "a/b/../b", "a/./b", "a/none", "none/deeper", "a/file", "a/b/.."] {
        assert!(is_physical(&root.join(physical)), "{physical}");
    }
    for odd in [
        "link",
        "link/..",
        "link/../a",
        "a/link2/../b",
        "none/../a",
        "a/file/../b",
        "a/none/../b",
    ] {
        assert!(!is_physical(&root.join(odd)), "{odd}");
    }
    assert!(!is_physical(Path::new("relative")));
    assert!(is_physical(Path::new("/")));
}

#[test]
fn a_path_whose_dots_the_link_would_read_differently_is_not_physical_and_resolves_elsewhere() {
    // The auditor's `lexical` fixture: gix reads `l/../real` as `w/real`, the system as `x/real`.
    let folder = crate::gitstatus::testing::tempdir();
    let root = gix::path::realpath(folder.path()).expect("its real path");
    fs::create_dir_all(root.join("x/y")).expect("folders");
    fs::create_dir(root.join("x/real")).expect("a folder");
    fs::create_dir(root.join("real")).expect("a folder");
    symlink("x/y", root.join("l")).expect("a link");
    let path = root.join("l/../real");

    assert!(!is_physical(&path));
    assert_eq!(resolve(&path).expect("resolved"), root.join("x/real"));
    assert_ne!(resolve(&path).expect("resolved"), normalise(&path));
}

#[cfg(target_os = "linux")]
#[test]
fn a_magic_link_that_is_no_path_is_refused_and_never_leads_nowhere() {
    use std::os::fd::AsRawFd;

    let folder = crate::gitstatus::testing::tempdir();
    let root = gix::path::realpath(folder.path()).expect("its real path");
    let (reader, writer) = std::io::pipe().expect("a pipe");
    let at = format!("/proc/self/fd/{}", reader.as_raw_fd());
    symlink(&at, root.join("to-pipe")).expect("a link");
    symlink(format!("/proc/self/fd/{}", writer.as_raw_fd()), root.join("to-writer")).expect("a link");

    for name in ["to-pipe", "to-writer"] {
        let error = resolve(&root.join(name)).expect_err(name);

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name}");
        assert!(!is_local(&root.join(name)), "{name}");
    }
    assert_eq!(
        resolve(Path::new(&at)).expect_err("the pipe itself").kind(),
        io::ErrorKind::InvalidInput
    );

    // A deleted file is the same: the link's text is a path that is not there, and the system opens the file.
    let gone = root.join("gone");
    let file = fs::File::create(&gone).expect("a file");
    fs::remove_file(&gone).expect("removed");
    let at = format!("/proc/self/fd/{}", file.as_raw_fd());

    assert_eq!(
        resolve(Path::new(&at)).expect_err("deleted").kind(),
        io::ErrorKind::InvalidInput
    );
}

/// What a walk asked and answered: the folders it was asked about, in order.
fn walked(path: &str, macos: bool, autofs: &[&Path]) -> (io::Result<PathBuf>, Vec<PathBuf>) {
    let mut asked = Vec::new();
    let answer = walk_with(Path::new(path), macos, &mut |folder| {
        asked.push(folder.to_path_buf());

        autofs.contains(&folder)
    });

    (answer, asked)
}

#[test]
fn a_name_that_is_an_automount_point_on_macos_is_refused_in_any_case_before_anything_is_looked_up() {
    for text in [
        "/Net/evilhost/export/repo.git",
        "/NET",
        "/Networ\u{212A}/Servers/x",
        "/system/volumes/data/net/x",
        "/HOME/x",
    ] {
        let (answer, asked) = walked(text, true, &[]);

        assert_eq!(answer.expect_err(text).kind(), io::ErrorKind::InvalidInput, "{text}");
        // Nothing at or below the refused name was looked up; only real folders above it were (`/`, `/System`...).
        assert!(
            !asked
                .iter()
                .any(|folder| crate::gitstatus::automount::is_automount_with(folder, true)),
            "{text}: {asked:?}"
        );
        assert!(!asked.iter().any(|folder| folder.ends_with("evilhost")), "{text}");
    }
}

#[test]
fn the_same_spellings_are_folders_like_any_other_where_names_are_case_sensitive() {
    for text in [
        "/Net/x",
        "/Networ\u{212A}/Servers",
        "/HOME/x",
        "/system/volumes/data/net/x",
    ] {
        let (answer, _) = walked(text, false, &[]);

        // Not refused: the folder is not there (or is one that nobody mounts a share into).
        assert_ne!(
            answer.as_ref().map_err(io::Error::kind).err(),
            Some(io::ErrorKind::InvalidInput),
            "{text}: {answer:?}"
        );
    }
}

#[test]
fn a_folder_that_the_file_system_says_is_an_automount_point_is_not_entered_whatever_its_name() {
    let folder = crate::gitstatus::testing::tempdir();
    let root = gix::path::realpath(folder.path()).expect("its real path");
    fs::create_dir_all(root.join("auto/host/export/repo.git")).expect("folders");
    let auto = root.join("auto");
    let deep = root.join("auto/host/export/repo.git");

    let (answer, asked) = walked(deep.to_str().expect("text"), false, &[auto.as_path()]);

    assert_eq!(answer.expect_err("refused").kind(), io::ErrorKind::InvalidInput);
    // The folders above the mount point were asked about, the mount point was, and nothing below it was.
    assert_eq!(asked.last(), Some(&auto));
    assert!(
        asked.iter().all(|asked| !asked.starts_with(root.join("auto/host"))),
        "{asked:?}"
    );

    // The same path with no mount point on it is resolved, and every real folder on it is asked about once.
    let (answer, asked) = walked(deep.to_str().expect("text"), false, &[]);

    assert_eq!(answer.expect("resolved"), deep);
    assert!(asked.contains(&root.join("auto/host/export")), "{asked:?}");
}

#[test]
fn a_link_to_an_automount_point_is_refused_when_its_target_is_entered() {
    let folder = crate::gitstatus::testing::tempdir();
    let root = gix::path::realpath(folder.path()).expect("its real path");
    fs::create_dir_all(root.join("auto/host")).expect("folders");
    symlink(root.join("auto"), root.join("via")).expect("a link");

    let (answer, asked) = walked(
        root.join("via/host").to_str().expect("text"),
        false,
        &[root.join("auto").as_path()],
    );

    assert_eq!(answer.expect_err("refused").kind(), io::ErrorKind::InvalidInput);
    assert!(!asked.iter().any(|asked| asked.ends_with("host")), "{asked:?}");
}

#[test]
fn an_ordinary_folder_is_resolved_with_the_systems_own_question() {
    let folder = crate::gitstatus::testing::tempdir();
    let root = gix::path::realpath(folder.path()).expect("its real path");
    fs::create_dir_all(root.join("a/b")).expect("folders");

    assert_eq!(resolve(&root.join("a/b")).expect("resolved"), root.join("a/b"));
    assert!(is_local(&root.join("a/b")));
}

#[test]
fn the_confined_helper_resolves_as_the_parent_does_and_still_refuses_the_names() {
    let folder = crate::gitstatus::testing::tempdir();
    let root = gix::path::realpath(folder.path()).expect("its real path");
    fs::create_dir_all(root.join("a/b")).expect("folders");
    symlink("a/b", root.join("link")).expect("a link");

    for path in [
        root.join("a/b"),
        root.join("link"),
        root.join("link/../b"),
        root.join("a/none"),
    ] {
        assert_eq!(
            resolve_confined(&path).map_err(|error| error.kind()),
            resolve(&path).map_err(|error| error.kind()),
            "{path:?}"
        );
    }
    // The names are checked all the same.
    assert_eq!(
        resolve_confined(Path::new("/net/host/x")).expect_err("refused").kind(),
        io::ErrorKind::InvalidInput
    );
}
