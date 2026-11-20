use pretty_assertions::assert_eq;

use super::*;
use crate::model::VolumeKind::{Drive, Home, Network, Removable, System};

/// A desktop's mount table (Ubuntu, trimmed): kernel interfaces, snap packages, a second disk, a USB stick with a
/// space in its label, and network mounts of every kind that matters.
const TABLE: &str = "\
sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0
proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0
udev /dev devtmpfs rw,nosuid,relatime,size=8115888k,nr_inodes=2028972,mode=755,inode64 0 0
devpts /dev/pts devpts rw,nosuid,noexec,relatime,gid=5,mode=620,ptmxmode=000 0 0
tmpfs /run tmpfs rw,nosuid,nodev,noexec,relatime,size=1631884k,mode=755,inode64 0 0
/dev/nvme0n1p2 / ext4 rw,relatime,errors=remount-ro 0 0
securityfs /sys/kernel/security securityfs rw,nosuid,nodev,noexec,relatime 0 0
tmpfs /dev/shm tmpfs rw,nosuid,nodev,inode64 0 0
cgroup2 /sys/fs/cgroup cgroup2 rw,nosuid,nodev,noexec,relatime,nsdelegate,memory_recursiveprot 0 0
systemd-1 /proc/sys/fs/binfmt_misc autofs rw,relatime,fd=29,pgrp=1,timeout=0,minproto=5,maxproto=5,direct,pipe_ino=17089 0 0
/dev/loop0 /snap/core22/1380 squashfs ro,nodev,relatime,errors=continue,threads=single 0 0
/dev/loop1 /snap/firefox/4793 squashfs ro,nodev,relatime,errors=continue,threads=single 0 0
/dev/nvme0n1p1 /boot/efi vfat rw,relatime,fmask=0077,dmask=0077,codepage=437,iocharset=iso8859-1 0 0
/dev/sda1 /mnt/data\\040disk ext4 rw,relatime 0 0
/dev/nvme1n1p1 /home/me/Games btrfs rw,relatime,ssd,subvolid=5,subvol=/ 0 0
/dev/nvme1n1p1 /var/lib/docker btrfs rw,relatime,ssd,subvolid=256,subvol=/docker 0 0
overlay /var/lib/docker/overlay2/4f8a/merged overlay rw,relatime,lowerdir=/var/lib/docker/overlay2/l/A 0 0
tmpfs /run/user/1000 tmpfs rw,nosuid,nodev,relatime,size=1631880k,mode=700,uid=1000,gid=1000,inode64 0 0
gvfsd-fuse /run/user/1000/gvfs fuse.gvfsd-fuse rw,nosuid,nodev,relatime,user_id=1000,group_id=1000 0 0
portal /run/user/1000/doc fuse.portal rw,nosuid,nodev,relatime,user_id=1000,group_id=1000 0 0
/dev/sdb1 /run/media/me/USB\\040STICK vfat rw,nosuid,nodev,relatime,uid=1000,gid=1000 0 0
/dev/sdc1 /media/me/BACKUP exfat rw,nosuid,nodev,relatime,uid=1000,gid=1000 0 0
nas.local:/export/media /mnt/nas nfs4 rw,relatime,vers=4.2,rsize=1048576,hard,proto=tcp,timeo=600,sec=sys 0 0
//server/share /home/me/share cifs rw,relatime,vers=3.1.1,cache=strict,username=me 0 0
me@host:/data /home/me/remote fuse.sshfs rw,nosuid,nodev,relatime,user_id=1000,group_id=1000 0 0
/dev/sdd1 /mnt/windows fuseblk rw,relatime,user_id=0,group_id=0,allow_other,blksize=4096 0 0
mergerfs /mnt/pool fuse.mergerfs rw,relatime,user_id=0,group_id=0 0 0
auto /mnt/auto autofs rw,relatime,fd=7,pgrp=1,timeout=300,minproto=5,maxproto=5,indirect 0 0
";

fn mounts(table: &str) -> Vec<Mount> {
    parse(table)
}

/// What a listing shows, as (name, kind, path, free space may be asked).
fn shown(found: &[Found]) -> Vec<(&str, VolumeKind, &str, bool)> {
    found
        .iter()
        .map(|found| {
            let volume = &found.volume;

            (volume.name.as_str(), volume.kind, volume.path.as_str(), found.measure)
        })
        .collect()
}

#[test]
fn a_desktop_mount_table_shows_the_places_to_go_and_nothing_else() {
    let found = classify(&mounts(TABLE), Some("/home/me"));

    assert_eq!(
        shown(&found),
        vec![
            ("System", System, "/", true),
            ("Home", Home, "/home/me", true),
            ("data disk", Drive, "/mnt/data disk", true),
            ("Games", Drive, "/home/me/Games", true),
            ("USB STICK", Removable, "/run/media/me/USB STICK", true),
            ("BACKUP", Removable, "/media/me/BACKUP", true),
            ("nas", Network, "/mnt/nas", false),
            ("share", Network, "/home/me/share", false),
            ("remote", Network, "/home/me/remote", false),
            ("windows", Drive, "/mnt/windows", true),
            ("pool", Drive, "/mnt/pool", false),
        ]
    );
}

#[test]
fn network_mounts_name_their_source_and_are_never_asked_for_free_space() {
    let found = classify(&mounts(TABLE), None);

    let network: Vec<_> = found.iter().filter(|found| found.volume.kind == Network).collect();

    assert_eq!(network.len(), 3);
    assert!(network.iter().all(|found| !found.measure));
    assert_eq!(network[0].volume.label.as_deref(), Some("nas.local:/export/media"));
    assert_eq!(network[1].volume.label.as_deref(), Some("//server/share"));
}

#[test]
fn every_network_type_is_listed_as_network_wherever_it_is_mounted() {
    for fstype in NETWORK_TYPES {
        let table = format!("server:/x /srv/files {fstype} rw 0 0\n");

        let found = classify(&mounts(&table), None);

        assert_eq!(shown(&found)[1], ("files", Network, "/srv/files", false), "{fstype}");
    }
}

#[test]
fn autofs_is_never_shown_or_asked_but_what_is_mounted_over_it_is_shown() {
    let waiting = "auto /mnt/nfs autofs rw,fd=7 0 0\n";
    let mounted = "auto /mnt/nfs autofs rw,fd=7 0 0\nnas:/export /mnt/nfs nfs4 rw 0 0\n";

    assert_eq!(shown(&classify(&mounts(waiting), None)).len(), 1, "only System");
    assert_eq!(
        shown(&classify(&mounts(mounted), None))[1],
        ("nfs", Network, "/mnt/nfs", false)
    );
}

#[test]
fn only_the_last_of_stacked_mounts_shows() {
    let table = "\
/dev/sdb1 /mnt/stack ext4 rw 0 0
/dev/sdc1 /mnt/stack xfs rw 0 0
/dev/sdd1 /mnt/hidden ext4 rw 0 0
tmpfs /mnt/hidden tmpfs rw 0 0
/dev/sda2 / ext4 rw 0 0
overlay / overlay rw 0 0
";

    let found = classify(&mounts(table), None);

    assert_eq!(
        shown(&found),
        vec![("System", System, "/", true), ("stack", Drive, "/mnt/stack", true)]
    );
}

#[test]
fn the_system_disk_is_always_listed_but_asked_only_when_it_is_local() {
    assert_eq!(
        shown(&classify(&[], Some("/home/me")))[0],
        ("System", System, "/", false)
    );
    assert_eq!(
        shown(&classify(&mounts("server:/root / nfs rw 0 0\n"), None))[0],
        ("System", System, "/", false)
    );
    assert!(
        shown(&classify(&mounts("overlay / overlay rw 0 0\n"), None))[0].3,
        "a container's root"
    );
}

#[test]
fn the_home_folder_is_asked_only_when_the_mount_that_holds_it_is_local() {
    let ask = |table: &str| shown(&classify(&mounts(table), Some("/home/me/docs")))[1].3;

    assert!(ask("/dev/sda2 / ext4 rw 0 0\n"));
    assert!(
        ask("/dev/sda2 / ext4 rw 0 0\nnas:/home /home nfs4 rw 0 0\n/dev/sdb1 /home/me ext4 rw 0 0\n"),
        "the deepest mount"
    );
    assert!(
        !ask("/dev/sda2 / ext4 rw 0 0\nnas:/home /home nfs4 rw 0 0\n"),
        "a network home"
    );
    assert!(
        !ask("/dev/sda2 / ext4 rw 0 0\nauto /home autofs rw 0 0\n"),
        "an automounted home"
    );
    assert!(
        !ask("/dev/sda2 / ext4 rw 0 0\nsshfs /home/me fuse.sshfs rw 0 0\n"),
        "a user-space filesystem"
    );
    assert!(!ask(""), "no table");
}

#[test]
fn a_mount_is_not_a_prefix_of_a_sibling_folder() {
    let table = "/dev/sda2 / ext4 rw 0 0\nnas:/x /home/me2 nfs4 rw 0 0\n";

    assert!(shown(&classify(&mounts(table), Some("/home/me")))[1].3);
}

#[test]
fn other_user_space_filesystems_are_listed_without_free_space_but_block_filesystems_keep_it() {
    let table = "\
fuse-x /mnt/x fuse.encfs rw 0 0
/dev/sdb1 /mnt/y fuseblk rw 0 0
";

    let found = classify(&mounts(table), None);

    assert_eq!(shown(&found)[1], ("x", Drive, "/mnt/x", false));
    assert_eq!(shown(&found)[2], ("y", Drive, "/mnt/y", true));
}

#[test]
fn a_plain_fuse_mount_is_never_measured_like_a_fuse_dot_one() {
    for fstype in ["fuse", "fuse.encfs"] {
        let table = format!("/dev/sdb1 /mnt/x {fstype} rw 0 0\n");

        assert_eq!(
            shown(&classify(&mounts(&table), None))[1],
            ("x", Drive, "/mnt/x", false),
            "{fstype}"
        );
    }

    assert!(
        !shown(&classify(&mounts("sshfs /home/me fuse rw 0 0\n"), Some("/home/me")))[1].3,
        "a home folder on one"
    );
}

#[test]
fn a_user_space_mount_is_not_a_drive_because_of_its_source() {
    for fstype in ["fuse", "fuse.anything", "fuse.sshfs"] {
        let table = format!("/dev/x /home/u/x {fstype} rw 0 0\n");

        let found = classify(&mounts(&table), None);

        assert!(
            found
                .iter()
                .all(|found| found.volume.path != "/home/u/x" || !found.measure),
            "{fstype}"
        );
    }

    let found = classify(&mounts("/dev/x /home/u/x fuse rw 0 0\n"), None);
    assert_eq!(shown(&found).len(), 1, "only the system disk: not a drive");

    let found = classify(&mounts("/dev/x /home/u/x ext4 rw 0 0\n"), None);
    assert_eq!(
        shown(&found)[1],
        ("x", Drive, "/home/u/x", true),
        "a real block device is"
    );
}

#[test]
fn more_network_filesystems_are_network_volumes_that_are_never_measured() {
    for fstype in ["smb", "ncpfs", "coda", "orangefs", "beegfs", "gpfs", "virtiofs"] {
        let table = format!("src /mnt/net {fstype} rw 0 0\n");

        assert_eq!(
            shown(&classify(&mounts(&table), None))[1],
            ("net", Network, "/mnt/net", false),
            "{fstype}"
        );
    }
}

#[test]
fn block_devices_in_system_folders_are_not_drives() {
    for point in [
        "/boot",
        "/boot/efi",
        "/efi",
        "/usr",
        "/var",
        "/var/log",
        "/snap/x",
        "/etc/hosts",
        "/proc/x",
    ] {
        let table = format!("/dev/sdb1 {point} ext4 rw 0 0\n");

        assert_eq!(shown(&classify(&mounts(&table), None)).len(), 1, "{point}");
    }

    let table = "/dev/sdb1 /varnish ext4 rw 0 0\n/dev/sdb2 /boots ext4 rw 0 0\n";
    assert_eq!(
        shown(&classify(&mounts(table), None)).len(),
        3,
        "a longer name is not the system folder"
    );
}

#[test]
fn parse_decodes_the_kernels_escapes() {
    let table = "/dev/sdb1 /mnt/a\\040b\\011c\\012d\\134e ext4 rw 0 0\n";

    let mounts = parse(table);

    assert_eq!(mounts[0].point, "/mnt/a b\tc\nd\\e");
}

#[test]
fn parse_decodes_utf8_bytes_and_leaves_other_backslashes_alone() {
    let table =
        "/dev/sdb1 /mnt/caf\\303\\251 ext4 rw 0 0\nsrc /mnt/a\\x\\04 ext4 rw 0 0\nsrc /mnt/b\\777 ext4 rw 0 0\n";

    let mounts = parse(table);

    assert_eq!(mounts[0].point, "/mnt/café");
    assert_eq!(mounts[1].point, "/mnt/a\\x\\04");
    assert_eq!(
        mounts[2].point, "/mnt/b\\777",
        "an octal number above a byte is not an escape"
    );
}

#[test]
fn parse_skips_lines_that_are_not_mounts() {
    let table = "\
short line
/dev/sdb1 relative ext4 rw 0 0

/dev/sdb1 /mnt/nul\\000x ext4 rw 0 0
/dev/sdb1 /mnt/bad\\377 ext4 rw 0 0
/dev/sdb1 /mnt/ok ext4 rw 0 0
";

    let mounts = parse(table);

    assert_eq!(mounts.len(), 1);
    assert_eq!(mounts[0].point, "/mnt/ok");
    assert_eq!(mounts[0].source, "/dev/sdb1");
    assert_eq!(mounts[0].fstype, "ext4");
}
