//! What [`Filter::ReadOnly`](super::kernel::Filter::ReadOnly) refuses on top of the deny-list: the calls that change a file or
//! the file system, and an open that would write one. The filter itself is built in `kernel.rs`.

use libc::{c_long, sock_filter};

use super::kernel::{
    ARG1_LOW, ARG2_LOW, BPF_JEQ_K, BPF_JSET_K, BPF_LD_W_ABS, TAIL_ALLOW, TAIL_ENOSYS, TAIL_EPERM, instruction,
};

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
    libc::SYS_fchmodat2,
    libc::SYS_fchown,
    libc::SYS_fchownat,
    libc::SYS_setxattr,
    libc::SYS_lsetxattr,
    libc::SYS_fsetxattr,
    libc::SYS_removexattr,
    libc::SYS_lremovexattr,
    libc::SYS_fremovexattr,
    libc::SYS_utimensat,
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

/// The flags of an open that write, create or truncate: `O_WRONLY`, `O_RDWR`, `O_CREAT` and `O_TRUNC` (`O_TMPFILE` needs a
/// write mode, and `O_APPEND` too). The same numbers on x86-64 and on aarch64.
const WRITE_FLAGS: u32 = (libc::O_WRONLY | libc::O_RDWR | libc::O_CREAT | libc::O_TRUNC) as u32;

/// The tests of [`Filter::ReadOnly`] on the opens, which come before the list of calls that are refused: `openat2` is
/// answered `ENOSYS`, and `openat` (and `open`, on x86-64) is allowed when its flags write nothing and refused with `EPERM`
/// when they would (see [`WRITE_FLAGS`]). The call number is in the accumulator when they begin, and when none of them applies
/// it is there again, so the tests that follow are unchanged. `tests` is how many tests follow.
pub(super) fn open_prelude(tests: usize) -> Option<Vec<sock_filter>> {
    let mut cases = vec![(libc::SYS_openat, ARG2_LOW)];
    #[cfg(target_arch = "x86_64")]
    cases.push((libc::SYS_open, ARG1_LOW));

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
