//! What [`Filter::ReadOnly`](super::kernel::Filter::ReadOnly) refuses on top of the deny-list: the calls that change a file or
//! the file system, an open that would write one, an `ioctl` that is not one of a short list of questions, and any call whose
//! number is above the highest one that was reviewed. The filter itself is built in `kernel.rs`.

use libc::{c_long, sock_filter};

use super::kernel::{
    ARG1_LOW, ARG2_LOW, BPF_JEQ_K, BPF_JSET_K, BPF_LD_W_ABS, BPF_RET_K, RET_EPERM, TAIL_ALLOW, TAIL_ENOSYS, TAIL_EPERM,
    instruction,
};

/// The calls added to Linux since the numbering became the same on every architecture (424 and up), by number, as the C
/// library of the build may not name them (`libc` has no `SYS_fchmodat2` for aarch64, and none of the others at all): the new
/// `fchmodat2`, `setxattrat`, `removexattrat`, `open_tree_attr` and `file_setattr`, which change a file or a mount.
const NEW_WRITES: [c_long; 5] = [452, 463, 466, 467, 469];

/// The highest call number that was reviewed (`file_setattr`, Linux 6.17; `listns`, the one after it, and what comes later are
/// not): a call with a higher number is refused with `EPERM` whatever it is, so that a call that a newer kernel adds, which no
/// list here knows to be harmless, is not let through by the filter's default of "allow". The calls above it that the C library
/// or gix would need are none.
pub(super) const HIGHEST_REVIEWED: u32 = 469;

/// The `ioctl` requests that are let through, which only ask a question of a descriptor: how many bytes can be read from it
/// (`FIONREAD`) and its terminal settings (`TCGETS`, which the C library asks of a descriptor to tell a terminal from a pipe).
/// No other request: those that write a terminal's input (`TIOCSTI`), change a file's attributes or flags
/// (`FS_IOC_SETFLAGS`, `FS_IOC_FSSETXATTR`), clone or deduplicate a file's extents (`FICLONE`, `FIDEDUPERANGE`) or freeze a
/// file system are refused with `EPERM`. The numbers are the same on x86-64 and on aarch64.
const IOCTLS: [u32; 2] = [0x541b, 0x5401];

/// The calls that change a file or the file system, which [`Filter::ReadOnly`] refuses with `EPERM` (what a helper may read
/// is for Landlock to say; what it may change is none, whatever Landlock is there or not): removing, renaming, linking,
/// creating (a folder, a link, a node), cutting or making room in a file, changing a mode, an owner, an attribute or a time,
/// and mounting.
pub(super) const WRITES: &[c_long] = &[
    #[cfg(target_arch = "x86_64")]
    libc::SYS_unlink,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_rename,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_mkdir,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_rmdir,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_link,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_symlink,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_creat,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_chmod,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_chown,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_lchown,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_mknod,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_utime,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_utimes,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_futimesat,
    libc::SYS_unlinkat,
    libc::SYS_renameat,
    libc::SYS_renameat2,
    libc::SYS_mkdirat,
    libc::SYS_linkat,
    libc::SYS_symlinkat,
    libc::SYS_mknodat,
    libc::SYS_truncate,
    libc::SYS_ftruncate,
    libc::SYS_fallocate,
    libc::SYS_fchmod,
    libc::SYS_fchmodat,
    libc::SYS_fchown,
    libc::SYS_fchownat,
    libc::SYS_setxattr,
    libc::SYS_lsetxattr,
    libc::SYS_fsetxattr,
    libc::SYS_removexattr,
    libc::SYS_lremovexattr,
    libc::SYS_fremovexattr,
    libc::SYS_utimensat,
    libc::SYS_flock,
    libc::SYS_mount,
    libc::SYS_umount2,
    libc::SYS_pivot_root,
    libc::SYS_chroot,
    libc::SYS_open_tree,
    libc::SYS_move_mount,
    libc::SYS_fsopen,
    libc::SYS_fsconfig,
    libc::SYS_fsmount,
    libc::SYS_fspick,
    libc::SYS_mount_setattr,
];

/// The calls of [`WRITES`] and [`NEW_WRITES`] as the numbers the filter tests.
pub(super) fn refused_writes() -> impl Iterator<Item = u32> {
    WRITES.iter().chain(&NEW_WRITES).map(|&number| number as u32)
}

/// The flags of an open that write, create or truncate: `O_WRONLY`, `O_RDWR`, `O_CREAT` and `O_TRUNC` (`O_TMPFILE` needs a
/// write mode, and `O_APPEND` too). The same numbers on x86-64 and on aarch64.
const WRITE_FLAGS: u32 = (libc::O_WRONLY | libc::O_RDWR | libc::O_CREAT | libc::O_TRUNC) as u32;

/// The tests of [`Filter::ReadOnly`] on the opens, which come before the list of calls that are refused: `openat2` is
/// answered `ENOSYS`, and `openat` (and `open`, on x86-64) is allowed when its flags write nothing and refused with `EPERM`
/// when they would (see [`WRITE_FLAGS`]). The call number is in the accumulator when they begin, and when none of them applies
/// it is there again, so the tests that follow are unchanged. `tests` is how many tests follow.
pub(super) fn open_prelude(tests: usize) -> Option<Vec<sock_filter>> {
    // The call and where its flags are (aarch64 has no `open`, only `openat`).
    #[cfg(target_arch = "x86_64")]
    let cases = [(libc::SYS_openat, ARG2_LOW), (libc::SYS_open, ARG1_LOW)];
    #[cfg(not(target_arch = "x86_64"))]
    let cases = [(libc::SYS_openat, ARG2_LOW)];

    let length = 1 + 3 * cases.len();
    // The offset from the instruction `at` of the prelude to the place `to` of the tail.
    let jump = |at: usize, to: usize| u8::try_from(length - at - 1 + tests + to).ok();

    let mut prelude = vec![instruction(
        BPF_JEQ_K,
        jump(0, TAIL_ENOSYS)?,
        0,
        libc::SYS_openat2 as u32,
    )];
    for (case, &(number, flags)) in cases.iter().enumerate() {
        let at = 1 + 3 * case;

        // Another call: past the next two instructions, to the next case.
        prelude.push(instruction(BPF_JEQ_K, 0, 2, number as u32));
        prelude.push(instruction(BPF_LD_W_ABS, 0, 0, flags));
        prelude.push(instruction(
            BPF_JSET_K,
            jump(at + 2, TAIL_EPERM)?,
            jump(at + 2, TAIL_ALLOW)?,
            WRITE_FLAGS,
        ));
    }

    Some(prelude)
}

/// How many instructions [`ioctl_prelude`] is: the test for the call, the load of the request, a test for each request that is
/// let through, and the refusal.
pub(super) const IOCTL_PRELUDE: usize = 3 + IOCTLS.len();

/// The test of [`Filter::ReadOnly`] on `ioctl`, which comes after [`open_prelude`] and before the list of calls that are
/// refused: the call is allowed when its request is one of [`IOCTLS`] and refused with `EPERM` when it is not. Another call is
/// skipped over it with the call number still in the accumulator. `tests` is how many tests follow.
pub(super) fn ioctl_prelude(tests: usize) -> Option<Vec<sock_filter>> {
    // The offset from the instruction `at` of the prelude to the place `to` of the tail.
    let jump = |at: usize, to: usize| u8::try_from(IOCTL_PRELUDE - at - 1 + tests + to).ok();

    let mut prelude = vec![
        // Another call: past the rest of the prelude.
        instruction(
            BPF_JEQ_K,
            0,
            u8::try_from(IOCTL_PRELUDE - 1).ok()?,
            libc::SYS_ioctl as u32,
        ),
        instruction(BPF_LD_W_ABS, 0, 0, ARG1_LOW),
    ];
    for (case, &request) in IOCTLS.iter().enumerate() {
        prelude.push(instruction(BPF_JEQ_K, jump(2 + case, TAIL_ALLOW)?, 0, request));
    }
    prelude.push(instruction(BPF_RET_K, 0, 0, RET_EPERM));

    Some(prelude)
}
