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

use std::ffi::c_void;
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

use super::super::environment;
use super::container::{Sid, wide};
use super::step::at;

/// The variables the helper is given, which the loader, the C runtime and the system's own start of a process in a
/// container read. Windows adds to the block of a process in an AppContainer the folders of the container (it replaces the
/// temporary folder and the application data folders with them), which it can do only with the usual variables of a user
/// there, so a block of the system directory alone is refused ("the system could not find the environment option", error
/// 203). They are the paths of the machine and of the user's own folders, which the container cannot open; nothing of the
/// app's own (tokens, proxy settings, a session) is in the list.
const ENVIRONMENT: &[&str] = &[
    "SystemRoot",
    "WINDIR",
    "SystemDrive",
    "TEMP",
    "TMP",
    "LOCALAPPDATA",
    "APPDATA",
    "USERPROFILE",
    "ProgramData",
    "ComSpec",
    "PATHEXT",
];

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

/// Handles that are inheritable for as long as this value lives: made so when it is made, and made not so again when it is
/// dropped, on every path, which is before the lock that is held around it is let go.
struct Inheriting<'a>(&'a [&'a OwnedHandle]);

impl<'a> Inheriting<'a> {
    fn new(handles: &'a [&'a OwnedHandle]) -> io::Result<Self> {
        // The value exists before the first handle is changed, so that one that fails leaves none of them inheritable.
        let inheriting = Self(handles);
        for handle in handles {
            set_inheritable(handle, true)?;
        }

        Ok(inheriting)
    }
}

impl Drop for Inheriting<'_> {
    fn drop(&mut self) {
        for handle in self.0 {
            // A handle that cannot be told so is closed with the call that follows or the owner's drop, which is as good.
            let _ = set_inheritable(handle, false);
        }
    }
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

/// The environment block of the helper: the variables of [`ENVIRONMENT`] that the app has, as `CreateProcessW` takes them
/// (see `environment.rs`). A variable that is not Unicode is left out.
fn environment_block() -> Vec<u16> {
    let variables: Vec<(&str, String)> = ENVIRONMENT
        .iter()
        .filter_map(|name| Some((*name, std::env::var(name).ok()?)))
        .collect();

    environment::block(&variables)
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
    // The standard error is the null device, so the helper has a valid handle for it and writes nothing anywhere.
    let child_err = OwnedHandle::from(
        File::options()
            .write(true)
            .open("NUL")
            .map_err(|error| at("open NUL", error))?,
    );

    // What the attribute list points to, which lives until the process is made.
    let handles = [
        child_in.as_raw_handle(),
        child_out.as_raw_handle(),
        child_err.as_raw_handle(),
    ];
    let capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: container.as_ptr(),
        Capabilities: ptr::null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let mut attributes = Attributes::new(2)?;

    // SAFETY: `handles` is an array of three live handles and `capabilities` a `SECURITY_CAPABILITIES` that points to the
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
    startup.StartupInfo.hStdError = child_err.as_raw_handle();
    startup.lpAttributeList = attributes.list();
    let mut info = PROCESS_INFORMATION::default();

    // No start of a process by the app (see `process::spawn`) runs while the helper's ends of the pipes can be inherited. A
    // start by something else (Tauri's restart, the opener and the updater's `ShellExecute`, WebView2) can, for the few
    // microseconds between here and the end of the call; what it inherits is a pipe end or the null device, and the helper's
    // own `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` keeps the sandbox safe whatever else holds them. The guard of the handles is
    // dropped before the lock, whichever way this function leaves.
    let held = SPAWN_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let to_inherit = [&child_in, &child_out, &child_err];
    let inheriting = Inheriting::new(&to_inherit)?;

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
    drop(inheriting);
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
