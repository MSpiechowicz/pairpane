//! Copying and moving between a server and this computer, through the same checks as a copy on the disk.
//!
//! A download writes what the server sends to a temporary file next to its target, gives it the mode and time the server
//! states, and puts it in place with a rename that refuses to replace (see `down`): a target that exists is never
//! overwritten without the answer to its question, and one that is there is whole. A move removes an item from the
//! server only once its copy is in place and on the disk, and the item is seen unchanged (see `down`). It also checks,
//! right before each removal and again right after it, that the folder is still where the walk went and not a link put in
//! its place: each folder on the way is looked at by its own name, so a server that does not resolve links shows a swap too
//! (see `walk`).
//! Two items of the server that come to one name here are never written one over the other (see `own`).
//!
//! Nothing on a server is followed: a symbolic link, a device or any other item that is not a file or a folder is skipped
//! and counted. What a server calls its items is not trusted: a name this computer cannot use is skipped and counted
//! too. The connection is not trusted either: a call that cannot reach the server fails its item, and a server that
//! fails three in a row, or is gone, fails the job.

mod down;
mod down_file;
mod home;
mod own;
#[cfg(test)]
mod testing;
mod walk;

#[cfg(test)]
mod down_chain_tests;
#[cfg(test)]
mod down_move_tests;
#[cfg(test)]
mod down_safety_tests;
#[cfg(test)]
mod down_stop_tests;
#[cfg(all(test, unix))]
mod down_swap_tests;
#[cfg(test)]
mod down_tests;

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::listing::platform;
use crate::vfs::{RemoteReader, is_remote_name};

use super::report::refusal;
use super::transfer::{Run, Stop};
use super::{Direction, RemoteJob, name_fits};

impl Run<'_> {
    /// Runs the transfer between a server and this computer that `job` describes.
    pub(super) fn remote(&mut self, job: &RemoteJob) -> Result<(), Stop> {
        match job.direction {
            Direction::Download => self.download(job),
            Direction::Upload => Err(refusal(io::ErrorKind::InvalidInput, "uploads are not supported yet").into()),
        }
    }

    /// Where the item `name` of the server is written, in the folder `to`.
    fn local(&self, to: &Path, name: &str) -> PathBuf {
        if self.inner.hooks.fold_names {
            to.join(name.to_lowercase())
        } else {
            to.join(name)
        }
    }

    /// `error` from a call that waits on the server, as the step stops: a cancel, if that is why it was interrupted.
    fn interrupted(&self, error: io::Error) -> Stop {
        if error.kind() == io::ErrorKind::Interrupted && self.job.control.is_cancelled() {
            Stop::Cancelled
        } else {
            Stop::Failed(error)
        }
    }

    /// Reports `error` against the item `rel`, and the job goes on: unless that is a cancel, which ends it.
    fn report(&mut self, rel: &str, error: io::Error) -> Result<(), Stop> {
        match self.interrupted(error) {
            Stop::Cancelled => Err(Stop::Cancelled),
            Stop::Failed(error) => {
                self.fail(rel, &error);

                Ok(())
            }
        }
    }
}

/// Whether this computer can have a file or folder of this name, as the server calls it.
fn usable_here(name: &str) -> bool {
    is_remote_name(name) && platform::is_plain_name(name) && !platform::is_unsafe_name(name) && name_fits(name)
}

/// A file on a server as the copy loop reads it: what the server sends, until it says the end.
struct Source<'a> {
    reader: &'a mut dyn RemoteReader,
    cancel: &'a AtomicBool,
}

impl Read for Source<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.reader.read(buf, self.cancel) {
            Ok(count) if count > buf.len() => Err(refusal(io::ErrorKind::InvalidData, "the server sent too much")),
            // An interruption is a cancel: a server that interrupts a read for no reason would loop the copy for good.
            Err(error) if error.kind() == io::ErrorKind::Interrupted && !self.cancel.load(Ordering::Acquire) => {
                Err(refusal(io::ErrorKind::Other, "the server interrupted the read"))
            }
            other => other,
        }
    }
}
