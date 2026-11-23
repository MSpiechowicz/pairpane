//! The raw calls the tests of the sandbox make to see what the filters refuse (the confinement itself is in `kernel.rs`).
#![allow(unsafe_code, reason = "raw system calls for the tests; see each `unsafe` block")]
#![deny(clippy::undocumented_unsafe_blocks)]

use std::io;

/// The error number of an `io_uring_setup` call, for the tests that check the filter refuses it (`0` if it worked; the
/// ring it then made is closed at once).
pub(super) fn io_uring_setup_errno() -> i32 {
    // `struct io_uring_params` is 120 bytes; zeroed it asks for nothing special.
    let mut params = [0_u8; 120];

    // SAFETY: `io_uring_setup(entries, params)` reads and writes the 120 bytes of `params`, which live through the call.
    let fd = unsafe { libc::syscall(libc::SYS_io_uring_setup, 4_u32, params.as_mut_ptr()) };
    if fd < 0 {
        return io::Error::last_os_error().raw_os_error().unwrap_or(-1);
    }

    // SAFETY: `fd` is the ring just made and owned by nothing else.
    unsafe { libc::close(fd as libc::c_int) };

    0
}

/// The error numbers of calls the filter must refuse, for the tests: a new process by `fork` (that is `clone` without
/// `CLONE_THREAD`), `clone3`, System V shared memory, a signal to the parent, and a look at the parent's limits (`0`
/// for one that worked; a child that `fork` made exits at once).
pub(super) fn refused_errnos() -> [i32; 5] {
    let errno = |result: i64| {
        if result < 0 {
            io::Error::last_os_error().raw_os_error().unwrap_or(-1)
        } else {
            0
        }
    };

    // SAFETY: `fork` takes nothing; a child it makes runs only `_exit`, which is safe after a fork.
    let forked = unsafe { libc::fork() };
    if forked == 0 {
        // SAFETY: ends the child at once, without running anything of the parent's state.
        unsafe { libc::_exit(0) };
    }
    let fork = errno(i64::from(forked));
    if forked > 0 {
        // SAFETY: waits for the child just made; the status is not read.
        unsafe { libc::waitpid(forked, std::ptr::null_mut(), 0) };
    }

    // SAFETY: `clone3` with NULL arguments and size 0 would only fail in the kernel (`EINVAL`); the filter answers first.
    let clone3 = errno(unsafe { libc::syscall(libc::SYS_clone3, std::ptr::null::<libc::c_void>(), 0_usize) });

    // SAFETY: `shmget` takes its arguments by value and only makes (or refuses) a private segment.
    let segment = unsafe { libc::shmget(libc::IPC_PRIVATE, 4096, libc::IPC_CREAT | 0o600) };
    let shm = errno(i64::from(segment));
    if segment >= 0 {
        // SAFETY: removes the segment just made.
        unsafe { libc::shmctl(segment, libc::IPC_RMID, std::ptr::null_mut()) };
    }

    // SAFETY: signal 0 to the parent only checks the permission; nothing is sent.
    let parent = unsafe { libc::getppid() };
    // SAFETY: as above.
    let signal = errno(i64::from(unsafe { libc::kill(parent, 0) }));

    let mut limit = libc::rlimit64 {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: reads the parent's limit on open files into `limit`, which lives through the call; sets nothing.
    let prlimit = errno(i64::from(unsafe {
        libc::prlimit64(parent, libc::RLIMIT_NOFILE, std::ptr::null(), &raw mut limit)
    }));

    [fork, clone3, shm, signal, prlimit]
}

/// The error number of the `mmap` of a page with `prot`, or `0` when it worked (the page is unmapped at once).
pub(super) fn mmap_errno(prot: libc::c_int) -> i32 {
    // SAFETY: an anonymous private mapping of one page, no address asked for: it only makes (or refuses) a mapping of its
    // own; the page is unmapped again below.
    let page = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            4096,
            prot,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if page == libc::MAP_FAILED {
        return std::io::Error::last_os_error().raw_os_error().unwrap_or(-1);
    }

    // SAFETY: `page` is the mapping just made and nothing uses it.
    unsafe { libc::munmap(page, 4096) };

    0
}

/// The error number of a `uname` call, or `0` when it worked: a call that the allow-list filter does not list.
pub(super) fn uname_errno() -> i32 {
    // SAFETY: `uname` writes a `utsname` into the structure it is given, which lives through the call.
    unsafe {
        let mut name: libc::utsname = std::mem::zeroed();
        if libc::uname(&raw mut name) == 0 {
            0
        } else {
            io::Error::last_os_error().raw_os_error().unwrap_or(-1)
        }
    }
}

/// The error number of the `mmap` of a page of the open file `fd` with `PROT_READ`, or `0` when it worked: a mapping that
/// is not anonymous, which the allow-list filter refuses.
pub(super) fn file_mmap_errno(fd: libc::c_int) -> i32 {
    // SAFETY: a private read-only mapping of one page of `fd`, no address asked for: it only makes (or refuses) a mapping
    // of its own, unmapped again below.
    let page = unsafe { libc::mmap(std::ptr::null_mut(), 4096, libc::PROT_READ, libc::MAP_PRIVATE, fd, 0) };
    if page == libc::MAP_FAILED {
        return std::io::Error::last_os_error().raw_os_error().unwrap_or(-1);
    }

    // SAFETY: `page` is the mapping just made and nothing uses it.
    unsafe { libc::munmap(page, 4096) };

    0
}

/// The error number of `mprotect` asking `PROT_EXEC` for a page that was mapped read-write, or `0` when it worked.
pub(super) fn mprotect_exec_errno() -> i32 {
    // SAFETY: an anonymous private read-write page, no address asked for, which is only asked to become executable (the
    // filter or the system refuses) and is unmapped again below.
    unsafe {
        let page = libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        );
        if page == libc::MAP_FAILED {
            return -1;
        }

        let errno = if libc::mprotect(page, 4096, libc::PROT_READ | libc::PROT_EXEC) == 0 {
            0
        } else {
            io::Error::last_os_error().raw_os_error().unwrap_or(-1)
        };
        libc::munmap(page, 4096);

        errno
    }
}

/// The error numbers of what [`Filter::ReadOnly`](super::kernel::Filter::ReadOnly) refuses and Landlock does not stop, for the
/// tests: a request to a terminal that pushes input (`TIOCSTI`, asked of the open file `fd`: it is refused before it is asked
/// whether `fd` is a terminal), a lock (`flock`), an attribute set by `setxattrat`, a call no list knows (a number above the
/// highest that was reviewed), and then a question the filter lets through (`FIONREAD`: any error but `EPERM`, or `0`).
pub(crate) fn read_only_errnos(fd: libc::c_int) -> [i32; 5] {
    let errno = |result: i64| {
        if result < 0 {
            io::Error::last_os_error().raw_os_error().unwrap_or(-1)
        } else {
            0
        }
    };

    let byte = b'x';
    // SAFETY: `ioctl` with `TIOCSTI` reads one byte at the address it is given, which lives through the call; the filter or
    // the kernel refuses it for a descriptor that is no terminal.
    let tiocsti = errno(i64::from(unsafe { libc::ioctl(fd, libc::TIOCSTI, &raw const byte) }));
    // SAFETY: `flock` takes the descriptor and an operation by value and touches no memory.
    let flock = errno(i64::from(unsafe { libc::flock(fd, libc::LOCK_SH | libc::LOCK_NB) }));
    // SAFETY: `setxattrat` (463) with every argument zero reads nothing, as the kernel refuses it for its bad descriptor and
    // flags at the latest; the filter answers first.
    let xattr = errno(unsafe { libc::syscall(463, 0, 0, 0, 0, 0, 0) });
    // SAFETY: a call number that no kernel has, with nothing in its arguments: the filter answers `EPERM`, the kernel `ENOSYS`.
    let unknown = errno(unsafe { libc::syscall(9_999) });

    let mut waiting: libc::c_int = 0;
    // SAFETY: `ioctl` with `FIONREAD` writes one `int` at the address it is given, which lives through the call.
    let fionread = errno(i64::from(unsafe { libc::ioctl(fd, libc::FIONREAD, &raw mut waiting) }));

    [tiocsti, flock, xattr, unknown, fionread]
}
