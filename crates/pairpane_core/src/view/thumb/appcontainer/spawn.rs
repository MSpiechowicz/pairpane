//! Starting the thumbnail helper in its AppContainer with `CreateProcessW`: stable Rust's `Command` cannot give a process
//! the `SECURITY_CAPABILITIES` attribute that puts it there (`raw_attribute` is nightly only).
//!
//! What the process gets: the container's token (low integrity, no access to the file system but what its access lists
//! name the container for, no network, none of the user's other rights), a standard input and output that are pipes with
//! one end each in the helper and none else of the app's handles (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`), no standard
//! error, no window, an environment of two variables, a working folder of the system's and no start: it is made
//! suspended, so that the caller can put it in its job before it runs its first instruction.
#![allow(
    unsafe_code,
    reason = "process creation with an attribute list and pipes; see each `unsafe` block"
)]
#![deny(clippy::undocumented_unsafe_blocks)]

use std::ffi::{OsString, c_void};
use std::fs::File;
use std::io;
use std::mem::{size_of, size_of_val};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::PoisonError;

use windows_sys::Win32::Foundation::{HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation};
use windows_sys::Win32::Security::SECURITY_CAPABILITIES;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, ResumeThread,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute,
};

use crate::process::SPAWN_LOCK;

use super::container::{Sid, wide};
use super::step::at;

/// The variables the helper is given, which the loader and the C runtime read (a Windows process needs its system
/// directory); it starts with no others, so nothing of the app's own (tokens, proxy settings) reaches it. Sorted.
const ENVIRONMENT: &[&str] = &["SystemRoot", "WINDIR"];

/// A helper that has been made and not yet started: its process, its main thread (suspended) and the ends of its pipes
/// that the app keeps.
pub(super) struct Created {
    pub(super) process: OwnedHandle,
    pub(super) thread: OwnedHandle,
    /// The write end of the helper's standard input.
    pub(super) stdin: File,
    /// The read end of the helper's standard output.
    pub(super) stdout: File,
}

/// Lets the helper whose main thread is `thread` run its first instruction.
pub(super) fn resume(thread: &OwnedHandle) -> io::Result<()> {
    // SAFETY: the handle is the live main thread of a process `create` made, with the right to suspend and resume it (the
    // one `CreateProcessW` returns has it).
    let before = unsafe { ResumeThread(thread.as_raw_handle()) };
    if before == u32::MAX {
        return Err(at("ResumeThread", io::Error::last_os_error()));
    }

    Ok(())
}

/// The storage of a process attribute list, which must have the alignment of a pointer. Deleted when dropped.
struct Attributes(Vec<usize>);

impl Attributes {
    /// An attribute list for `count` attributes.
    fn new(count: u32) -> io::Result<Self> {
        let mut size = 0_usize;

        // SAFETY: with a NULL list the call only reports in `size` how much storage a list of `count` attributes takes (it
        // fails with `ERROR_INSUFFICIENT_BUFFER`, which is not an error here: `size` is checked below).
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), count, 0, &raw mut size) };
        if size == 0 {
            return Err(at(
                "InitializeProcThreadAttributeList (size)",
                io::Error::last_os_error(),
            ));
        }

        let mut storage = vec![0_usize; size.div_ceil(size_of::<usize>())];

        // SAFETY: `storage` has `size` bytes at least, aligned for a pointer, and lives as long as the list (it is moved
        // into the value returned, which deletes the list before it frees the storage).
        let made = unsafe {
            InitializeProcThreadAttributeList(storage.as_mut_ptr().cast::<c_void>(), count, 0, &raw mut size)
        };
        if made == 0 {
            return Err(at("InitializeProcThreadAttributeList", io::Error::last_os_error()));
        }

        Ok(Self(storage))
    }

    fn list(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.0.as_mut_ptr().cast()
    }

    /// Sets the attribute `attribute` to the `size` bytes at `value`.
    ///
    /// # Safety
    ///
    /// `value` must point to `size` readable bytes of the kind `attribute` takes, which stay alive and unchanged until the
    /// list has been used by `CreateProcessW` (the list keeps the pointer, not a copy).
    unsafe fn set(&mut self, attribute: u32, value: *const c_void, size: usize) -> io::Result<()> {
        // SAFETY: the list is initialised (see `new`); the value is the caller's, as its contract says.
        let set = unsafe {
            UpdateProcThreadAttribute(
                self.list(),
                0,
                attribute as usize,
                value,
                size,
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if set == 0 {
            return Err(at("UpdateProcThreadAttribute", io::Error::last_os_error()));
        }

        Ok(())
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: the list was initialised in `new` and is deleted here once, before its storage is freed.
        unsafe { DeleteProcThreadAttributeList(self.list()) };
    }
}

/// An anonymous pipe: the read end and the write end, neither inheritable.
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write): (HANDLE, HANDLE) = (ptr::null_mut(), ptr::null_mut());

    // SAFETY: two places for the handles, no security attributes (NULL: the handles cannot be inherited and have the
    // default security) and the default size of the buffer.
    let made = unsafe { CreatePipe(&raw mut read, &raw mut write, ptr::null(), 0) };
    if made == 0 {
        return Err(at("CreatePipe", io::Error::last_os_error()));
    }

    // SAFETY: `CreatePipe` made the two handles, each owned by nothing else; they are closed by these values.
    Ok(unsafe { (OwnedHandle::from_raw_handle(read), OwnedHandle::from_raw_handle(write)) })
}

/// Makes the handle one that a process made with a list of handles to inherit can be given (`inherit`), or one that no
/// process inherits.
fn set_inheritable(handle: &OwnedHandle, inherit: bool) -> io::Result<()> {
    let flags = if inherit { HANDLE_FLAG_INHERIT } else { 0 };

    // SAFETY: the handle is live and owned by `handle`; only its inherit flag is set.
    let set = unsafe { SetHandleInformation(handle.as_raw_handle(), HANDLE_FLAG_INHERIT, flags) };
    if set == 0 {
        return Err(at("SetHandleInformation", io::Error::last_os_error()));
    }

    Ok(())
}

/// The command line `"program" arg arg`, as UTF-16 with its NUL. The arguments are the app's own fixed ones (a flag and a
/// number): one with a space or a quote in it is refused rather than quoted.
fn command_line(program: &Path, args: &[&str]) -> io::Result<Vec<u16>> {
    let mut line: Vec<u16> = vec![u16::from(b'"')];
    line.extend(program.as_os_str().encode_wide());
    line.push(u16::from(b'"'));

    for arg in args {
        if arg.is_empty() || arg.chars().any(|c| c.is_whitespace() || c == '"') {
            return Err(at(
                "command line",
                io::Error::other("an argument of the helper needs quoting"),
            ));
        }

        line.push(u16::from(b' '));
        line.extend(arg.encode_utf16());
    }
    line.push(0);

    Ok(line)
}

/// The environment block of the helper: `NAME=value` and a NUL for each of [`ENVIRONMENT`] the app has, and a NUL more.
fn environment_block() -> Vec<u16> {
    let mut block: Vec<u16> = Vec::new();

    for name in ENVIRONMENT {
        if let Some(value) = std::env::var_os(name) {
            let mut entry = OsString::from(name);
            entry.push("=");
            entry.push(value);

            block.extend(entry.encode_wide());
            block.push(0);
        }
    }

    // The block ends with an empty string; one with no variable is two NULs.
    block.push(0);
    if block.len() == 1 {
        block.push(0);
    }

    block
}

/// The system's folder, where the helper starts: the app's own working folder is none that its container can reach.
fn system_directory() -> Option<Vec<u16>> {
    let root = std::env::var_os("SystemRoot")?;

    Some(wide(PathBuf::from(root).join("System32").as_os_str()))
}

/// Makes the helper `program` with `args` in the container `container`, suspended.
pub(super) fn create(program: &Path, args: &[&str], container: &Sid) -> io::Result<Created> {
    let (child_in, parent_in) = pipe()?;
    let (parent_out, child_out) = pipe()?;

    // What the attribute list points to, which lives until the process is made.
    let handles = [child_in.as_raw_handle(), child_out.as_raw_handle()];
    let capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: container.as_ptr(),
        Capabilities: ptr::null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let mut attributes = Attributes::new(2)?;

    // SAFETY: `handles` is an array of two live handles and `capabilities` a `SECURITY_CAPABILITIES` that points to the
    // container's SID, which `container` keeps alive; both live until `CreateProcessW` has returned below.
    unsafe {
        attributes.set(
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            handles.as_ptr().cast(),
            size_of_val(&handles),
        )?;
        attributes.set(
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
            ptr::from_ref(&capabilities).cast(),
            size_of::<SECURITY_CAPABILITIES>(),
        )?;
    }

    let application = wide(program.as_os_str());
    let mut command = command_line(program, args)?;
    let environment = environment_block();
    let directory = system_directory();

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = child_in.as_raw_handle();
    startup.StartupInfo.hStdOutput = child_out.as_raw_handle();
    // No standard error: the helper writes nothing there.
    startup.StartupInfo.hStdError = ptr::null_mut();
    startup.lpAttributeList = attributes.list();
    let mut info = PROCESS_INFORMATION::default();

    // Nothing else of the app starts a process while the helper's ends of the pipes can be inherited (`SPAWN_LOCK`), and
    // they can be only from here to the end of the call: the window of a start by a plugin of the shell is that call.
    let held = SPAWN_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    set_inheritable(&child_in, true)?;
    set_inheritable(&child_out, true)?;

    // SAFETY: the program and the command line are NUL-terminated and live through the call (the command line is the
    // mutable buffer the function wants); no security attributes for the process and its thread; the handles are
    // inherited, which only the two of the attribute list are; the environment is a Unicode block of NUL-ended entries
    // that ends with an empty one; the folder is NUL-terminated or NULL; `startup` is a `STARTUPINFOEXW` whose size is in
    // its `cb` and whose list is alive, and a pointer to it as a `STARTUPINFOW` is what the function takes with the
    // extended flag; `info` is a place for the process and thread handles, set on success.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            directory.as_ref().map_or(ptr::null(), |directory| directory.as_ptr()),
            ptr::from_ref(&startup).cast(),
            &raw mut info,
        )
    };
    let failure = (created == 0).then(io::Error::last_os_error);

    // The pipes are the helper's from here: no process that is started later inherits them, whether the call worked or not.
    let _ = set_inheritable(&child_in, false);
    let _ = set_inheritable(&child_out, false);
    drop(held);

    if let Some(error) = failure {
        return Err(at("CreateProcessW", error));
    }

    // SAFETY: `CreateProcessW` made the two handles, each owned by nothing else; they are closed by these values.
    let (process, thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(info.hProcess),
            OwnedHandle::from_raw_handle(info.hThread),
        )
    };

    // The helper's ends of the pipes are closed here, as the function ends, so that the app sees the end of the helper's
    // answer when the helper does.
    Ok(Created {
        process,
        thread,
        stdin: File::from(parent_in),
        stdout: File::from(parent_out),
    })
}
