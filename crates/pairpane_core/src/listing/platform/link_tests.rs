//! Tests of [`local_target`], and the Windows network chain the listing tests share.

use super::*;

/// Windows only: in `dir`, `b` is a link to the network or device path `remote` and `a` a link to `b`. `None` when
/// Windows does not let this user make symbolic links (it can need a privilege), so the test has nothing to check.
#[cfg(windows)]
pub(crate) fn network_chain(dir: &Path, remote: &str) -> Option<(PathBuf, PathBuf)> {
    use std::os::windows::fs::symlink_dir;

    let (first, second) = (dir.join("a"), dir.join("b"));
    symlink_dir(remote, &second).ok()?;
    symlink_dir(&second, &first).ok()?;

    Some((first, second))
}

/// Network and device paths a chain may end at.
#[cfg(windows)]
pub(crate) const REMOTES: [&str; 3] = [r"\\?\UNC\attacker\share", r"\\attacker\share", r"\\.\pipe\attacker"];

#[cfg(windows)]
mod windows {
    use std::os::windows::fs::symlink_dir;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_chain_ending_at_a_network_or_device_path_is_refused() {
        for remote in REMOTES {
            let temp = TempDir::new().expect("temp dir");
            let Some((first, second)) = network_chain(temp.path(), remote) else {
                return;
            };

            assert_eq!(local_target(&second), None, "{remote}");
            assert_eq!(local_target(&first), None, "{remote}");
        }
    }

    #[test]
    fn a_path_through_a_folder_link_to_a_network_share_is_refused() {
        for remote in REMOTES {
            let temp = TempDir::new().expect("temp dir");
            let (folder, through) = (temp.path().join("dirlink"), temp.path().join("a"));

            if symlink_dir(remote, &folder).is_err() || symlink_dir(folder.join("y"), &through).is_err() {
                return;
            }

            assert_eq!(local_target(&through), None, "{remote}");
        }
    }

    #[test]
    fn a_network_link_anywhere_on_a_path_is_refused() {
        for remote in REMOTES {
            let temp = TempDir::new().expect("temp dir");
            let folder = temp.path().join("dirlink");

            if symlink_dir(remote, &folder).is_err() {
                return;
            }

            for path in [folder.clone(), folder.join("deep").join("er")] {
                let error = resolve_local(&path).expect_err(remote);
                assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{remote}");
            }
        }
    }

    /// A name Windows rewrites (`x ` is opened as `x`) must never be resolved: checked as `x` it could pass, and then
    /// be used as the literal `x `, which can be a link to a network share.
    #[test]
    fn names_windows_rewrites_are_refused_in_a_path_and_in_a_link_target() {
        let temp = TempDir::new().expect("temp dir");
        fs::create_dir(temp.path().join("real")).expect("dir");

        for name in ["x ", "x.", "NUL", "com1.txt"] {
            let error = resolve_local(&temp.path().join("real").join(name)).expect_err(name);
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name:?} in a path");

            // A relative link target naming it: `a -> "x "`.
            let link = temp.path().join("a");
            if symlink_dir(name, &link).is_err() {
                return;
            }
            let error = resolve_local(&link).expect_err(name);
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name:?} as a target");
            fs::remove_dir(&link).ok();
        }
    }

    /// `read_link` cannot tell `\x` (relative to the link's drive) from a path in the system's own namespace
    /// (`\Device\Mup\h\s`, which leads to the network), so both are refused.
    #[test]
    fn a_target_rooted_without_a_drive_is_refused() {
        for target in [
            r"\x",
            r"\Device\Mup\attacker\share",
            r"\GLOBAL??\UNC\attacker\share",
            r"\DosDevices\UNC\a\b",
        ] {
            let temp = TempDir::new().expect("temp dir");
            let link = temp.path().join("a");

            if symlink_dir(target, &link).is_err() {
                return;
            }

            let error = resolve_local(&link).expect_err(target);
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{target}");
            assert_eq!(
                resolve_local(&link.join("deeper")).map_err(|e| e.kind()),
                Err(io::ErrorKind::InvalidInput)
            );
        }
    }

    #[test]
    fn a_drive_relative_target_is_refused() {
        let temp = TempDir::new().expect("temp dir");
        let link = temp.path().join("a");

        // Without the privilege, or when Windows stores the target in another form, there is nothing to test.
        if symlink_dir(r"C:somewhere", &link).is_err()
            || fs::read_link(&link).ok().as_deref() != Some(Path::new(r"C:somewhere"))
        {
            return;
        }

        let error = resolve_local(&link).expect_err("drive-relative");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_chain_of_local_links_still_resolves() {
        let temp = TempDir::new().expect("temp dir");
        fs::create_dir(temp.path().join("real")).expect("dir");
        let (first, second) = (temp.path().join("a"), temp.path().join("b"));

        if symlink_dir(temp.path().join("real"), &second).is_err() || symlink_dir(&second, &first).is_err() {
            return;
        }

        let resolved = local_target(&first).expect("local chain");

        assert!(fs::metadata(resolved).expect("metadata").is_dir());
    }
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::super::resolve::MAX_LINKS;
    use super::*;

    /// A temp dir with a real folder `real`, and the folder's resolved path.
    fn setup() -> (TempDir, PathBuf) {
        let temp = TempDir::new().expect("temp dir");
        fs::create_dir(temp.path().join("real")).expect("dir");
        let real = fs::canonicalize(temp.path().join("real")).expect("real");

        (temp, real)
    }

    #[test]
    fn a_chain_of_links_resolves_to_the_folder_it_ends_at() {
        let (temp, real) = setup();
        symlink(temp.path().join("real"), temp.path().join("b")).expect("link");
        symlink(temp.path().join("b"), temp.path().join("a")).expect("link");

        assert_eq!(local_target(&temp.path().join("a")), Some(real));
    }

    #[test]
    fn relative_targets_start_from_the_folder_holding_the_link() {
        let (temp, real) = setup();
        fs::create_dir_all(temp.path().join("sub/deep")).expect("dirs");
        symlink("../real", temp.path().join("sub/up")).expect("link");
        symlink("up", temp.path().join("sub/again")).expect("link");
        symlink("./deep/../../real", temp.path().join("sub/winding")).expect("link");

        assert_eq!(local_target(&temp.path().join("sub/up")), Some(real.clone()));
        assert_eq!(local_target(&temp.path().join("sub/again")), Some(real.clone()));
        assert_eq!(local_target(&temp.path().join("sub/winding")), Some(real));
    }

    #[test]
    fn a_link_inside_a_linked_folder_is_relative_to_the_real_folder() {
        let (temp, real) = setup();
        fs::create_dir_all(temp.path().join("one/two")).expect("dirs");
        fs::create_dir(temp.path().join("other")).expect("dir");
        symlink("../real", temp.path().join("other/inner")).expect("link");
        // Through `alias` the link sits at `one/two/alias/inner`, but `..` means the real parent of `other`.
        symlink(temp.path().join("other"), temp.path().join("one/two/alias")).expect("link");

        let resolved = local_target(&temp.path().join("one/two/alias/inner"));

        assert_eq!(resolved, Some(real));
    }

    #[test]
    fn loops_and_endless_chains_have_no_target() {
        let (temp, real) = setup();
        // The temp folder's own path may hold links (`/var` is one on macOS) that would count against the limit.
        let base = real.parent().expect("temp folder").to_owned();
        symlink("self", base.join("self")).expect("link");
        symlink("two", base.join("one")).expect("link");
        symlink("one", base.join("two")).expect("link");
        symlink(&real, base.join("link-0")).expect("link");
        for index in 1..=MAX_LINKS {
            symlink(format!("link-{}", index - 1), base.join(format!("link-{index}"))).expect("link");
        }

        assert_eq!(local_target(&base.join("self")), None);
        assert_eq!(local_target(&base.join("one")), None);
        assert_eq!(local_target(&base.join(format!("link-{MAX_LINKS}"))), None);
        assert!(local_target(&base.join(format!("link-{}", MAX_LINKS - 2))).is_some());
        drop(temp);
    }

    #[test]
    fn a_path_through_a_folder_link_resolves_and_says_why_it_cannot() {
        let (temp, real) = setup();
        fs::create_dir(temp.path().join("real/sub")).expect("dir");
        symlink(temp.path().join("real"), temp.path().join("alias")).expect("link");
        symlink("loop", temp.path().join("loop")).expect("link");

        assert_eq!(
            resolve_local(&temp.path().join("alias/sub")).ok(),
            Some(real.join("sub"))
        );
        assert_eq!(
            resolve_local(&real).ok(),
            Some(real.clone()),
            "an ordinary path is itself"
        );
        let missing = resolve_local(&temp.path().join("alias/missing")).expect_err("missing");
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
        let looping = resolve_local(&temp.path().join("loop/sub")).expect_err("loop");
        assert_eq!(looping.kind(), io::ErrorKind::InvalidInput);
        let relative = resolve_local(Path::new("relative")).expect_err("relative");
        assert_eq!(relative.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_path_with_too_many_components_is_refused() {
        let temp = TempDir::new().expect("temp dir");
        let mut endless = temp.path().to_owned();
        for _ in 0..5_000 {
            endless.push("..");
        }

        let error = resolve_local(&endless).expect_err("long");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_broken_link_or_relative_path_has_no_target() {
        let (temp, _) = setup();
        symlink("missing", temp.path().join("broken")).expect("link");
        symlink("broken", temp.path().join("onwards")).expect("link");

        assert_eq!(local_target(&temp.path().join("broken")), None);
        assert_eq!(local_target(&temp.path().join("onwards")), None);
        assert_eq!(local_target(Path::new("relative")), None);
    }
}
