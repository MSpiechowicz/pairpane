//! Opening a file in the editor.

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use crate::model::{EditDocument, EditOpened};
use crate::view::{Refusals, open_regular, refusal};

use super::text::{self, Decoded};
use super::{MAX_EDIT, disk};

/// The words the editor refuses a name that is not a regular file in.
pub(super) const EDITOR: Refusals = Refusals {
    link: "the editor opens regular files, not links",
    folder: "the editor opens files, not folders",
    other: "the editor opens regular files only",
};

/// Checks that `path` is a regular file for an editor that is another program, which gets the path and opens it
/// itself: a link is followed (the program does too), a folder and anything that is not a regular file are refused.
pub fn check_regular(path: &Path) -> io::Result<()> {
    let meta = fs::metadata(path)?;

    if meta.is_dir() {
        return Err(refusal(io::ErrorKind::InvalidInput, EDITOR.folder));
    }

    if !meta.is_file() {
        return Err(refusal(io::ErrorKind::InvalidInput, EDITOR.other));
    }

    Ok(())
}

/// Opens the regular file `name` of `folder` as text, in the encoding labelled `encoding` or, without one, in the one
/// found from the whole file. A link, a folder and anything that is not a regular file are refused; a pipe is never
/// waited on.
pub fn open(folder: &Path, name: &str, encoding: Option<&str>) -> io::Result<EditOpened> {
    // An encoding that is not known is refused before anything is read.
    if let Some(label) = encoding {
        crate::view::by_name(label)?;
    }

    let (file, meta) = open_regular(folder, name, &EDITOR)?;

    if meta.len() > MAX_EDIT as u64 {
        return Ok(EditOpened::TooLarge { size: meta.len() });
    }

    // One byte more than the most: a file that grew since it was looked at is told as too large, not cut.
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
    (&file).take(MAX_EDIT as u64 + 1).read_to_end(&mut bytes)?;

    if bytes.len() > MAX_EDIT {
        return Ok(EditOpened::TooLarge {
            size: bytes.len() as u64,
        });
    }

    document_of(
        name,
        &bytes,
        encoding,
        (!disk::writable(&folder.join(name), &meta), disk::revision(&meta)),
    )
}

/// The text of `bytes`, an entry of an archive called `name`, for a comparison: in the encoding labelled `encoding` or the
/// one found from the bytes, read-only (an archive is never written from the editor). More than the editor takes is
/// [`EditOpened::TooLarge`].
pub fn open_bytes(name: &str, bytes: &[u8], encoding: Option<&str>) -> io::Result<EditOpened> {
    if let Some(label) = encoding {
        crate::view::by_name(label)?;
    }
    if bytes.len() > MAX_EDIT {
        return Ok(EditOpened::TooLarge {
            size: bytes.len() as u64,
        });
    }

    document_of(name, bytes, encoding, (true, String::new()))
}

/// The document of the text `bytes` of the file `name`, with whether it is read-only and the revision it was read at.
fn document_of(
    name: &str,
    bytes: &[u8],
    encoding: Option<&str>,
    (read_only, revision): (bool, String),
) -> io::Result<EditOpened> {
    let (text, found, bom) = match text::decode(bytes, encoding)? {
        Decoded::Text { text, encoding, bom } => (text, encoding, bom),
        Decoded::Binary => return Ok(EditOpened::Binary),
        Decoded::Undecodable(found) => {
            return Ok(EditOpened::Undecodable {
                encoding: found.name().to_owned(),
            });
        }
    };

    let breaks = text::breaks(&text);

    Ok(EditOpened::Text {
        document: EditDocument {
            name: name.to_owned(),
            text: text::normalise(&text),
            encoding: found.name().to_owned(),
            bom,
            line_ending: breaks.ending,
            mixed_endings: breaks.mixed,
            read_only,
            size: bytes.len() as u64,
            revision,
        },
    })
}
