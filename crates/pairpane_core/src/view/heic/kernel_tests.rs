//! The seccomp filter of the git status helper (`Filter::ReadOnly`) run on a small interpreter of classic BPF, call by call:
//! what it answers for each call number and argument, for the architecture the tests are built for, without installing it (a
//! filter on the test process would filter the other tests). The kernel's own refusals are tried in
//! `gitstatus::confine_tests`.

use libc::sock_filter;

use super::kernel::{AUDIT_ARCH, RET_ALLOW, RET_EPERM, program};
use super::writes::HIGHEST_REVIEWED;

const ENOSYS: u32 = 0x0005_0000 | libc::ENOSYS as u32;

/// The answer of `filter` to the call `number` with the first three arguments `args` (the process `pid` is the one asking).
fn answer(filter: &[sock_filter], arch: u32, number: u32, args: [u64; 3]) -> u32 {
    // `struct seccomp_data`: the number, the architecture, the instruction pointer, the six arguments.
    let mut data = [0_u8; 64];
    data[0..4].copy_from_slice(&number.to_le_bytes());
    data[4..8].copy_from_slice(&arch.to_le_bytes());
    data[16..24].copy_from_slice(&args[0].to_le_bytes());
    data[24..32].copy_from_slice(&args[1].to_le_bytes());
    data[32..40].copy_from_slice(&args[2].to_le_bytes());
    let word = |at: u32| {
        let at = at as usize;

        u32::from_le_bytes(data[at..at + 4].try_into().expect("four bytes"))
    };

    let (mut accumulator, mut at) = (0_u32, 0_usize);
    loop {
        let step = filter[at];
        at += 1;
        let jump = |taken: bool| usize::from(if taken { step.jt } else { step.jf });

        match step.code {
            0x20 => accumulator = word(step.k),
            0x15 => at += jump(accumulator == step.k),
            0x25 => at += jump(accumulator > step.k),
            0x45 => at += jump(accumulator & step.k != 0),
            0x06 => return step.k,
            other => panic!("an instruction the filter should not have: {other:#x}"),
        }
    }
}

fn read_only() -> (Vec<sock_filter>, u32) {
    (
        program(4242, true).expect("a filter for this architecture"),
        AUDIT_ARCH.expect("an architecture"),
    )
}

fn call(number: i64, args: [u64; 3]) -> u32 {
    let (filter, arch) = read_only();

    answer(&filter, arch, number as u32, args)
}

#[test]
fn the_calls_that_read_are_allowed_and_the_ones_that_write_or_lock_are_refused() {
    for number in [
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_mmap,
        libc::SYS_fstat,
        libc::SYS_getdents64,
        libc::SYS_close,
        libc::SYS_futex,
        libc::SYS_exit_group,
    ] {
        assert_eq!(call(number, [0, 0, 0]), RET_ALLOW, "call {number}");
    }

    // The new calls that write, by number (the C library may not name them), the call that locks, and the old ones.
    for number in [
        452,
        463,
        466,
        467,
        469,
        libc::SYS_flock,
        libc::SYS_unlinkat,
        libc::SYS_fchmodat,
        libc::SYS_setxattr,
        libc::SYS_mount,
        libc::SYS_socket,
        libc::SYS_execve,
    ] {
        assert_eq!(call(number, [0, 0, 0]), RET_EPERM, "call {number}");
    }
    // Reads of attributes by the new calls are not writes.
    for number in [464, 465, 468] {
        assert_eq!(call(number, [0, 0, 0]), RET_ALLOW, "call {number}");
    }
}

#[test]
fn every_call_numbered_above_the_highest_that_was_reviewed_is_refused() {
    for number in [
        HIGHEST_REVIEWED + 1,
        HIGHEST_REVIEWED + 2,
        500,
        1_000,
        9_999,
        0x3fff_ffff,
    ] {
        assert_eq!(call(i64::from(number), [0, 0, 0]), RET_EPERM, "call {number}");
    }
    assert_eq!(
        call(i64::from(HIGHEST_REVIEWED), [0, 0, 0]),
        RET_EPERM,
        "the highest is a call that writes"
    );
    assert_eq!(
        call(i64::from(HIGHEST_REVIEWED) - 1, [0, 0, 0]),
        RET_ALLOW,
        "the one before it reads"
    );
}

#[test]
fn an_ioctl_is_refused_but_for_two_questions() {
    let ioctl = i64::from(libc::SYS_ioctl as i32);

    // Allowed: how many bytes can be read (`FIONREAD`), and the terminal settings (`TCGETS`).
    for request in [0x541b_u64, 0x5401] {
        assert_eq!(call(ioctl, [3, request, 0]), RET_ALLOW, "{request:#x}");
    }
    // Refused: pushing a character into a terminal (`TIOCSTI`), setting the flags of a file, cloning its extents, anything else.
    for request in [0x5412_u64, 0x4004_6602, 0x4004_6602 | 1, 0x4020_9409, 0xc020_9405, 0] {
        assert_eq!(call(ioctl, [3, request, 0]), RET_EPERM, "{request:#x}");
    }
}

#[test]
fn an_open_that_writes_is_refused_and_one_that_reads_is_allowed() {
    let flags = |flags: libc::c_int| u64::from(flags as u32);

    // `openat(dirfd, path, flags)` has its flags in the third argument.
    let openat = libc::SYS_openat;
    assert_eq!(
        call(openat, [0, 0, flags(libc::O_RDONLY | libc::O_NOFOLLOW)]),
        RET_ALLOW
    );
    for write in [
        libc::O_WRONLY,
        libc::O_RDWR,
        libc::O_CREAT,
        libc::O_TRUNC,
        libc::O_CREAT | libc::O_EXCL,
    ] {
        assert_eq!(call(openat, [0, 0, flags(write)]), RET_EPERM, "{write:#x}");
    }

    // The old `open` has them in the second (x86-64 only).
    #[cfg(target_arch = "x86_64")]
    {
        let open = libc::SYS_open;

        assert_eq!(call(open, [0, flags(libc::O_RDONLY), 0]), RET_ALLOW);
        for write in [libc::O_WRONLY, libc::O_RDWR, libc::O_CREAT, libc::O_TRUNC] {
            assert_eq!(call(open, [0, flags(write), 0]), RET_EPERM, "{write:#x}");
        }
    }
    // `openat2` is answered `ENOSYS`, and the C library goes on with `openat`.
    assert_eq!(call(libc::SYS_openat2, [0, 0, 0]), ENOSYS);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn a_call_of_the_x32_abi_and_another_architecture_are_refused() {
    let (filter, arch) = read_only();

    assert_eq!(
        answer(&filter, arch, 0x4000_0000 | libc::SYS_read as u32, [0, 0, 0]),
        RET_EPERM
    );
    // Another architecture kills the process.
    assert_eq!(
        answer(&filter, 0x4000_0003, libc::SYS_read as u32, [0, 0, 0]),
        0x8000_0000
    );
}

#[test]
fn the_deny_list_of_the_picture_helpers_is_not_the_read_only_filter() {
    // The HEIC helper's filter is the deny-list alone: a call numbered high, a lock and an ioctl are for its codecs.
    let (filter, arch) = (
        program(4242, false).expect("a filter"),
        AUDIT_ARCH.expect("an architecture"),
    );

    for number in [libc::SYS_flock as u32, libc::SYS_ioctl as u32, 500, 9_999] {
        assert_eq!(
            answer(&filter, arch, number, [0x5412, 0x5412, 0]),
            RET_ALLOW,
            "call {number}"
        );
    }
    assert_eq!(answer(&filter, arch, libc::SYS_socket as u32, [0, 0, 0]), RET_EPERM);
}
