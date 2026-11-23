//! The confinement of the helper process on Linux, done by the helper itself (the parent cannot do it before the exec).
//! The thumbnail helper (`view::thumb`) takes the same three steps with a smaller address space, loads no codec first and
//! has an allow-list seccomp filter instead of the deny-list that the HEIC helper's codecs need ([`Filter`]).
//!
//! Done in three steps (see [`child_main`](super::child_main)):
//!
//! 1. [`close_inherited_fds`], the very first thing: every descriptor from 3 up is closed, so nothing the app had open
//!    (and did not mark close-on-exec) reaches the codecs. The parent also starts the helper with a cleared environment
//!    (see `process::command`).
//! 2. [`limit`]: the helper dies with its parent (`PR_SET_PDEATHSIG`), can never gain privileges
//!    (`PR_SET_NO_NEW_PRIVS`) and is held to limits on core dumps, CPU time and open files.
//! 3. [`confine`], after the codecs are loaded (libheif opens its plugins when it starts, which needs the file system; a
//!    libheif before 1.13 does so on its first context, which the loader makes then) and before the picture is read: a
//!    limit on the address space, then Landlock with every file system right and, where the kernel has them, every
//!    network right and the scopes for abstract sockets and signals handled and none allowed, and last a seccomp filter
//!    (`kernel.rs`, on every thread at once) that refuses with `EPERM` every call to make or use a socket (io_uring
//!    included), to start a program or a process (threads stay allowed), to reach into, signal or limit another
//!    process, System V IPC and message queues, and BPF, perf events, the key ring, `userfaultfd` and namespaces; a call
//!    of the x32 ABI is refused too, and one of another architecture (32-bit x86 or Arm) kills the helper. The filter
//!    is required: where it cannot be installed the helper exits without reading the picture. Landlock holds only the
//!    thread that sets it, so the helper also refuses to go on if it has more than one thread at that point.
//!
//! What is stopped, by what, and on which kernel:
//!
//! - Opening a file or a directory, writing, creating, renaming and deleting: Landlock, from Linux 5.13 (ABI 1; the
//!   later rights, such as truncating, come with the later ABIs and are handled where the kernel knows them).
//! - Connecting a TCP socket or binding a port: Landlock from Linux 6.7 (ABI 4). Connecting to a path-named Unix socket:
//!   Landlock from ABI 9 (`AccessFs::ResolveUnix`, handled here as part of every file system right, so it is stopped
//!   wherever the kernel has it); an abstract one: the scope, from ABI 6 (Linux 6.12). UDP is never stopped by Landlock.
//! - Making or using any socket at all, and so every connection above on every kernel, UDP included: the seccomp
//!   filter, which needs no Landlock (io_uring, which makes and connects sockets by other means, is refused with them).
//!   A socket made before the filter would not be stopped by it, but the helper has none (step 1).
//! - Starting a program or another process, looking into, writing to, signalling, limiting or taking descriptors from
//!   another process, and System V shared memory: the seccomp filter (signals also by the Landlock scope, ABI 6); the
//!   helper may signal itself. Without Landlock, and where Yama's `ptrace_scope` is 0, `/proc/<pid>/mem` of another
//!   process of the user is a file like any other: that is part of "can write the user's files" below.
//!
//! Best effort by design, and it is the kernel that decides how much applies: a kernel without Landlock (older than 5.13,
//! or with it switched off) runs the helper with the limits, `PR_SET_NO_NEW_PRIVS` and the seccomp filter only (the
//! parent logs that once, [`note_kernel`]), so the files stay readable by it; one with an older ABI enforces the rights
//! it knows. On an architecture other than x86-64 and aarch64 there is no seccomp filter. The HEIC helper has no allow-list
//! seccomp filter (the calls of a codec differ between libheif's plugins and distributions; the thumbnail helper has one,
//! which allows the dozen calls it makes and refuses the rest, `PROT_EXEC` mappings included). Not done: a namespace of its own
//! (needs privileges or user namespaces, which distributions switch off).
//!
//! The parent's death is seen by the kernel only if it is the thread that started the helper that ends: the decode runs
//! on one blocking thread from the start of the helper to its reaping, so that thread is there throughout.

use std::io;
use std::sync::Once;

use landlock::{ABI, Access, AccessFs, AccessNet, Ruleset, RulesetAttr, RulesetStatus, Scope};
use rustix::process::{Resource, Signal, getppid};
use rustix::thread::set_no_new_privs;

use crate::process::lower_limit;

use super::kernel;

pub(in crate::view) use super::kernel::Filter;

/// The most CPU time of the helper, over all its threads. A 64-megapixel picture takes a few seconds of it.
const CPU_SECONDS: u64 = 60;

/// The most open files. The helper has its three standard streams and opens nothing else (after Landlock it cannot).
const MAX_FILES: u64 = 64;

/// How much address space the helper may map above what it has mapped when its codecs are loaded. A picture of the
/// pixel cap needs the input (64 MiB), the planes of the decoder (up to 192 MiB for 16 bits per channel), the converted
/// RGBA (256 MiB, up to 512 MiB in a block of the decoder) and the answer (256 MiB), and the decoder's threads reserve
/// 64 MiB of arena each: well under 2 GiB. 3 GiB is that and room to spare; a library that wants a lot more than a
/// picture of the cap needs is stopped (its allocation fails) instead of growing without end.
pub(in crate::view) const ADDRESS_SPACE: u64 = 3 * 1024 * 1024 * 1024;

/// The address space of the thumbnail helper above what it has mapped when it starts: the input (64 MiB), the decoded
/// picture (at most 256 MiB, see `view::thumb`) and what the decoder keeps beside it (PNG is held to a 512 MiB allocation
/// limit; the other decoders keep a few planes of the picture, which the limit on the picture's size bounds), which is
/// under 1 GiB; 1.5 GiB is that and room to spare. The same room is the memory limit of the Windows job.
pub(in crate::view) const THUMBNAIL_ADDRESS_SPACE: u64 = 1536 * 1024 * 1024;

/// Step 1 (see the module). Whatever cannot be closed stays open.
pub(in crate::view) fn close_inherited_fds() {
    kernel::close_inherited_fds();
}

/// Logs once per run, in the parent (the helper has no log), when this kernel has no Landlock, so the helper's files are
/// not closed to it (see the module for what then still holds).
pub(in crate::view) fn note_kernel() {
    static NOTED: Once = Once::new();

    NOTED.call_once(|| {
        if kernel::landlock_abi().is_none() {
            tracing::warn!(
                "this kernel has no Landlock: the heic helper runs with limits and a seccomp filter only, and can read, \
                 write and create the user's files"
            );
        }
        if !kernel::has_filter() {
            tracing::warn!("no seccomp filter for this architecture: the heic helper runs without one");
        }
    });
}

/// Step 2 (see the module). Fails when the process cannot be tied to its parent or cannot be limited; the helper then
/// exits and the parent counts it as a file that cannot be decoded.
pub(in crate::view) fn limit() -> io::Result<()> {
    let parent = getppid();
    set_parent_process_death_signal(Signal::KILL)?;

    // The parent may have ended between the exec and the call above: the signal is then never sent.
    if getppid() != parent {
        return Err(io::Error::other("the parent of the helper has ended"));
    }

    set_no_new_privs(true)?;
    for (resource, value) in [
        (Resource::Core, 0),
        (Resource::Cpu, CPU_SECONDS),
        (Resource::Nofile, MAX_FILES),
    ] {
        lower_limit(resource, value)?;
    }

    Ok(())
}

/// Step 3 (see the module), with the address space limited to `address_space` bytes above what is mapped now and the
/// seccomp filter `filter`. The limit and Landlock are best effort (what the kernel cannot do is left undone); the
/// seccomp filter is not: where this architecture has one and it cannot be installed, this fails and the helper exits
/// without reading the picture.
pub(in crate::view) fn confine(address_space: u64, filter: Filter) -> io::Result<()> {
    if let Some(mapped) = mapped_bytes()
        && let Err(error) = lower_limit(Resource::As, mapped.saturating_add(address_space))
    {
        tracing::debug!(%error, "the address space of the helper is not limited");
    }

    match deny_all() {
        Ok(RulesetStatus::FullyEnforced) => {}
        Ok(status) => tracing::debug!(?status, "the helper is only partly confined by Landlock"),
        Err(error) => tracing::debug!(%error, "the helper is not confined by Landlock"),
    }

    // The filter is the last thing set, so nothing above needs a call it refuses.
    match kernel::deny_syscalls(filter) {
        Ok(()) => Ok(()),
        // No filter for this architecture: the parent says so once (`note_kernel`).
        Err(error) if error.kind() == io::ErrorKind::Unsupported => Ok(()),
        Err(error) => Err(error),
    }
}

/// Fails unless the process has one thread: Landlock holds only the thread that sets it (the seccomp filter takes every
/// thread), so a thread a plugin started before [`confine`] would be outside it. The helper calls this right before it.
pub(in crate::view) fn single_threaded() -> io::Result<()> {
    // Unreadable `/proc` cannot show it: that fails too.
    let threads = std::fs::read_dir("/proc/self/task")?.count();
    if threads != 1 {
        return Err(io::Error::other(
            "the helper has more than one thread before it is confined",
        ));
    }

    Ok(())
}

/// `PR_SET_PDEATHSIG` with `signal`.
fn set_parent_process_death_signal(signal: Signal) -> io::Result<()> {
    Ok(rustix::process::set_parent_process_death_signal(Some(signal))?)
}

/// The size of the address space the process has mapped now (`VmSize` of `/proc/self/status`), in bytes.
fn mapped_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find_map(|line| line.strip_prefix("VmSize:"))?;
    let kib: u64 = line.trim().strip_suffix("kB")?.trim().parse().ok()?;

    kib.checked_mul(1024)
}

/// Landlock with everything handled and nothing allowed, for the calling thread and every thread it starts later (the
/// helper decodes on its main thread, and the codecs' own threads are started from it after this).
fn deny_all() -> Result<RulesetStatus, landlock::RulesetError> {
    // The newest ABI the crate knows (it includes `AccessFs::ResolveUnix` from ABI 9): a kernel that has less handles what
    // it knows (best effort, the crate's default).
    let abi = ABI::V10;

    let status = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))?
        .handle_access(AccessNet::from_all(abi))?
        .scope(Scope::AbstractUnixSocket | Scope::Signal)?
        .create()?
        .restrict_self()?;

    Ok(status.ruleset)
}
