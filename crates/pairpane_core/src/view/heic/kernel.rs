//! The raw Linux system calls of the helper's confinement (`sandbox.rs` says what it is for): the seccomp filter, the
//! closing of the descriptors the helper inherits and the probe of the Landlock version. Each is a few calls with no
//! state, and none of them has a safe wrapper in a crate the app already uses.
#![allow(
    unsafe_code,
    reason = "raw system calls (seccomp, close_range, Landlock's version); see each `unsafe` block"
)]
#![deny(clippy::undocumented_unsafe_blocks)]

use std::io;

use libc::{c_long, sock_filter, sock_fprog};

/// `AUDIT_ARCH_*` of the architecture this is built for, which the filter requires of every call (a program of another
/// ABI that the kernel runs, the 32-bit one on x86-64, has other call numbers). `None` where there is no filter.
#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH: Option<u32> = Some(0xC000_003E);
#[cfg(target_arch = "aarch64")]
const AUDIT_ARCH: Option<u32> = Some(0xC000_00B7);
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const AUDIT_ARCH: Option<u32> = None;

/// The call numbers the filter refuses with `EPERM`:
///
/// - no socket of any kind and nothing to do with one (so no connection to a path socket, which Landlock stops only
///   from its ABI 9, and no UDP, which it never stops), io_uring included, which makes and connects sockets without
///   these calls;
/// - no new program, no new process (only threads, see [`program`]), no looking into, writing to, signalling (but
///   itself, see [`program`]), rescheduling or taking descriptors from another process, and no System V IPC or POSIX
///   message queue, which Landlock does not cover and through which the shared memory of other programs can be
///   reached;
/// - none of the kernel's other ways in that a codec never needs: BPF, perf events, the key ring, `userfaultfd` and new
///   namespaces.
const DENIED: &[c_long] = &[
    libc::SYS_socket,
    libc::SYS_socketpair,
    libc::SYS_connect,
    libc::SYS_bind,
    libc::SYS_listen,
    libc::SYS_accept,
    libc::SYS_accept4,
    libc::SYS_sendto,
    libc::SYS_sendmsg,
    libc::SYS_sendmmsg,
    libc::SYS_io_uring_setup,
    libc::SYS_io_uring_enter,
    libc::SYS_io_uring_register,
    libc::SYS_execve,
    libc::SYS_execveat,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_fork,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_vfork,
    libc::SYS_ptrace,
    libc::SYS_process_vm_writev,
    libc::SYS_process_vm_readv,
    libc::SYS_pidfd_getfd,
    libc::SYS_pidfd_send_signal,
    libc::SYS_tkill,
    libc::SYS_rt_sigqueueinfo,
    libc::SYS_rt_tgsigqueueinfo,
    libc::SYS_sched_setaffinity,
    libc::SYS_sched_setscheduler,
    libc::SYS_sched_setattr,
    libc::SYS_setpriority,
    libc::SYS_ioprio_set,
    libc::SYS_shmget,
    libc::SYS_shmat,
    libc::SYS_shmctl,
    libc::SYS_msgget,
    libc::SYS_msgsnd,
    libc::SYS_msgrcv,
    libc::SYS_msgctl,
    libc::SYS_semget,
    libc::SYS_semop,
    libc::SYS_semtimedop,
    libc::SYS_semctl,
    libc::SYS_mq_open,
    libc::SYS_bpf,
    libc::SYS_perf_event_open,
    libc::SYS_keyctl,
    libc::SYS_add_key,
    libc::SYS_request_key,
    libc::SYS_userfaultfd,
    libc::SYS_unshare,
    libc::SYS_setns,
];

/// The bit of the number of a call of the x32 ABI on x86-64, which the kernel runs under the same `AUDIT_ARCH` and which
/// has other numbers for the same calls.
#[cfg(target_arch = "x86_64")]
const X32_BIT: u32 = 0x4000_0000;

const BPF_LD_W_ABS: u16 = 0x20;
const BPF_JEQ_K: u16 = 0x15;
const BPF_JSET_K: u16 = 0x45;
const BPF_RET_K: u16 = 0x06;

/// The offsets of `seccomp_data`: the call number, then the architecture.
const NR: u32 = 0;
const ARCH: u32 = 4;

/// The offsets of the low and the high half of the first argument of the call in `seccomp_data` (both architectures
/// are little endian): the flags of `clone`, the process of `kill`, `tgkill` and `prlimit64`.
const ARG0_LOW: u32 = 16;
const ARG0_HIGH: u32 = 20;

const RET_KILL_PROCESS: u32 = 0x8000_0000;
const RET_ALLOW: u32 = 0x7fff_0000;
const RET_EPERM: u32 = 0x0005_0000 | libc::EPERM as u32;
const RET_ENOSYS: u32 = 0x0005_0000 | libc::ENOSYS as u32;

/// The classic BPF program of the filter for the process `pid` (the helper's own):
///
/// - the wrong architecture kills the process;
/// - a call of [`DENIED`] (or any call of the x32 ABI) is refused with `EPERM`;
/// - `clone3` is answered `ENOSYS` (its flags are in memory the filter cannot read, and the C library then makes its
///   threads with `clone`), and `clone` is allowed only for a thread (`CLONE_THREAD`), so the helper can start no
///   process;
/// - `kill` and `tgkill` are allowed only towards the helper itself (`abort` and the C library's thread signals), and
///   `prlimit64` only for itself (process 0), so no other process of the user can be signalled or limited, which
///   Landlock stops only from its ABI 6;
/// - every other call is allowed.
///
/// `None` where [`AUDIT_ARCH`] is `None`.
fn program(pid: u32) -> Option<Vec<sock_filter>> {
    let arch = AUDIT_ARCH?;

    // The tests that end in `EPERM`, in order; each jumps forward to that refusal when it holds.
    let mut tests: Vec<(u16, u32)> = Vec::new();
    #[cfg(target_arch = "x86_64")]
    tests.push((BPF_JSET_K, X32_BIT));
    tests.extend(DENIED.iter().map(|&number| (BPF_JEQ_K, number as u32)));

    let instruction = |code: u16, jt: u8, jf: u8, k: u32| sock_filter { code, jt, jf, k };

    let mut program = vec![
        instruction(BPF_LD_W_ABS, 0, 0, ARCH),
        // The architecture is right: skip the kill that follows.
        instruction(BPF_JEQ_K, 1, 0, arch),
        instruction(BPF_RET_K, 0, 0, RET_KILL_PROCESS),
        instruction(BPF_LD_W_ABS, 0, 0, NR),
    ];

    // The tail below, numbered; a jump of n lands n instructions after the one that follows the jump. Each test of
    // the list jumps over the tests left after it and the tail up to its `EPERM` return.
    const TAIL_EPERM: usize = 14;
    for (at, &(code, k)) in tests.iter().enumerate() {
        let to_refusal = u8::try_from(tests.len() - at - 1 + TAIL_EPERM).ok()?;
        program.push(instruction(code, to_refusal, 0, k));
    }

    program.extend([
        // 0: `clone3` → `ENOSYS` (15).
        instruction(BPF_JEQ_K, 14, 0, libc::SYS_clone3 as u32),
        // 1: `clone` → its flags (2); else on to 4.
        instruction(BPF_JEQ_K, 0, 2, libc::SYS_clone as u32),
        // 2.
        instruction(BPF_LD_W_ABS, 0, 0, ARG0_LOW),
        // 3: a thread → allow (13), else `EPERM` (14).
        instruction(BPF_JSET_K, 9, 10, libc::CLONE_THREAD as u32),
        // 4: `kill` → its process (6); else on to 5.
        instruction(BPF_JEQ_K, 1, 0, libc::SYS_kill as u32),
        // 5: `tgkill` → its process (6); else on to 8.
        instruction(BPF_JEQ_K, 0, 2, libc::SYS_tgkill as u32),
        // 6.
        instruction(BPF_LD_W_ABS, 0, 0, ARG0_LOW),
        // 7: the helper itself → allow (13), else `EPERM` (14).
        instruction(BPF_JEQ_K, 5, 6, pid),
        // 8: `prlimit64` → its process (9); else allow (13).
        instruction(BPF_JEQ_K, 0, 4, libc::SYS_prlimit64 as u32),
        // 9.
        instruction(BPF_LD_W_ABS, 0, 0, ARG0_LOW),
        // 10: low half 0 → the high half (11), else `EPERM` (14).
        instruction(BPF_JEQ_K, 0, 3, 0),
        // 11.
        instruction(BPF_LD_W_ABS, 0, 0, ARG0_HIGH),
        // 12: process 0, itself → allow (13), else `EPERM` (14).
        instruction(BPF_JEQ_K, 0, 1, 0),
        // 13.
        instruction(BPF_RET_K, 0, 0, RET_ALLOW),
        // 14.
        instruction(BPF_RET_K, 0, 0, RET_EPERM),
        // 15.
        instruction(BPF_RET_K, 0, 0, RET_ENOSYS),
    ]);

    Some(program)
}

/// Whether this architecture has a seccomp filter (x86-64 and aarch64).
pub(super) fn has_filter() -> bool {
    AUDIT_ARCH.is_some()
}

/// Installs the seccomp filter of [`program`] (for this process) on every thread of the process (`SECCOMP_FILTER_FLAG_TSYNC`, so a thread
/// a library started earlier is held too), and for every thread started later. Needs `PR_SET_NO_NEW_PRIVS` to be set
/// already (`limit` does). `Unsupported` on an architecture without a filter.
pub(super) fn deny_syscalls() -> io::Result<()> {
    let Some(mut filters) = program(std::process::id()) else {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no seccomp filter for this architecture",
        ));
    };

    let program = sock_fprog {
        len: u16::try_from(filters.len()).map_err(|_| io::Error::other("the filter is too long"))?,
        filter: filters.as_mut_ptr(),
    };

    // SAFETY: `seccomp(SECCOMP_SET_MODE_FILTER, flags, args)` takes a pointer to a `sock_fprog`, which `program` is, and
    // the `len` instructions it points to (`filters`); both live through the call, which copies the program and keeps
    // nothing of the pointers.
    let result = unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_SET_MODE_FILTER,
            libc::SECCOMP_FILTER_FLAG_TSYNC,
            &raw const program,
        )
    };
    // With `TSYNC` a thread that cannot take the filter is reported by its id (a positive result): the filter is then on
    // no thread.
    match result {
        0 => Ok(()),
        -1 => Err(io::Error::last_os_error()),
        _ => Err(io::Error::other("a thread of the helper cannot take the filter")),
    }
}

/// Closes every descriptor from 3 up: what the app had open when it started the helper and did not mark close-on-exec
/// (`close_range`, Linux 5.9; before that by the list in `/proc/self/fd`, or without `/proc` every one up to the limit
/// on open files).
pub(super) fn close_inherited_fds() {
    // SAFETY: `close_range` takes two descriptors and flags by value and touches no memory; the helper keeps its three
    // standard streams and holds no other descriptor it needs at this point (it is the first thing it does).
    let closed = unsafe { libc::syscall(libc::SYS_close_range, 3_u32, u32::MAX, 0_u32) };
    if closed == 0 {
        return;
    }

    // The list is read to the end first, so the descriptor of the directory is closed before any is closed below.
    // Without `/proc` every descriptor up to the limit on open files is closed blindly (at most 65 536 calls).
    let open: Vec<libc::c_int> = match std::fs::read_dir("/proc/self/fd") {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
            .filter(|&fd| fd >= 3)
            .collect(),
        Err(_) => {
            let limit = rustix::process::getrlimit(rustix::process::Resource::Nofile)
                .current
                .unwrap_or(65_536)
                .min(65_536);

            (3..libc::c_int::try_from(limit).unwrap_or(65_536)).collect()
        }
    };

    for fd in open {
        // SAFETY: `close` takes the descriptor by value; one that is not open (the directory's own) only fails.
        unsafe { libc::close(fd) };
    }
}

/// The version of the Landlock ABI of the running kernel (1 or more), or `None` when it has none (older than Linux 5.13,
/// built without it or switched off).
pub(super) fn landlock_abi() -> Option<u32> {
    /// `LANDLOCK_CREATE_RULESET_VERSION`.
    const VERSION: u32 = 1;

    // SAFETY: with a NULL attribute, a size of 0 and this flag `landlock_create_ruleset` only returns the version; it
    // reads and writes no memory.
    let version = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<libc::c_void>(),
            0_usize,
            VERSION,
        )
    };

    u32::try_from(version).ok().filter(|&version| version >= 1)
}

/// The error number of an `io_uring_setup` call, for the tests that check the filter refuses it (`0` if it worked; the
/// ring it then made is closed at once).
#[cfg(test)]
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
#[cfg(test)]
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
