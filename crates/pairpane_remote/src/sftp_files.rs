//! A file of a server opened for reading: [`RemoteReader`] over one SFTP handle.
//!
//! One SFTP read asks for at most [`CHUNK`] bytes and costs a round trip, so a read that waits for each reply would be as
//! fast as the latency allows. [`SftpReader`] asks for a window of consecutive chunks at once (at most [`WINDOW`]) and
//! uses the replies in order, up to the first one that is short: a server may answer with less than was asked, and the
//! chunks after it are of no use (the next window starts where the data stopped). So that a server that answers with less
//! than it is asked for does not make most of every window useless, the next window asks for what that server sent. The
//! size is tried again, doubled, only after [`TRUST`] windows in a row were answered in full, and by a window of a few
//! reads: a server with a cap of its own then costs a few reads now and then, not a window of useless ones. A reply of the
//! end of the file ends the reader for good.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use pairpane_core::vfs::{RemoteMeta, RemoteReader};
use russh_sftp::client::RawSftpSession;
use russh_sftp::protocol::SftpHandle;
use tokio::task::JoinSet;

use crate::error::{cancelled, closed, fail, from_sftp};
use crate::link::Link;
use crate::runtime::{call, call_until, limited};
use crate::sftp::meta_of;
use crate::site::{OwnedLease, Site};

/// The most bytes one SFTP read asks for: what servers (OpenSSH among them) answer in full.
const CHUNK: usize = 32 * 1024;

/// The least one SFTP read asks for, however little a server answers with.
const MIN_CHUNK: usize = 512;

/// The most reads in flight at once.
const WINDOW: usize = 16;

/// Windows in a row answered in full, after which a larger size is tried again.
const TRUST: u32 = 4;

/// The reads of a window that tries a larger size.
const TRY: usize = 2;

/// An open file; dropping it closes the server's handle in the background.
pub(crate) struct SftpReader {
    link: Arc<Link>,
    site: Arc<Site>,
    handle: SftpHandle,
    /// Held until the handle is closed: the connection is not closed for being idle meanwhile.
    lease: Option<OwnedLease>,
    /// What each read of the next window asks for: [`CHUNK`], or less for a server that answers with less.
    chunk: usize,
    /// Windows in a row that were answered in full, at this size.
    full: u32,
    /// Where in the file `ready` ends: the next window starts here.
    offset: u64,
    /// What the last window brought, of which the first `taken` bytes were given out.
    ready: Vec<u8>,
    taken: usize,
    ended: bool,
}

impl SftpReader {
    pub(crate) fn new(link: Arc<Link>, site: Arc<Site>, handle: SftpHandle, lease: OwnedLease) -> Self {
        Self {
            link,
            site,
            handle,
            lease: Some(lease),
            chunk: CHUNK,
            full: 0,
            offset: 0,
            ready: Vec::new(),
            taken: 0,
            ended: false,
        }
    }

    /// Fetches the next window, for a caller that wants `wanted` bytes. Leaves the reader as it was when it fails.
    fn fill(&mut self, wanted: usize, cancel: &AtomicBool) -> io::Result<()> {
        // A server that answered with less is trusted with more only once it has kept up for a while.
        let trying = self.chunk < CHUNK && self.full >= TRUST;
        let size = if trying {
            (self.chunk * 2).min(CHUNK)
        } else {
            self.chunk
        };
        let chunks = wanted.div_ceil(size).clamp(1, if trying { TRY } else { WINDOW });
        let window = fetch(
            Arc::clone(&self.link.sftp),
            self.handle.clone(),
            (self.offset, size, chunks),
            self.site.config.timeouts.request,
        );
        let replies = call_until(&self.site.runtime, window, cancel)?;
        let mut ready = Vec::new();
        let mut ended = false;
        let mut short = None;

        for reply in replies {
            match reply {
                Ok(data) if data.is_empty() && ready.is_empty() => {
                    return Err(fail(io::ErrorKind::InvalidData, "the server sent something unexpected"));
                }
                Ok(data) => {
                    // A server that sends more than was asked for is not believed.
                    let kept = data.len().min(size);

                    ready.extend_from_slice(&data[..kept]);

                    if kept < size {
                        short = Some(kept);

                        break;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                    ended = true;

                    break;
                }
                Err(error) if ready.is_empty() => return Err(error),
                // What came first is given out; the next window meets the same failure at its start.
                Err(_) => break,
            }
        }

        match short {
            Some(kept) => {
                self.chunk = if kept > 0 { kept.max(MIN_CHUNK) } else { size };
                self.full = 0;
            }
            None if trying => {
                self.chunk = size;
                self.full = 0;
            }
            None => self.full += 1,
        }
        self.offset += ready.len() as u64;
        self.ready = ready;
        self.taken = 0;
        self.ended = ended;

        Ok(())
    }

    /// A connection that was lost is forgotten, so that the next call on the site opens a new one.
    fn failed(&self, error: io::Error) -> io::Error {
        if error.kind() == io::ErrorKind::ConnectionAborted {
            self.site.forget(&self.link);
        }

        error
    }
}

impl RemoteReader for SftpReader {
    fn read(&mut self, buf: &mut [u8], cancel: &AtomicBool) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        if cancel.load(Ordering::Acquire) {
            return Err(cancelled());
        }

        if self.taken == self.ready.len() {
            if self.ended {
                return Ok(0);
            }

            self.fill(buf.len(), cancel).map_err(|error| self.failed(error))?;

            if self.ready.is_empty() {
                return Ok(0);
            }
        }

        let count = buf.len().min(self.ready.len() - self.taken);

        buf[..count].copy_from_slice(&self.ready[self.taken..self.taken + count]);
        self.taken += count;

        Ok(count)
    }

    fn meta(&mut self) -> io::Result<RemoteMeta> {
        let sftp = Arc::clone(&self.link.sftp);
        let handle = self.handle.clone();
        let limit = self.site.config.timeouts.request;
        let attributes = call(&self.site.runtime, async move {
            limited(limit, async { sftp.fstat(handle).await.map_err(from_sftp) }).await
        })
        .map_err(|error| self.failed(error))?;

        Ok(meta_of(&attributes.attrs))
    }
}

impl Drop for SftpReader {
    /// Closes the handle on the runtime, without waiting: a drop may be on any thread, and a server that is not
    /// answering would cost the whole request limit. The lease is let go when the handle is.
    fn drop(&mut self) {
        let link = Arc::clone(&self.link);
        let handle = self.handle.clone();
        let lease = self.lease.take();
        let limit = self.site.config.timeouts.request;

        self.site.runtime.spawn(async move {
            let _ = limited(limit, async { link.sftp.close(handle).await.map_err(from_sftp) }).await;

            drop(lease);
        });
    }
}

/// `chunks` reads of `size` bytes from `offset` on (the three, in this order), in flight together; their replies in the
/// order of their offsets. Each has the request limit.
async fn fetch(
    sftp: Arc<RawSftpSession>,
    handle: SftpHandle,
    (offset, size, chunks): (u64, usize, usize),
    limit: Duration,
) -> io::Result<Vec<io::Result<Vec<u8>>>> {
    let mut reads = JoinSet::new();

    for index in 0..chunks {
        let sftp = Arc::clone(&sftp);
        let handle = handle.clone();
        let at = offset + (index * size) as u64;

        reads.spawn(async move {
            let reply = limited(limit, async {
                sftp.read(handle, at, size as u32).await.map_err(from_sftp)
            })
            .await;

            (index, reply.map(|data| data.data))
        });
    }

    let mut replies: Vec<Option<io::Result<Vec<u8>>>> = (0..chunks).map(|_| None).collect();

    while let Some(joined) = reads.join_next().await {
        let (index, reply) = joined.map_err(|_| closed())?;

        replies[index] = Some(reply);
    }

    Ok(replies
        .into_iter()
        .map(|reply| reply.unwrap_or_else(|| Err(closed())))
        .collect())
}
