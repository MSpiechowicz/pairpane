//! [`RemoteFs`] over a site's SFTP channel: the calls a pane's listing and a download make, each with its own time limit.
//! What reads a file is in [`SftpReader`](crate::sftp_files::SftpReader).

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use pairpane_core::model::SiteId;
use pairpane_core::vfs::{RemoteEntry, RemoteFs, RemoteKind, RemoteMeta, RemotePath, RemoteReader};
use russh_sftp::client::RawSftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, OpenFlags};
use tokio::task::JoinSet;

use crate::error::{closed, fail, from_sftp, lost};
use crate::link::Link;
use crate::names::{Collected, MAX_NAME_BYTES};
use crate::runtime::{call, call_until, limited};
use crate::sftp_files::SftpReader;
use crate::site::Site;

/// The looks of one batch (see [`RemoteFs::lstat_many`]) in flight at once: what an SFTP client of OpenSSH keeps.
const LOOKS_IN_FLIGHT: usize = 64;

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

            present(asked.map(|attributes| meta_of(&attributes.attrs)))
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

    fn lstat_many(&self, paths: &[RemotePath], cancel: &AtomicBool) -> Vec<io::Result<Option<RemoteMeta>>> {
        let asked = self.run(|link| {
            let names = paths.iter().map(|path| path.as_str().to_owned()).collect();
            let limit = self.site.config.timeouts.request;
            let answers = call_until(&self.site.runtime, looks(Arc::clone(&link.sftp), names, limit), cancel)?;

            // A connection that was lost is opened again and the batch asked once more, as for one look.
            if answers.iter().any(|answer| {
                answer
                    .as_ref()
                    .is_err_and(|error| error.kind() == io::ErrorKind::ConnectionAborted)
            }) {
                return Err(lost());
            }

            Ok(answers)
        });

        // A batch that failed as a whole fails each look with the same words (they hold no path).
        asked.unwrap_or_else(|error| {
            paths
                .iter()
                .map(|_| Err(io::Error::new(error.kind(), error.to_string())))
                .collect()
        })
    }

    fn realpath(&self, path: &RemotePath) -> io::Result<RemotePath> {
        self.run(|link| {
            let path = path.as_str().to_owned();
            let name = self.request(link, move |sftp| async move { sftp.realpath(path).await })?;
            let file = name
                .files
                .into_iter()
                .next()
                .ok_or_else(|| fail(io::ErrorKind::InvalidData, "the server sent something unexpected"))?;

            answered_path(&file.filename)
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

/// `answer` of a look, where a path that is not there is `None`.
fn present(answer: io::Result<RemoteMeta>) -> io::Result<Option<RemoteMeta>> {
    match answer {
        Ok(meta) => Ok(Some(meta)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// The `lstat` of each of `paths`, [`LOOKS_IN_FLIGHT`] of them in flight together (the next goes out as one is answered);
/// the answers in the order of the paths. Each has the request limit.
async fn looks(
    sftp: Arc<RawSftpSession>,
    paths: Vec<String>,
    limit: Duration,
) -> io::Result<Vec<io::Result<Option<RemoteMeta>>>> {
    let mut answers: Vec<Option<io::Result<Option<RemoteMeta>>>> = (0..paths.len()).map(|_| None).collect();
    let mut waiting = paths.into_iter().enumerate();
    let mut asked = JoinSet::new();

    loop {
        while asked.len() < LOOKS_IN_FLIGHT
            && let Some((index, path)) = waiting.next()
        {
            let sftp = Arc::clone(&sftp);

            asked.spawn(async move {
                let answer = limited(limit, async { sftp.lstat(path).await.map_err(from_sftp) }).await;

                (index, present(answer.map(|attributes| meta_of(&attributes.attrs))))
            });
        }

        let Some(joined) = asked.join_next().await else {
            break;
        };
        let (index, answer) = joined.map_err(|_| closed())?;

        answers[index] = Some(answer);
    }

    Ok(answers
        .into_iter()
        .map(|answer| answer.unwrap_or_else(|| Err(closed())))
        .collect())
}

/// The path a server answered with as a [`RemotePath`]. A trailing slash (`/home/me/`) is only a spelling and is dropped;
/// anything else that is not a path of ours (a relative one, a `..` in it) is not guessed at.
fn answered_path(text: &str) -> io::Result<RemotePath> {
    let trimmed = match text.strip_suffix('/') {
        Some(rest) if rest.len() > 1 => rest,
        _ => text,
    };

    RemotePath::new(trimmed).map_err(|_| {
        fail(
            io::ErrorKind::InvalidData,
            "the server answered with a folder path Pairpane cannot use",
        )
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_the_server_ends_with_a_slash_is_the_same_path_and_the_root_is_kept() {
        for (said, is) in [
            ("/home/me/", "/home/me"),
            ("/home/me", "/home/me"),
            ("/", "/"),
            ("/a", "/a"),
        ] {
            assert_eq!(answered_path(said).expect("a path").as_str(), is, "{said}");
        }
    }

    #[test]
    fn a_path_that_cannot_be_used_is_refused_in_words_that_name_no_path() {
        for said in ["", "home/me", "/home//me/", "//", "/a/../b", "/a/./b/"] {
            let error = answered_path(said).expect_err(said);

            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{said}");
            assert!(
                error
                    .to_string()
                    .ends_with("the server answered with a folder path Pairpane cannot use"),
                "{said}: {error}"
            );
            assert!(!error.to_string().contains("home"), "{said}: {error}");
        }
    }
}
