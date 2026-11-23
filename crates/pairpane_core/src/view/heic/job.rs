//! The Job Object that holds the HEIC helper process (and the thumbnail helper, with a smaller memory limit) on Windows.
//!
//! The parent creates a job and puts the helper in it right after the start, before a byte is sent to it, so the codec
//! has nothing to parse while it is not yet held. The job has three limits: closing the last handle to it ends every
//! process in it (so the helper cannot outlive the app, even when the app is killed: the system closes the handle), at
//! most the memory `contain` is given of committed memory for a process in it (a runaway allocation fails instead of growing), and one
//! active process, which is the helper itself (the codec cannot start another). Its UI restrictions also keep the
//! helper from the handles of other processes' windows, the clipboard, the desktop, the display settings, ending the
//! session, global atoms and system parameters.
//!
//! The helper is in the job a moment after it has started, not from its first instruction: it is created without
//! `CREATE_SUSPENDED`, which the standard library's `Command` cannot resume, and in that moment it only starts up and
//! waits for its standard input. Known limitations, not done: a low-integrity token for the helper (it runs with the
//! user's own integrity level, so the job's limits are what holds it), and a list of the handles it inherits
//! (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`); the standard library's `Command` offers neither.
#![allow(unsafe_code, reason = "Job Object limits; see `contain`")]
#![deny(clippy::undocumented_unsafe_blocks)]

use std::io;
use std::mem::size_of;
use std::process::Child;
use std::ptr;

use windows_sys::Win32::System::JobObjects::{
    JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD, JOBOBJECT_BASIC_UI_RESTRICTIONS,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};

use crate::process::job::Job;

/// The UI restrictions of the job (see the module).
const UI_LIMITS: u32 = JOB_OBJECT_UILIMIT_HANDLES
    | JOB_OBJECT_UILIMIT_READCLIPBOARD
    | JOB_OBJECT_UILIMIT_WRITECLIPBOARD
    | JOB_OBJECT_UILIMIT_DESKTOP
    | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
    | JOB_OBJECT_UILIMIT_EXITWINDOWS
    | JOB_OBJECT_UILIMIT_GLOBALATOMS
    | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS;

/// Creates the job with its limits, `memory` bytes of committed memory for a process in it (see `process::MEMORY` for the
/// HEIC helper, and `view::thumb` for the thumbnail helper's), and puts `child` in it. The job must be kept until the
/// child is reaped; dropping it ends every process still in it (the kill-on-close limit).
pub(super) fn contain(child: &Child, memory: usize) -> io::Result<Job> {
    let job = limited(memory)?;
    job.assign(child)?;

    Ok(job)
}

/// Creates the job with its limits and nothing in it: the thumbnail helper's, whose process is made suspended in its
/// container and put in the job before it runs its first instruction (`view::thumb::appcontainer`).
pub(in crate::view) fn limited(memory: usize) -> io::Result<Job> {
    let job = Job::new()?;

    // SAFETY: the structure holds only integers and has no invalid bit pattern, so all-zero is a valid value.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
    limits.BasicLimitInformation.ActiveProcessLimit = 1;
    limits.ProcessMemoryLimit = memory;

    // SAFETY: `job.handle()` is a live job handle, and the pointer and the length are those of `limits`, the structure
    // that the `JobObjectExtendedLimitInformation` class takes, which lives through the call.
    let set = unsafe {
        SetInformationJobObject(
            job.handle(),
            JobObjectExtendedLimitInformation,
            ptr::from_ref(&limits).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if set == 0 {
        return Err(io::Error::last_os_error());
    }

    let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
        UIRestrictionsClass: UI_LIMITS,
    };

    // SAFETY: `job.handle()` is a live job handle, and the pointer and the length are those of `ui`, the structure that
    // the `JobObjectBasicUIRestrictions` class takes, which lives through the call.
    let restricted = unsafe {
        SetInformationJobObject(
            job.handle(),
            JobObjectBasicUIRestrictions,
            ptr::from_ref(&ui).cast(),
            size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
        )
    };
    if restricted == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(job)
}
