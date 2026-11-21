//! Opening the file of a viewer session.

use std::fs;
use std::io;
use std::path::Path;

use crate::listing::platform;
use crate::model::ViewKind;
use crate::ops::open_listed;

use super::file::ViewFile;
use super::{SNIFF_BYTES, encoding, refusal, sniff};

/// A file opened for the viewer and what is known of it. The shell adds the session id.
#[derive(Debug)]
pub struct Opened {
    pub file: ViewFile,
    pub name: String,
    /// The size when it was opened, in bytes.
    pub size: u64,
    /// The last modification, in milliseconds since the Unix epoch.
    pub modified: Option<i64>,
    pub kind: ViewKind,
    /// The detected encoding label for text and Markdown; empty for any other kind.
    pub encoding: String,
}

/// What is said when a name is not a regular file: one text for a link, a folder and anything else, in the words of
/// the tool that asked.
pub(crate) struct Refusals {
    pub link: &'static str,
    pub folder: &'static str,
    pub other: &'static str,
}

const VIEWER: Refusals = Refusals {
    link: "the viewer opens regular files, not links",
    folder: "the viewer opens files, not folders",
    other: "the viewer opens regular files only",
};

/// Opens the regular file `name` of `folder` for reading without following a link, with its metadata. A link, a folder
/// and anything that is not a regular file are refused with the text of `words`; a pipe that slips in is never waited
/// on.
pub(crate) fn open_regular(folder: &Path, name: &str, words: &Refusals) -> io::Result<(fs::File, fs::Metadata)> {
    if !platform::is_plain_name(name) {
        return Err(refusal(io::ErrorKind::InvalidInput, "the name cannot be opened"));
    }

    let seen = fs::symlink_metadata(folder.join(name))?;

    if seen.file_type().is_symlink() {
        return Err(refusal(io::ErrorKind::InvalidInput, words.link));
    }

    if seen.is_dir() {
        return Err(refusal(io::ErrorKind::InvalidInput, words.folder));
    }

    if !seen.is_file() {
        return Err(refusal(io::ErrorKind::InvalidInput, words.other));
    }

    open_listed(folder, name)
}

/// Opens the regular file `name` of `folder` for viewing. A link, a folder and anything that is not a regular file
/// are refused with a text of their own; the file itself is opened without following a link and never waited on (a
/// pipe that slips in is refused). The result keeps the open file, so a later rename or delete changes nothing.
pub fn open(folder: &Path, name: &str) -> io::Result<Opened> {
    let (file, meta) = open_regular(folder, name, &VIEWER)?;
    let file = ViewFile::new(file);
    let size = file.size()?;
    let head = file.read_at(0, SNIFF_BYTES.min(size.try_into().unwrap_or(usize::MAX)))?;
    let kind = sniff::sniff(&head, name);

    let encoding = match kind {
        ViewKind::Text | ViewKind::Markdown => encoding::detect(&head, head.len() as u64 >= size).name().to_owned(),
        _ => String::new(),
    };

    Ok(Opened {
        file,
        name: name.to_owned(),
        size,
        modified: modified_millis(&meta),
        kind,
        encoding,
    })
}

/// Last modification in milliseconds since the Unix epoch (negative before it).
fn modified_millis(meta: &fs::Metadata) -> Option<i64> {
    let modified = meta.modified().ok()?;

    match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).ok(),
        Err(before) => i64::try_from(before.duration().as_millis()).ok().map(|millis| -millis),
    }
}
