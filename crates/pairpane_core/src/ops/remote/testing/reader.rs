//! A file of the fake server open for reading.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::vfs::{RemoteMeta, RemoteReader};

use super::{Grow, Shared, describe};

/// A file of the server open for reading.
pub(super) struct Reader {
    file: File,
    disk: PathBuf,
    shared: Arc<Shared>,
    served: u64,
    started: bool,
    grown: bool,
}

impl Reader {
    pub(super) fn new(file: File, disk: PathBuf, shared: Arc<Shared>) -> Self {
        Self {
            file,
            disk,
            shared,
            served: 0,
            started: false,
            grown: false,
        }
    }

    fn grow(&self) {
        let mut file = OpenOptions::new().append(true).open(&self.disk).expect("the file");
        file.write_all(b"more").expect("append");
    }
}

impl RemoteReader for Reader {
    fn read(&mut self, buf: &mut [u8], cancel: &AtomicBool) -> io::Result<usize> {
        self.shared.biggest_read.fetch_max(buf.len(), Ordering::SeqCst);

        let hook = {
            let mut switches = self.shared.switches();
            assert!(!switches.panic_reads, "the server broke");

            if self.started {
                None
            } else {
                switches.at_first_read.take()
            }
        };
        self.started = true;
        if let Some(hook) = hook {
            hook();
        }

        let (hang, short, fail, kind, grow) = {
            let switches = self.shared.switches();

            (
                switches.hang_reads,
                switches.short_reads,
                switches.fail_after,
                switches.fail_kind.unwrap_or(io::ErrorKind::ConnectionAborted),
                switches.grow,
            )
        };

        if hang {
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                thread::sleep(Duration::from_millis(5));
            }
        }

        if fail.is_some_and(|limit| self.served >= limit) {
            return Err(kind.into());
        }

        let want = short.map_or(buf.len(), |most| most.min(buf.len()));
        let count = self.file.read(&mut buf[..want])?;
        self.served += count as u64;

        if count == 0 && grow == Grow::AtEnd && !self.grown {
            self.grown = true;
            self.grow();
        }

        Ok(count)
    }

    fn meta(&mut self) -> io::Result<RemoteMeta> {
        Ok(describe(&self.file.metadata()?, &self.shared.switches()))
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        if self.shared.switches().grow == Grow::OnClose {
            self.grow();
        }

        self.shared.handles.fetch_sub(1, Ordering::SeqCst);
    }
}
