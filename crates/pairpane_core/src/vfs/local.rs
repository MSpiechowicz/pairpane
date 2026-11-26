//! A folder on disk.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::VfsFolder;
use crate::listing::{self, FolderWatch, Outcome, platform};
use crate::model::{ListEvent, PaneCapabilities, PaneLocation};

/// A folder on disk, shown as the path the user chose and read at the path that [`platform::local_path`] resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFolder {
    shown: PathBuf,
    actual: PathBuf,
}

impl LocalFolder {
    /// The folder `shown` (absolute), read where it leads once every link on its path is resolved and checked (see
    /// [`platform::local_path`]); fails when a link on it leads to a network or device path, or loops, or cannot be
    /// read. Touches the disk on Windows only. From here on the system resolves none of its own links.
    pub fn resolve(shown: PathBuf) -> io::Result<Self> {
        let actual = platform::local_path(&shown)?;

        Ok(Self { shown, actual })
    }

    /// The path the folder is read at: `shown` without links.
    pub fn actual(&self) -> &Path {
        &self.actual
    }
}

impl VfsFolder for LocalFolder {
    fn location(&self) -> PaneLocation {
        PaneLocation::Folder {
            path: self.shown.to_string_lossy().into_owned(),
        }
    }

    fn capabilities(&self) -> PaneCapabilities {
        PaneCapabilities::WRITABLE
    }

    fn list(&self, show_hidden: bool, cancel: &AtomicBool, emit: &mut dyn FnMut(ListEvent)) -> io::Result<Outcome> {
        listing::list_at(&self.shown, &self.actual, show_hidden, cancel, emit)
    }

    fn watch(&self, on_change: Box<dyn Fn() + Send + 'static>) -> Option<notify::Result<FolderWatch>> {
        Some(listing::watch(&self.actual, on_change))
    }
}
