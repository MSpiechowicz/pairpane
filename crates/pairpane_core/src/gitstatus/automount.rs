//! The folders a system mounts a network share into on first use. Looking at a path below one (an `lstat`, an `open`) is what
//! mounts it, and that is a connection to a host that a repository's pointer chose, so a path that leads there is refused before
//! anything below the mount point is looked up. Two checks, one of which holds when the other does not:
//!
//! - **By name** ([`is_automount`]): macOS `/net` and `/Network`, and `/home`, which is an autofs mount point there, each also as
//!   seen through the data volume (`/System/Volumes/Data/...`). On macOS the names are compared as the file system compares
//!   them (see [`super::names`]), since its default file system is case-insensitive: `/Net/x`, `/NET`, `/Networ\u{212A}` and
//!   `/system/volumes/data/net/x` are the same places as the spellings above.
//! - **By file system** ([`is_autofs`]): the folder that is about to be entered is asked, through a descriptor that was opened
//!   without mounting anything, whether it is an autofs mount point (`f_type` of `fstatfs` on Linux; the file system's name or
//!   `MNT_AUTOMOUNTED` on macOS). That also covers a folder a site's own automount map names (`/mnt/auto`, `/misc`), AFS's dynamic
//!   root and whatever else the names do not list.

use std::path::{Component, Path};

use super::links::normalise;
use super::names::same_name;

/// Whether `place` is under a folder the system mounts a network share into on first use (macOS `/net`, `/Network` and `/home`,
/// which is an autofs mount point there, also as seen through the data volume), after its `..` are taken out. Never on Windows,
/// where the same names are ordinary folders, and `/home` is one on Linux.
/// A link on the way is not looked at here; see [`super::links::resolve`].
pub(super) fn is_automount(place: &Path) -> bool {
    !cfg!(windows) && is_automount_with(place, cfg!(target_os = "macos"))
}

/// [`is_automount`] for a system that is macOS (`macos`: names are compared without regard to case and to the way Unicode spells
/// them, and `/home` is an automount point) or is not (names are compared as they are written, and `/home` is a folder).
pub(super) fn is_automount_with(place: &Path, macos: bool) -> bool {
    let place = normalise(place);
    let mut names = place.components().filter_map(|component| match component {
        Component::Normal(name) => Some(name.to_string_lossy()),
        _ => None,
    });
    let is = |name: &str, wanted: &str| same_name(name, wanted, macos);

    let Some(first) = names.next() else {
        return false;
    };

    // `/.nofollow/…`, `/.resolve/<n>/…` and `/.vol/<dev>/<ino>` are prefixes macOS's kernel reads as a way of naming another
    // path (with which `/.nofollow/net/<host>` is `/net/<host>`), so on macOS none of them is let through whatever follows.
    let names_another_path = macos && (is(&first, ".nofollow") || is(&first, ".resolve") || is(&first, ".vol"));

    if names_another_path || is(&first, "net") || is(&first, "Network") {
        true
    } else if is(&first, "home") {
        macos
    } else if is(&first, "System") {
        let rest: Vec<_> = names.take(3).collect();

        matches!(rest.as_slice(), [volumes, data, name]
            if is(volumes, "Volumes") && is(data, "Data") && automounted(name, macos))
    } else {
        false
    }
}

/// Whether the name `name` of the data volume's top folder is an automount point (`net` and `Network`, and on macOS `home`).
fn automounted(name: &str, macos: bool) -> bool {
    same_name(name, "net", macos) || same_name(name, "Network", macos) || (macos && same_name(name, "home", macos))
}

/// Whether the folder `path` (one that `lstat` said is a real folder, no link) is an autofs mount point or on autofs, asked of a
/// descriptor opened so that the open mounts nothing (Linux: `O_PATH`, which never triggers an automount; macOS: `O_SYMLINK`
/// and `O_EVTONLY`, which do not either), and never by `statfs(path)`, which may itself trigger the mount. A folder that cannot be
/// opened for a reason other than that it is gone, or that nobody may read, is taken for one.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn is_autofs(path: &Path) -> bool {
    use rustix::fs::{Mode, fstatfs, open};
    use rustix::io::Errno;

    match open(path, probe_flags(), Mode::empty()) {
        Ok(fd) => fstatfs(&fd).map_or(true, |stat| on_autofs(&stat)),
        // Gone since it was looked at: the next look at it says so. Not to be opened by this user: it is no mount point that
        // the user's own lookups would trigger (a mount point is a folder anyone may enter).
        Err(Errno::NOENT | Errno::NOTDIR | Errno::ACCESS) => false,
        Err(_) => true,
    }
}

/// Where there is no autofs to ask (another Unix): never. (Windows resolves paths by its own code, see `platform::resolve_local`.)
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub(super) fn is_autofs(_path: &Path) -> bool {
    false
}

#[cfg(target_os = "linux")]
fn probe_flags() -> rustix::fs::OFlags {
    use rustix::fs::OFlags;

    // No `O_DIRECTORY`: with it the lookup asks for a folder, which makes the kernel mount a direct automount point it lands
    // on (`follow_automount`), and then `fstatfs` would see the mounted file system rather than the trigger.
    OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

#[cfg(target_os = "macos")]
fn probe_flags() -> rustix::fs::OFlags {
    use rustix::fs::OFlags;

    let evtonly = OFlags::from_bits_retain(u32::try_from(libc::O_EVTONLY).unwrap_or(0));

    OFlags::RDONLY | OFlags::SYMLINK | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC | evtonly
}

/// `AUTOFS_SUPER_MAGIC`: the `f_type` of an autofs mount.
#[cfg(target_os = "linux")]
const AUTOFS_SUPER_MAGIC: u64 = 0x0187;

/// `MNT_AUTOMOUNTED` of macOS: the mount was made by an automounter.
#[cfg(any(target_os = "macos", test))]
const MNT_AUTOMOUNTED: u32 = 0x0040_0000;

#[cfg(target_os = "linux")]
fn on_autofs(stat: &rustix::fs::StatFs) -> bool {
    u64::try_from(stat.f_type).is_ok_and(|kind| kind == AUTOFS_SUPER_MAGIC)
}

#[cfg(target_os = "macos")]
fn on_autofs(stat: &rustix::fs::StatFs) -> bool {
    let name: Vec<u8> = stat
        .f_fstypename
        .iter()
        .map(|c| c.cast_unsigned())
        .take_while(|c| *c != 0)
        .collect();

    is_autofs_on_macos(&name, stat.f_flags)
}

/// Whether the file system named `fstypename` (`f_fstypename`, without its NUL) with the mount flags `flags` is an automounter's.
#[cfg(any(target_os = "macos", test))]
fn is_autofs_on_macos(fstypename: &[u8], flags: u32) -> bool {
    fstypename == b"autofs" || flags & MNT_AUTOMOUNTED != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ways a name that is an automount point on macOS is spelled, and the folder they lead to, there.
    const MACOS_SPELLINGS: [&str; 12] = [
        "/net/x",
        "/Net/x",
        "/NET",
        "/Networ\u{212A}/Servers",
        "/network/Servers",
        "/NETWORK",
        "/HOME/x",
        "/Home",
        "/system/volumes/data/net/x",
        "/SYSTEM/VOLUMES/DATA/NETWORK/x",
        "/System/Volumes/Data/Networ\u{212A}/x",
        "/System/Volumes/Data/HOME/x",
    ];

    #[test]
    fn every_spelling_of_an_automount_point_is_refused_on_macos() {
        for text in MACOS_SPELLINGS {
            assert!(is_automount_with(Path::new(text), true), "{text}");
        }
        // The kernel's prefixes that name another path are refused, in any case.
        for text in [
            "/.nofollow/net/x",
            "/.NOFOLLOW/Net",
            "/.resolve/1/net/x",
            "/.vol/1/2",
            "/srv/../.nofollow/x",
        ] {
            assert!(is_automount_with(Path::new(text), true), "{text}");
            assert!(
                !is_automount_with(Path::new(text), false) || text.contains("/net"),
                "{text}"
            );
        }
        // Dots are read away first, whatever case the names after them have.
        for text in [
            "/srv/../NET/x",
            "/./Networ\u{212A}",
            "/home/../System/Volumes/DATA/network",
        ] {
            assert!(is_automount_with(Path::new(text), true), "{text}");
        }
    }

    #[test]
    fn a_name_that_is_no_automount_point_stays_fine_whatever_its_case_on_macos() {
        for text in [
            "/srv/net/x",
            "/net2",
            "/Net2/x",
            "/networks",
            "/Users/Net",
            "/System/Library",
            "/System/Volumes/Data/Users",
            "/System/Volumes/Data",
            "/System/Volumes/Data/net2",
            "/Networ\u{3BA}",
            "/",
            "/Volumes/Home",
        ] {
            assert!(!is_automount_with(Path::new(text), true), "{text}");
        }
    }

    #[test]
    fn elsewhere_names_are_compared_as_they_are_written_and_home_is_a_folder() {
        for text in [
            "/net/x",
            "/Network",
            "/System/Volumes/Data/net/x",
            "/System/Volumes/Data/Network",
        ] {
            assert!(is_automount_with(Path::new(text), false), "{text}");
        }
        // A case-sensitive file system has a folder `/Net` that is not `/net`, and `/home` is no mount point.
        for text in [
            "/Net/x",
            "/NET",
            "/Networ\u{212A}/Servers",
            "/network/x",
            "/system/volumes/data/net/x",
            "/home/x",
            "/HOME",
            "/System/Volumes/Data/home",
        ] {
            assert!(!is_automount_with(Path::new(text), false), "{text}");
        }
    }

    #[test]
    fn a_name_that_is_not_text_is_no_automount_point() {
        #[cfg(unix)]
        {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;

            let odd = Path::new(OsStr::from_bytes(b"/\xff/ok"));

            assert!(!is_automount_with(odd, true));
            assert!(!is_automount_with(odd, false));
            // A name that is no text does not stand for the one after it.
            assert!(!is_automount_with(Path::new(OsStr::from_bytes(b"/\xff/net")), true));
        }
    }

    #[test]
    fn the_systems_own_flavour_is_the_one_that_is_used() {
        let place = Path::new("/Net/x");

        assert_eq!(is_automount(place), cfg!(target_os = "macos"));
        assert_eq!(is_automount(Path::new("/net/x")), !cfg!(windows));
        assert_eq!(is_automount(Path::new("/home/x")), cfg!(target_os = "macos"));
    }

    #[test]
    fn a_file_system_is_an_automounters_by_its_name_or_its_flag_on_macos() {
        assert!(is_autofs_on_macos(b"autofs", 0));
        assert!(is_autofs_on_macos(b"apfs", MNT_AUTOMOUNTED));
        assert!(is_autofs_on_macos(b"nfs", 0x0000_1000 | MNT_AUTOMOUNTED));
        assert!(!is_autofs_on_macos(b"apfs", 0x0000_1000));
        assert!(!is_autofs_on_macos(b"autofs2", 0));
        assert!(!is_autofs_on_macos(b"", 0));
    }

    #[cfg(unix)]
    #[test]
    fn an_ordinary_folder_is_not_on_autofs() {
        let folder = crate::gitstatus::testing::tempdir();
        std::fs::create_dir(folder.path().join("inner")).expect("a folder");

        for place in [
            folder.path().to_path_buf(),
            folder.path().join("inner"),
            "/".into(),
            "/usr".into(),
        ] {
            assert!(!is_autofs(&place), "{place:?}");
        }
        // Gone, or no folder: nothing to refuse (the next look says what it is).
        assert!(!is_autofs(&folder.path().join("none")));
        std::fs::write(folder.path().join("file"), b"x").expect("a file");
        assert!(!is_autofs(&folder.path().join("file")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_autofs_mount_point_of_this_machine_is_found_when_there_is_one() {
        // Whatever autofs mounts the machine has (systemd's `binfmt_misc` and `efi` automounts, a site's maps): each is found,
        // unless something was mounted over it already (the open follows the mount that is on top, and that one is no autofs).
        let mounts = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        let mut on_top: Vec<(&str, &str)> = Vec::new();

        for line in mounts.lines() {
            let Some((before, after)) = line.split_once(" - ") else {
                continue;
            };
            let (Some(point), Some(kind)) = (before.split(' ').nth(4), after.split(' ').next()) else {
                continue;
            };

            on_top.retain(|(seen, _)| *seen != point);
            on_top.push((point, kind));
        }

        for (point, kind) in &on_top {
            assert_eq!(is_autofs(Path::new(point)), *kind == "autofs", "{point} ({kind})");
        }
    }
}
