//! The extra columns of the detailed view: the size of a picture in pixels and the SHA-256 of a file, computed for the
//! rows that are in view.
//!
//! # Why the sizes are read in the app's own process
//!
//! Decoding a picture is untrusted work and never happens in the app (`thumb` hands it to a sandboxed helper). Reading a
//! size is not decoding: it is a few bytes at a known place of the header. So the header parsers here (`header.rs` for PNG,
//! GIF and BMP, `jpeg.rs`, `webp.rs`, `heif.rs`) are small, and they are safe Rust that is bounded by construction:
//!
//! - at most [`HEAD_BYTES`] (64 KiB) are read from the start of the file, once, and that is all a parser ever sees;
//! - every access is a checked `get` on that slice, so a field that lies past its end is "no dimensions", never a panic;
//! - nothing is allocated from a value in the file (a size, a count, a length): the JPEG walk and the HEIF box walk
//!   move forward through the slice with loops that are bounded by its length or by a constant, and a count in a header
//!   only ever lowers a bound that a constant sets;
//! - there is no `unsafe`, no `image` crate and no codec: the pixels are never touched.
//!
//! The format is chosen from the first bytes by the viewer's own check ([`crate::view`]'s `sniff`), never from the
//! extension. A file of any other format, or whose header does not parse or says a side is zero, has no dimensions. What
//! the header says is returned as it is: a picture that claims 4 billion pixels on a side is reported so (the sides are
//! `u32`), since nothing is allocated for it.
//!
//! # Checksum
//!
//! [`checksum`] is the SHA-256 of a regular file up to [`MAX_CHECKSUM_BYTES`], read in chunks through the digest the
//! checksum job uses (`ops::Digester`), stopping between chunks when the cancel flag is raised. A bigger file is refused
//! (`InvalidInput`) before it is read, and one that grows past the cap while it is read is refused too.
//!
//! Both open the file without following a link as the last component (as the viewer and the thumbnails do) and refuse
//! anything that is not a regular file; the texts of the errors are fixed and hold no path.

mod header;
mod heif;
mod jpeg;
mod webp;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod fuzz_tests;
#[cfg(test)]
mod header_tests;
#[cfg(test)]
mod heif_tests;
#[cfg(test)]
mod jpeg_tests;
#[cfg(test)]
mod read_tests;

use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::model::{Dimensions, HashAlgorithm};
use crate::ops::Digester;

use super::file::ViewFile;
use super::{Refusals, cancelled, open_regular, refusal};

/// How much of the start of a file the size is looked for in. A frame header is near the start; a JPEG with more than this
/// of metadata before it has no dimensions here.
pub const HEAD_BYTES: usize = 64 * 1024;

/// The biggest file [`checksum`] takes, in bytes.
pub const MAX_CHECKSUM_BYTES: u64 = 64 * 1024 * 1024;

/// Bytes read between two looks at the cancel flag.
const CHUNK: usize = 1024 * 1024;

/// What is said when a name is not a regular file.
const REFUSALS: Refusals = Refusals {
    link: "a column is read of a regular file, not a link",
    folder: "a column is read of a file, not a folder",
    other: "a column is read of a regular file only",
};

/// The size of the picture file `name` of `folder`, from its header (see the module). `None` when the name is not a
/// regular file (a link, a folder, a missing one), cannot be read, is not a picture of a format that is read here, or its
/// header does not parse. A name that is not a plain name is `None` as well.
pub fn dimensions(folder: &Path, name: &str) -> Option<Dimensions> {
    let (file, meta) = open_regular(folder, name, &REFUSALS).ok()?;
    let file = ViewFile::new(file);

    let wanted = usize::try_from(meta.len()).map_or(HEAD_BYTES, |len| len.min(HEAD_BYTES));
    let head = file.read_at(0, wanted).ok()?;

    header::from_head(&head)
}

/// The SHA-256 of the regular file `name` of `folder` as lowercase hex. A file over [`MAX_CHECKSUM_BYTES`] is refused
/// (`InvalidInput`) without being read, a name that is not a regular file is refused (`InvalidInput`), and `cancel`
/// is looked at between the chunks (`Interrupted`). All with fixed texts.
pub fn checksum(folder: &Path, name: &str, cancel: &AtomicBool) -> io::Result<String> {
    let (mut file, meta) = open_regular(folder, name, &REFUSALS)?;

    if meta.len() > MAX_CHECKSUM_BYTES {
        return Err(too_big());
    }

    let mut digester = Digester::new(HashAlgorithm::Sha256);
    let mut buffer = vec![0; CHUNK];
    let mut total = 0_u64;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }

        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };

        // A file that grows while it is read is held to the cap as well.
        total += read as u64;
        if total > MAX_CHECKSUM_BYTES {
            return Err(too_big());
        }

        digester.update(&buffer[..read]);
    }

    Ok(digester.finish())
}

fn too_big() -> io::Error {
    refusal(io::ErrorKind::InvalidInput, "the file is too big to checksum here")
}
