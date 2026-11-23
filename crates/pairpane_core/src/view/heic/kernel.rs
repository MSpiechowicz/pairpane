//! The raw Linux system calls of the helper's confinement (`sandbox.rs` says what it is for): the seccomp filters, the
//! closing of the descriptors the helper inherits and the probe of the Landlock version. Each is a few calls with no
//! state, and none of them has a safe wrapper in a crate the app already uses.
#![allow(
    unsafe_code,
    reason = "raw system calls (seccomp, close_range, Landlock's version); see each `unsafe` block"
)]
#![deny(clippy::undocumented_unsafe_blocks)]

use std::io;

use libc::{c_long, sock_filter, sock_fprog};

use super::writes::{HIGHEST_REVIEWED, IOCTL_PRELUDE, ioctl_prelude, open_prelude, refused_writes};

/// `AUDIT_ARCH_*` of the architecture this is built for, which the filter requires of every call (a program of another
/// ABI that the kernel runs, the 32-bit one on x86-64, has other call numbers). `None` where there is no filter.
#[cfg(target_arch = "x86_64")]
pub(super) const AUDIT_ARCH: Option<u32> = Some(0xC000_003E);
#[cfg(target_arch = "aarch64")]
pub(super) const AUDIT_ARCH: Option<u32> = Some(0xC000_00B7);
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub(super) const AUDIT_ARCH: Option<u32> = None;

/// Which seccomp filter a helper takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Filter {
    /// Refuse the calls of [`DENIED`] and allow the rest: for the HEIC helper, whose codecs (and their plugins) make calls
    /// that differ between libheif's builds and distributions.
    DenyList,
    /// Allow only the calls of [`ALLOWED`] and refuse the rest: for the thumbnail helper, which, once it is confined, only
    /// reads its input, allocates, computes and writes its answer.
    AllowList,
    /// Refuse the calls of [`DENIED`] and of `WRITES`, and an open of a file to write, create or truncate it (`openat` and
    /// `open` with such flags; `openat2` is answered `ENOSYS`, as its flags are in memory the filter cannot read, and the C
    /// library goes on with `openat`), an `ioctl` but for a few questions, and every call numbered above the highest that was
    /// reviewed, and allow the rest: for the helper that reads a repository (`gitstatus`), which reads files in many ways and
    /// writes none, so that where Landlock is not there the filter is still a read-only file system.
    ReadOnly,
}

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

/// The calls the allow-list filter lets through, which are those of the Rust runtime and the C library's allocator and
/// exit once the thumbnail helper is confined (measured, with a margin for the ones that other versions of the libraries
/// make): reading the input and writing the answer (`readv` and `writev` too), the memory (`brk`, `mmap` and `mprotect`
/// without `PROT_EXEC`, `munmap`, `mremap`, `madvise`), a lock that is not free (`futex`) and a yield, the random keys of
/// a hash map, the clock, the process and thread number (the C library's `abort` and exit ask), the alternate stack of
/// the signal handlers that the runtime removes when it ends, the return from one, and the end of the process or of its
/// one thread. Nothing opens, creates, signals, maps a file (`mmap` is allowed for an anonymous mapping only) or starts
/// anything.
const ALLOWED: &[c_long] = &[
    libc::SYS_read,
    libc::SYS_readv,
    libc::SYS_write,
    libc::SYS_writev,
    libc::SYS_close,
    libc::SYS_brk,
    libc::SYS_munmap,
    libc::SYS_mremap,
    libc::SYS_madvise,
    libc::SYS_futex,
    libc::SYS_sched_yield,
    libc::SYS_getrandom,
    libc::SYS_clock_gettime,
    libc::SYS_getpid,
    libc::SYS_gettid,
    libc::SYS_sigaltstack,
    libc::SYS_rt_sigreturn,
    libc::SYS_restart_syscall,
    libc::SYS_exit,
    libc::SYS_exit_group,
];

/// The bit of the number of a call of the x32 ABI on x86-64, which the kernel runs under the same `AUDIT_ARCH` and which
/// has other numbers for the same calls.
#[cfg(target_arch = "x86_64")]
const X32_BIT: u32 = 0x4000_0000;

pub(super) const BPF_LD_W_ABS: u16 = 0x20;
pub(super) const BPF_JEQ_K: u16 = 0x15;
pub(super) const BPF_JGT_K: u16 = 0x25;
pub(super) const BPF_JSET_K: u16 = 0x45;
pub(super) const BPF_RET_K: u16 = 0x06;

/// The offsets of `seccomp_data`: the call number, then the architecture.
const NR: u32 = 0;
const ARCH: u32 = 4;

/// The offsets of the low and the high half of the first argument of the call in `seccomp_data` (both architectures
/// are little endian): the flags of `clone`, the process of `kill`, `tgkill` and `prlimit64`.
const ARG0_LOW: u32 = 16;
const ARG0_HIGH: u32 = 20;

/// The offset of the low half of the second argument of the call: the flags of `open`.
#[cfg_attr(
    not(target_arch = "x86_64"),
    allow(dead_code, reason = "aarch64 has no `open`, only `openat`")
)]
pub(super) const ARG1_LOW: u32 = 24;

/// The offset of the low half of the third argument of the call: the protection of `mmap` and `mprotect`.
pub(super) const ARG2_LOW: u32 = 32;

/// The offset of the low half of the fifth argument of the call: the descriptor of `mmap`.
const ARG4_LOW: u32 = 48;

const RET_KILL_PROCESS: u32 = 0x8000_0000;
pub(super) const RET_ALLOW: u32 = 0x7fff_0000;
pub(super) const RET_EPERM: u32 = 0x0005_0000 | libc::EPERM as u32;
const RET_ENOSYS: u32 = 0x0005_0000 | libc::ENOSYS as u32;

/// One instruction of a classic BPF program.
pub(super) fn instruction(code: u16, jt: u8, jf: u8, k: u32) -> sock_filter {
    sock_filter { code, jt, jf, k }
}

/// The start of both filters: the wrong architecture kills the process, and then the call number is loaded.
fn header(arch: u32) -> Vec<sock_filter> {
    vec![
        instruction(BPF_LD_W_ABS, 0, 0, ARCH),
        // The architecture is right: skip the kill that follows.
        instruction(BPF_JEQ_K, 1, 0, arch),
        instruction(BPF_RET_K, 0, 0, RET_KILL_PROCESS),
        instruction(BPF_LD_W_ABS, 0, 0, NR),
    ]
}

/// The places in the tail of [`program`] that a jump goes to: its `RET_ALLOW`, `RET_EPERM` and `RET_ENOSYS`.
pub(super) const TAIL_ALLOW: usize = 13;
pub(super) const TAIL_EPERM: usize = 14;
pub(super) const TAIL_ENOSYS: usize = 15;

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
///
/// With `read_only` ([`Filter::ReadOnly`]) the calls of `WRITES` are refused too, and an open that writes, creates or
/// truncates (see [`open_prelude`]), an `ioctl` that is not one of a few questions (see [`ioctl_prelude`]), and every call
/// numbered above [`HIGHEST_REVIEWED`].
pub(super) fn program(pid: u32, read_only: bool) -> Option<Vec<sock_filter>> {
    let arch = AUDIT_ARCH?;

    // The tests that end in `EPERM`, in order; each jumps forward to that refusal when it holds.
    let mut tests: Vec<(u16, u32)> = Vec::new();
    #[cfg(target_arch = "x86_64")]
    tests.push((BPF_JSET_K, X32_BIT));
    tests.extend(DENIED.iter().map(|&number| (BPF_JEQ_K, number as u32)));
    if read_only {
        // Whatever is numbered above the highest call that was reviewed is refused, and then the calls that write.
        tests.push((BPF_JGT_K, HIGHEST_REVIEWED));
        tests.extend(refused_writes().map(|number| (BPF_JEQ_K, number)));
    }

    let mut program = header(arch);
    if read_only {
        program.extend(open_prelude(tests.len() + IOCTL_PRELUDE)?);
        program.extend(ioctl_prelude(tests.len())?);
    }

    // The tail below, numbered; a jump of n lands n instructions after the one that follows the jump. Each test of
    // the list jumps over the tests left after it and the tail up to its `EPERM` return.
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

/// The classic BPF program of [`Filter::AllowList`]:
///
/// - the wrong architecture kills the process;
/// - a call of [`ALLOWED`] is allowed, and `mmap` only for an anonymous mapping (the descriptor is -1: no file is
///   mapped) and `mmap` and `mprotect` only unless they ask for `PROT_EXEC`, so the process can map no code (a call of
///   the x32 ABI has a number that is none of these);
/// - every other call is refused with `EPERM`.
///
/// `None` where [`AUDIT_ARCH`] is `None`.
fn allow_program() -> Option<Vec<sock_filter>> {
    let arch = AUDIT_ARCH?;
    let mut program = header(arch);

    // The tail below, numbered; each test of the list jumps over the tests left after it and the tail up to its allow
    // (the last instruction, `TAIL_ALLOW`).
    const TAIL_ALLOW: usize = 7;
    for (at, &number) in ALLOWED.iter().enumerate() {
        let to_allow = u8::try_from(ALLOWED.len() - at - 1 + TAIL_ALLOW).ok()?;
        program.push(instruction(BPF_JEQ_K, to_allow, 0, number as u32));
    }

    program.extend([
        // 0: `mmap` → its descriptor (2); else on to 1.
        instruction(BPF_JEQ_K, 1, 0, libc::SYS_mmap as u32),
        // 1: `mprotect` → its protection (4); else `EPERM` (6).
        instruction(BPF_JEQ_K, 2, 4, libc::SYS_mprotect as u32),
        // 2.
        instruction(BPF_LD_W_ABS, 0, 0, ARG4_LOW),
        // 3: no file (the descriptor is -1: an anonymous mapping) → its protection (4), else `EPERM` (6).
        instruction(BPF_JEQ_K, 0, 2, u32::MAX),
        // 4.
        instruction(BPF_LD_W_ABS, 0, 0, ARG2_LOW),
        // 5: asks for `PROT_EXEC` → `EPERM` (6), else allow (7).
        instruction(BPF_JSET_K, 0, 1, libc::PROT_EXEC as u32),
        // 6.
        instruction(BPF_RET_K, 0, 0, RET_EPERM),
        // 7.
        instruction(BPF_RET_K, 0, 0, RET_ALLOW),
    ]);

    Some(program)
}

/// Whether this architecture has a seccomp filter (x86-64 and aarch64).
pub(super) fn has_filter() -> bool {
    AUDIT_ARCH.is_some()
}

/// Installs the seccomp filter `filter` (for this process) on every thread of the process (`SECCOMP_FILTER_FLAG_TSYNC`,
/// so a thread a library started earlier is held too), and for every thread started later. Needs `PR_SET_NO_NEW_PRIVS`
/// to be set already (`limit` does). `Unsupported` on an architecture without a filter.
pub(super) fn deny_syscalls(filter: Filter) -> io::Result<()> {
    let program = match filter {
        Filter::DenyList => program(std::process::id(), false),
        Filter::ReadOnly => program(std::process::id(), true),
        Filter::AllowList => allow_program(),
    };
    let Some(mut filters) = program else {
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
