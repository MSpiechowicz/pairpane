//! The SFTP service of the test server: a read-only view of a folder on disk.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh_sftp::protocol::{Attrs, File, FileAttributes, Handle, Name, SftpHandle, Status, StatusCode};

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
    counts: Arc<Counts>,
    open: Arc<Mutex<HashMap<String, Listing>>>,
}

impl Files {
    pub(super) fn new(root: PathBuf, settings: &Settings, counts: Arc<Counts>) -> Self {
        Self {
            root,
            page_size: settings.page_size.max(1),
            page_delay: settings.page_delay,
            stalled: settings.stalled.clone(),
            counts,
            open: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn on_disk(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
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
        let resolved = if path == "." { HOME.to_owned() } else { path };

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

    async fn close(&mut self, id: u32, handle: SftpHandle) -> Result<Status, Self::Error> {
        self.open
            .lock()
            .expect("open folders")
            .remove(&String::from_utf8_lossy(handle.as_bytes()).into_owned());

        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".to_owned(),
            language_tag: "en".to_owned(),
        })
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
