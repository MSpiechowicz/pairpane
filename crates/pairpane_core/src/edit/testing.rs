//! Helpers shared by the tests of the editor: files on disk opened and saved the way the shell does.

use std::fs;
use std::path::Path;

use crate::model::{EditDocument, EditOpened, EditSaveOptions, EditSaved, LineEnding};

use super::{open, save};

#[cfg(unix)]
pub(super) use crate::view::testing::fifo;
pub(super) use crate::view::testing::{put, temp};

pub(super) fn read(folder: &Path, name: &str) -> Vec<u8> {
    fs::read(folder.join(name)).expect("read")
}

/// The names in `folder`, sorted.
pub(super) fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("a folder")
        .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();

    names
}

/// Whether the tests run as root, which no permission stops.
#[cfg(unix)]
pub(super) fn is_root() -> bool {
    rustix::process::geteuid().is_root()
}

/// The document of the file, which must open as text.
pub(super) fn document(folder: &Path, name: &str) -> EditDocument {
    match open(folder, name, None).expect("open") {
        EditOpened::Text { document } => document,
        other => panic!("not text: {other:?}"),
    }
}

/// UTF-8 with no mark and `\n` breaks, saved at `revision` without force.
pub(super) fn options(revision: &str) -> EditSaveOptions {
    EditSaveOptions {
        encoding: "UTF-8".to_owned(),
        bom: false,
        line_ending: LineEnding::Lf,
        revision: revision.to_owned(),
        force: false,
    }
}

/// Saves `text` over the file as it is now, in UTF-8 with `\n` breaks.
pub(super) fn save_now(folder: &Path, name: &str, text: &str) -> EditSaved {
    let revision = document(folder, name).revision;

    save(folder, name, text, &options(&revision)).expect("save")
}
