//! [`RemoteFs`] over a site's SFTP channel: the calls a pane's listing and a download make, each with its own time limit.
//! What reads a file is in [`SftpReader`](crate::sftp_files::SftpReader).

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pairpane_core::model::SiteId;
use pairpane_core::vfs::{RemoteEntry, RemoteFs, RemoteKind, RemoteMeta, RemotePath, RemoteReader};
use russh_sftp::client::RawSftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, OpenFlags};

use crate::error::{fail, from_sftp};
use crate::link::Link;
use crate::names::{Collected, MAX_NAME_BYTES};
use crate::runtime::{call, limited};
use crate::sftp_files::SftpReader;
use crate::site::Site;

/// The most entries one listing returns; a server that sends more is refused rather than held in memory.
pub(crate) const MAX_ENTRIES: usize = 1_000_000;

const TYPE_MASK: u32 = 0o170_000;
const TYPE_DIR: u32 = 0o040_000;
const TYPE_FILE: u32 = 0o100_000;
const TYPE_LINK: u32 = 0o120_000;
const PERMISSIONS: u32 = 0o7777;

/// The browsing and downloading calls of one site. Blocking, and safe to call from several threads.
pub(crate) struct SftpFs {
    site: Arc<Site>,
}

impl SftpFs {
    pub(crate) fn new(site: Arc<Site>) -> Self {
        Self { site }
    }

    /// Runs `op` on the site's connection. When the connection turns out to be gone, it is opened again and `op` runs
    /// once more on the new one.
    fn run<T>(&self, mut op: impl FnMut(&Arc<Link>) -> io::Result<T>) -> io::Result<T> {
        let _lease = self.site.lease();
        let link = self.link()?;

        match op(&link) {
            Err(error) if error.kind() == io::ErrorKind::ConnectionAborted => {
                self.site.forget(&link);

                let fresh = self.link()?;

                op(&fresh)
            }
            result => result,
        }
    }

    /// Runs `op` on the site's connection, once: a call that changes the server is never repeated on a new connection,
    /// because after a lost one nobody can tell whether it took effect. A connection found gone is forgotten, so that
    /// the next call opens a new one.
    fn run_once<T>(&self, op: impl FnOnce(&Link) -> io::Result<T>) -> io::Result<T> {
        let _lease = self.site.lease();
        let link = self.link()?;
        let result = op(&link);

        if result
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::ConnectionAborted)
        {
            self.site.forget(&link);
        }

        result
    }

    fn link(&self) -> io::Result<Arc<Link>> {
        let site = Arc::clone(&self.site);

        call(&self.site.runtime, async move { site.link().await })
    }

    /// One SFTP request, ended by the request limit at the latest.
    fn request<T, F, Fut>(&self, link: &Link, op: F) -> io::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Arc<RawSftpSession>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, SftpError>> + Send,
    {
        let sftp = Arc::clone(&link.sftp);
        let limit = self.site.config.timeouts.request;

        call(&self.site.runtime, async move {
            limited(limit, async { op(sftp).await.map_err(from_sftp) }).await
        })
    }

    /// The attributes of `path` (of what a link points to, unless `itself`), `None` when there is no such file.
    fn attributes(&self, path: &RemotePath, itself: bool) -> io::Result<Option<RemoteMeta>> {
        self.run(|link| {
            let path = path.as_str().to_owned();
            let asked = self.request(link, move |sftp| async move {
                if itself {
                    sftp.lstat(path).await
                } else {
                    sftp.stat(path).await
                }
            });

            match asked {
                Ok(attributes) => Ok(Some(meta_of(&attributes.attrs))),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            }
        })
    }

    fn list(&self, link: &Link, dir: &RemotePath, cancel: &AtomicBool) -> io::Result<Vec<RemoteEntry>> {
        let path = dir.as_str().to_owned();
        let handle = self
            .request(link, move |sftp| async move { sftp.opendir(path).await })?
            .handle
            .into_bytes();
        let listed = self.pages(link, &handle, cancel);

        // The handle is let go whatever happened, unless the server is not answering: that would cost another wait.
        let stuck = listed
            .as_ref()
            .is_err_and(|error| matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::ConnectionAborted));

        if !stuck {
            let _ = self.request(link, move |sftp| async move { sftp.close(handle).await });
        }

        listed
    }

    fn pages(&self, link: &Link, handle: &[u8], cancel: &AtomicBool) -> io::Result<Vec<RemoteEntry>> {
        let mut entries = Collected::new(MAX_NAME_BYTES);

        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(fail(io::ErrorKind::Interrupted, "the listing was cancelled"));
            }

            let handle = handle.to_vec();
            let page = match self.request(link, move |sftp| async move { sftp.readdir(handle).await }) {
                Ok(page) => page,
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(entries.into_entries()),
                Err(error) => return Err(error),
            };

            // A server that has no more entries says so; an empty page is the same, and must not loop.
            if page.files.is_empty() {
                return Ok(entries.into_entries());
            }

            for file in page.files {
                entries.add(file.filename, meta_of(&file.attrs))?;
            }

            if entries.len() > MAX_ENTRIES {
                return Err(fail(io::ErrorKind::Other, "the folder has too many entries"));
            }
        }
    }
}

impl RemoteFs for SftpFs {
    fn site(&self) -> SiteId {
        self.site.id
    }

    fn label(&self) -> &str {
        &self.site.label
    }

    fn read_dir(&self, dir: &RemotePath, cancel: &AtomicBool) -> io::Result<Vec<RemoteEntry>> {
        self.run(|link| self.list(link, dir, cancel))
    }

    fn stat(&self, path: &RemotePath) -> io::Result<Option<RemoteMeta>> {
        self.attributes(path, false)
    }

    fn lstat(&self, path: &RemotePath) -> io::Result<Option<RemoteMeta>> {
        self.attributes(path, true)
    }

    fn realpath(&self, path: &RemotePath) -> io::Result<RemotePath> {
        self.run(|link| {
            let path = path.as_str().to_owned();
            let name = self.request(link, move |sftp| async move { sftp.realpath(path).await })?;
            let unexpected = || fail(io::ErrorKind::InvalidData, "the server sent something unexpected");

            let file = name.files.into_iter().next().ok_or_else(unexpected)?;

            RemotePath::new(&file.filename).map_err(|_| unexpected())
        })
    }

    fn open_read(&self, path: &RemotePath) -> io::Result<Box<dyn RemoteReader>> {
        // Taken first: from here on the connection is not closed for being idle, whatever the reader's life is like.
        let lease = self.site.lease_owned();
        let (link, handle) = self.run(|link| {
            let path = path.as_str().to_owned();
            let opened = self.request(link, move |sftp| async move {
                sftp.open(path, OpenFlags::READ, FileAttributes::default()).await
            })?;

            Ok((Arc::clone(link), opened.handle))
        })?;

        Ok(Box::new(SftpReader::new(link, Arc::clone(&self.site), handle, lease)))
    }

    fn remove_file(&self, path: &RemotePath) -> io::Result<()> {
        self.run_once(|link| {
            let path = path.as_str().to_owned();

            self.request(link, move |sftp| async move { sftp.remove(path).await })
                .map(|_| ())
        })
    }

    fn remove_dir(&self, path: &RemotePath) -> io::Result<()> {
        self.run_once(|link| {
            let path = path.as_str().to_owned();

            self.request(link, move |sftp| async move { sftp.rmdir(path).await })
                .map(|_| ())
        })
    }

    fn read_link(&self, path: &RemotePath) -> io::Result<String> {
        self.run(|link| {
            let path = path.as_str().to_owned();
            let name = self.request(link, move |sftp| async move { sftp.readlink(path).await })?;

            name.files.into_iter().next().map(|file| file.filename).ok_or_else(|| {
                fail(
                    io::ErrorKind::InvalidData,
                    "the server did not say where the link points",
                )
            })
        })
    }
}

/// What the server's attributes say, as the listing wants it. The type is in the mode's upper bits.
pub(crate) fn meta_of(attributes: &FileAttributes) -> RemoteMeta {
    let kind = match attributes.permissions.map(|mode| mode & TYPE_MASK) {
        Some(TYPE_DIR) => RemoteKind::Dir,
        Some(TYPE_FILE) => RemoteKind::File,
        Some(TYPE_LINK) => RemoteKind::Link,
        _ => RemoteKind::Other,
    };

    RemoteMeta {
        kind,
        size: if kind == RemoteKind::Dir {
            0
        } else {
            attributes.size.unwrap_or(0)
        },
        modified_ms: attributes.mtime.map(|seconds| i64::from(seconds) * 1000),
        mode: attributes.permissions.map(|mode| mode & PERMISSIONS),
    }
}
