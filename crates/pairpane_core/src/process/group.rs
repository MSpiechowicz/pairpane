//! A child that is ended with everything it started: for a program that may run another (`ssh -G` runs the commands of
//! a `Match exec`), so that a stop or a time out leaves nothing behind.
//!
//! On Unix the child leads a process group of its own, and the group is sent `SIGKILL`. On Windows the child is held in a
//! Job Object that has the kill-on-close limit, which is terminated; the child is in the job a moment after it has
//! started (`Command` cannot start it suspended), and if the job cannot be made the stop reaches the child only.

use std::io;
use std::process::{Child, Command, ExitStatus};

use super::{Started, spawn};

/// A started child and what it starts. A [`Group`] that is dropped ends the child and everything in its group, and reaps
/// it. Wait for it with the methods of [`Started`] (not through [`Group::child_mut`]): once the child has ended they end
/// the group, and only then reap the child, so a signal to the group cannot reach another one that took its number.
pub struct Group {
    child: Child,
    /// The child has been reaped, so its group number may be used by another process.
    #[cfg(unix)]
    reaped: bool,
    #[cfg(windows)]
    job: Option<super::job::Job>,
}

impl Group {
    /// Starts `command` (through [`spawn`]) as the leader of its group.
    pub fn start(command: &mut Command) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;

            command.process_group(0);
        }

        let child = spawn(command)?;

        // Without the job a stop ends the child only; that is logged, not refused.
        #[cfg(windows)]
        let job = super::job::Job::kill_on_close()
            .and_then(|job| job.assign(&child).map(|()| job))
            .inspect_err(|error| tracing::warn!(%error, "a child is not held in a job"))
            .ok();

        Ok(Self {
            child,
            #[cfg(unix)]
            reaped: false,
            #[cfg(windows)]
            job,
        })
    }

    /// The child, for its pipes. Not for waiting: see the type.
    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    /// Ends everything in the group that is still there.
    fn end_group(&self) {
        #[cfg(unix)]
        if !self.reaped {
            let group = rustix::process::Pid::from_child(&self.child);

            // An error is a group that has ended already.
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        }

        #[cfg(windows)]
        if let Some(job) = &self.job {
            let _ = job.terminate(1);
        }
    }
}

impl Started for Group {
    /// The exit status once the child has ended; the rest of its group is ended with it.
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        #[cfg(unix)]
        {
            use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};

            if !self.reaped {
                // Looked at, not reaped: until it is, the group number stays the child's.
                let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;

                if waitid(WaitId::Pid(Pid::from_child(&self.child)), options)?.is_none() {
                    return Ok(None);
                }

                self.end_group();
            }
        }

        let status = self.child.try_wait()?;

        #[cfg(windows)]
        if status.is_some() {
            self.end_group();
        }

        #[cfg(unix)]
        if status.is_some() {
            self.reaped = true;
        }

        Ok(status)
    }

    fn kill(&mut self) -> io::Result<()> {
        self.end_group();

        self.child.kill()
    }

    /// Waits for the child to end, ends the rest of its group, and reaps it (in that order: until it is reaped the group
    /// number is the child's, so the signal cannot reach another process).
    fn wait(&mut self) -> io::Result<ExitStatus> {
        #[cfg(unix)]
        if !self.reaped {
            use rustix::io::Errno;
            use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};

            // Waits for it to end without reaping it.
            loop {
                match waitid(
                    WaitId::Pid(Pid::from_child(&self.child)),
                    WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
                ) {
                    Err(Errno::INTR) => {}
                    // A child that cannot be looked at is reaped as it is.
                    Err(_) => break,
                    Ok(_) => {
                        self.end_group();

                        break;
                    }
                }
            }
        }

        let status = self.child.wait();

        #[cfg(unix)]
        {
            self.reaped = true;
        }

        #[cfg(windows)]
        self.end_group();

        status
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        let _ = Started::kill(self);
        let _ = Started::wait(self);
    }
}
