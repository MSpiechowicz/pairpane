//! The SFTP service of the test server: a view of a folder on disk, to browse, read from and remove from.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, SftpHandle, SftpString, Status, StatusCode,
};

use super::open::OpenFiles;
use super::{Counts, HOME, Settings};

/// The entries of an open folder, and how many were sent.
struct Listing {
    files: Vec<File>,
    sent: usize,
}

#[derive(Clone)]
pub(super) struct Files {
    root: PathBuf,
    page_size: usize,
    page_delay: Duration,
    stalled: Vec<String>,
    short_reads: Option<usize>,
    stall_reads: bool,
    fail_reads_from: Option<u64>,
    stall_reads_after: Option<usize>,
    /// How many READ requests this connection received.
    reads: Arc<AtomicUsize>,
    counts: Arc<Counts>,
    open: Arc<Mutex<HashMap<String, Listing>>>,
    files: OpenFiles,
}

impl Files {
    pub(super) fn new(root: PathBuf, settings: &Settings, counts: Arc<Counts>) -> Self {
        Self {
            root,
            page_size: settings.page_size.max(1),
            page_delay: settings.page_delay,
            stalled: settings.stalled.clone(),
            short_reads: settings.short_reads,
            stall_reads: settings.stall_reads,
            fail_reads_from: settings.fail_reads_from,
            stall_reads_after: settings.stall_reads_after,
            reads: Arc::new(AtomicUsize::new(0)),
            counts,
            open: Arc::new(Mutex::new(HashMap::new())),
            files: OpenFiles::default(),
        }
    }

    /// The connection is over: its open files are closed, whether or not a request of it is still waiting (a stalled
    /// one never ends, and would keep them open).
    pub(super) fn end(&self) {
        self.files.close_all();
    }

    fn on_disk(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    /// `path` with its links resolved, as a path of the served folder; `None` when it does not exist or leads out of it.
    fn resolved(&self, path: &str) -> Option<String> {
        let root = fs::canonicalize(&self.root).ok()?;
        let found = fs::canonicalize(self.on_disk(path)).ok()?;
        let inside = found.strip_prefix(&root).ok()?;
        let names: Vec<_> = inside
            .components()
            .map(|name| name.as_os_str().to_string_lossy())
            .collect();

        Some(format!("/{}", names.join("/")))
    }

    /// Counts a change, and never answers one for a stalled path.
    async fn change(&self, path: &str) {
        self.counts.removes.fetch_add(1, Ordering::Relaxed);

        if self.stalled.iter().any(|stalled| stalled == path) {
            std::future::pending::<()>().await;
        }
    }
}

fn done(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".to_owned(),
        language_tag: "en".to_owned(),
    }
}

/// The status a failed change on disk is answered with.
fn refused(error: &std::io::Error) -> StatusCode {
    match error.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

fn attributes(metadata: &fs::Metadata) -> FileAttributes {
    FileAttributes::from(metadata)
}

impl russh_sftp::server::Handler for Files {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        if path == "." {
            return Ok(Name {
                id,
                files: vec![File::dummy(HOME.to_owned())],
            });
        }

        let resolved = self.resolved(&path).ok_or(StatusCode::NoSuchFile)?;

        Ok(Name {
            id,
            files: vec![File::dummy(resolved)],
        })
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        if self.stalled.contains(&path) {
            std::future::pending::<()>().await;
        }

        if path.ends_with("/locked") {
            return Err(StatusCode::PermissionDenied);
        }

        let entries = fs::read_dir(self.on_disk(&path)).map_err(|_| StatusCode::NoSuchFile)?;
        let files = entries
            .flatten()
            .filter_map(|entry| {
                let metadata = fs::symlink_metadata(entry.path()).ok()?;

                Some(File::new(
                    entry.file_name().to_string_lossy().into_owned(),
                    attributes(&metadata),
                ))
            })
            .collect();
        let mut open = self.open.lock().expect("open folders");
        let handle = format!("folder-{}", open.len() + 1);

        open.insert(handle.clone(), Listing { files, sent: 0 });

        Ok(Handle {
            id,
            handle: SftpHandle::from(handle.as_str()),
        })
    }

    async fn readdir(&mut self, id: u32, handle: SftpHandle) -> Result<Name, Self::Error> {
        tokio::time::sleep(self.page_delay).await;

        let key = String::from_utf8_lossy(handle.as_bytes()).into_owned();
        let mut open = self.open.lock().expect("open folders");
        let listing = open.get_mut(&key).ok_or(StatusCode::Failure)?;
        let page: Vec<File> = listing
            .files
            .iter()
            .skip(listing.sent)
            .take(self.page_size)
            .cloned()
            .collect();

        if page.is_empty() {
            return Err(StatusCode::Eof);
        }

        listing.sent += page.len();
        self.counts.pages.fetch_add(1, Ordering::Relaxed);

        Ok(Name { id, files: page })
    }

    async fn open(
        &mut self,
        id: u32,
        filename: SftpString,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        if pflags.bits() != OpenFlags::READ.bits() {
            return Err(StatusCode::OpUnsupported);
        }

        let path = self.on_disk(&filename);
        let metadata = fs::metadata(&path).map_err(|error| refused(&error))?;

        if !metadata.is_file() {
            return Err(StatusCode::Failure);
        }

        let file = fs::File::open(path).map_err(|error| refused(&error))?;
        let handle = self.files.add(file, &self.counts);

        Ok(Handle {
            id,
            handle: SftpHandle::from(handle.as_str()),
        })
    }

    async fn read(&mut self, id: u32, handle: SftpHandle, offset: u64, len: u32) -> Result<Data, Self::Error> {
        let asked = self.reads.fetch_add(1, Ordering::Relaxed) + 1;

        self.counts.reads.fetch_add(1, Ordering::Relaxed);

        if self.stall_reads {
            std::future::pending::<()>().await;
        }

        if self.stall_reads_after.is_some_and(|most| asked > most) {
            while !self.counts.reads_released.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }

        if self.fail_reads_from.is_some_and(|from| offset >= from) {
            return Err(StatusCode::Failure);
        }

        let key = String::from_utf8_lossy(handle.as_bytes()).into_owned();
        let len = self.short_reads.map_or(len as usize, |most| most.min(len as usize));
        let data = self
            .files
            .read(&key, offset, len)
            .ok_or(StatusCode::Failure)?
            .map_err(|_| StatusCode::Failure)?;

        if data.is_empty() {
            return Err(StatusCode::Eof);
        }

        Ok(Data { id, data })
    }

    async fn fstat(&mut self, id: u32, handle: SftpHandle) -> Result<Attrs, Self::Error> {
        let key = String::from_utf8_lossy(handle.as_bytes()).into_owned();
        let metadata = self
            .files
            .metadata(&key)
            .ok_or(StatusCode::Failure)?
            .map_err(|_| StatusCode::Failure)?;

        Ok(Attrs {
            id,
            attrs: attributes(&metadata),
        })
    }

    async fn close(&mut self, id: u32, handle: SftpHandle) -> Result<Status, Self::Error> {
        let key = String::from_utf8_lossy(handle.as_bytes()).into_owned();

        if !self.files.close(&key) {
            self.open.lock().expect("open folders").remove(&key);
        }

        Ok(done(id))
    }

    async fn remove(&mut self, id: u32, filename: SftpString) -> Result<Status, Self::Error> {
        self.change(&filename).await;
        fs::remove_file(self.on_disk(&filename)).map_err(|error| refused(&error))?;

        Ok(done(id))
    }

    async fn rmdir(&mut self, id: u32, path: SftpString) -> Result<Status, Self::Error> {
        self.change(&path).await;
        fs::remove_dir(self.on_disk(&path)).map_err(|error| refused(&error))?;

        Ok(done(id))
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let metadata = fs::symlink_metadata(self.on_disk(&path)).map_err(|_| StatusCode::NoSuchFile)?;

        Ok(Attrs {
            id,
            attrs: attributes(&metadata),
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let metadata = fs::metadata(self.on_disk(&path)).map_err(|_| StatusCode::NoSuchFile)?;

        Ok(Attrs {
            id,
            attrs: attributes(&metadata),
        })
    }

    async fn readlink(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let target = fs::read_link(self.on_disk(&path)).map_err(|_| StatusCode::NoSuchFile)?;

        Ok(Name {
            id,
            files: vec![File::dummy(target.to_string_lossy().into_owned())],
        })
    }
}
