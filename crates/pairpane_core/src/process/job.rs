//! A Job Object on Windows: the handle, a process put in it, and what ends it. The limits a job has are its user's
//! (the HEIC helper's in `view::heic::job`; the command line's has none).
#![allow(
    unsafe_code,
    reason = "Job Object calls; see `Job::new`, `Job::assign`, `Job::terminate` and `Job::drop`"
)]
#![deny(clippy::undocumented_unsafe_blocks)]

use std::io;
use std::mem::size_of;
use std::os::windows::io::AsRawHandle;
use std::process::Child;
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject,
};

/// A job. Dropping it closes the handle: what is in it then ends only when the job has the kill-on-close limit.
pub(crate) struct Job(HANDLE);

impl Job {
    /// An unnamed job with no limits.
    pub(crate) fn new() -> io::Result<Self> {
        // SAFETY: no security attributes and no name (both NULL, which the function documents) make an unnamed job that
        // only this handle reaches; the result is checked for NULL before it is used.
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }

        Ok(Self(handle))
    }

    /// An unnamed job with one limit: closing its last handle (dropping it, or the app ending) ends every process in it.
    pub(crate) fn kill_on_close() -> io::Result<Self> {
        let job = Self::new()?;

        // SAFETY: the structure holds only integers and has no invalid bit pattern, so all-zero is a valid value.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: `job.0` is a live job handle, and the pointer and the length are those of `limits`, the structure that
        // the `JobObjectExtendedLimitInformation` class takes, which lives through the call.
        let set = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                ptr::from_ref(&limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if set == 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(job)
    }

    /// The handle, for setting the job's limits. It lives as long as `self`.
    pub(crate) fn handle(&self) -> HANDLE {
        self.0
    }

    /// Puts `child` in the job.
    pub(crate) fn assign(&self, child: &Child) -> io::Result<()> {
        self.assign_handle(child.as_raw_handle())
    }

    /// Puts the process whose handle `process` is in the job. The handle must be live (and have the right to set quota
    /// and terminate, as the one `CreateProcessW` returns has).
    pub(crate) fn assign_handle(&self, process: HANDLE) -> io::Result<()> {
        // SAFETY: `self.0` is a live job handle and the caller gives a live process handle, which stays open through the
        // call; the call only reads both.
        let assigned = unsafe { AssignProcessToJobObject(self.0, process) };
        if assigned == 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    /// Ends every process in the job with the exit code `code`.
    pub(crate) fn terminate(&self, code: u32) -> io::Result<()> {
        // SAFETY: `self.0` is the live job handle this value owns; the call only ends the processes in the job.
        let ended = unsafe { TerminateJobObject(self.0, code) };
        if ended == 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: `self.0` is the handle `CreateJobObjectW` returned, owned by this value and closed here once.
        unsafe { CloseHandle(self.0) };
    }
}
