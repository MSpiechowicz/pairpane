//! The two doors into an archive of any format: [`records`] walks its entries (and [`build_index`] keeps them), and
//! [`open_source`] reads the data of chosen ones. A format that this build does not read yet says `Unsupported` here;
//! this is the one place a new format is wired in.

use std::cell::Cell;
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use crate::model::ArchiveFormat;

use super::MAX_INDEX_SCAN;
use super::entries::{EntrySource, Flow, Record};
use super::error::unsupported;
use super::file::ReadAt;
use super::gz::{self, GzSource};
use super::index::{ArchiveIndex, Builder};
use super::password::Password;
use super::tar::{self, TarSource, stream};
use super::zip::directory;

/// An opened archive file.
#[derive(Clone, Copy)]
pub struct Input<'a> {
    /// The file, read at offsets.
    pub file: &'a dyn ReadAt,
    /// Where it is: the name a gzip file falls back to, and the file a RAR is handed to `unrar` by. The viewer has no
    /// path and passes `None`.
    pub path: Option<&'a Path>,
}

/// Walks the entries of the archive of `format` in archive order, calling `visit` with each until the end or until it says
/// [`Flow::Stop`]. A compressed tar is decoded at most `scan` bytes. `password` is for the formats that can encrypt their
/// headers (7z); a password that is needed and missing is an error that [`super::password_fault`] knows. Returns whether the
/// archive ended early (damaged after some entries, or past `scan`): what was visited is good. Reads only what listing
/// needs; no data is written anywhere.
pub fn records(
    format: ArchiveFormat,
    input: &Input<'_>,
    cancel: &AtomicBool,
    scan: u64,
    _password: Option<&Password>,
    visit: &mut dyn FnMut(Record) -> Flow,
) -> io::Result<bool> {
    match format {
        ArchiveFormat::Zip => Ok(directory::walk(input.file, cancel, visit)?.cut),
        ArchiveFormat::Gz => Ok(gz::walk(input.file, input.path, visit)?.cut),
        ArchiveFormat::Tar
        | ArchiveFormat::TarGz
        | ArchiveFormat::TarXz
        | ArchiveFormat::TarZst
        | ArchiveFormat::TarBz2 => {
            let reached = Cell::new(false);
            let mut source = stream::open(format, input.file, cancel, &reached, scan)?;

            Ok(tar::walk(&mut source, Some(&reached), cancel, |record, _| Ok(visit(record)))?.cut)
        }
        ArchiveFormat::SevenZ | ArchiveFormat::Rar => Err(unsupported("this kind of archive cannot be read yet")),
    }
}

/// The index of the archive of `format`: its entries and the tree of its folders (see [`ArchiveIndex`]), within the limits
/// of [`super::MAX_INDEX_ENTRIES`], [`super::MAX_INDEX_NAME_BYTES`] and [`MAX_INDEX_SCAN`]. An archive that is cut or
/// larger than the limits gives an index marked truncated. Errors are those of [`records`]; one that holds no usable
/// text of ours is `InvalidData` ("the archive cannot be read").
pub fn build_index(
    format: ArchiveFormat,
    input: &Input<'_>,
    cancel: &AtomicBool,
    password: Option<&Password>,
) -> io::Result<ArchiveIndex> {
    let mut builder = Builder::new(format);
    let cut = records(format, input, cancel, MAX_INDEX_SCAN, password, &mut |record| {
        builder.push(record)
    })?;

    Ok(builder.finish(cut))
}

/// A source of the data of the entries of `index`, read from `input`. It reads what `password` opens and stops when
/// `cancel` is raised.
pub fn open_source<'a>(
    index: &'a ArchiveIndex,
    input: &Input<'a>,
    cancel: &'a AtomicBool,
    _password: Option<&'a Password>,
) -> io::Result<Box<dyn EntrySource + 'a>> {
    match index.format() {
        ArchiveFormat::Tar
        | ArchiveFormat::TarGz
        | ArchiveFormat::TarXz
        | ArchiveFormat::TarZst
        | ArchiveFormat::TarBz2 => Ok(Box::new(TarSource::new(index, input.file, cancel, MAX_INDEX_SCAN))),
        ArchiveFormat::Gz => Ok(Box::new(GzSource::new(index, input.file, cancel, MAX_INDEX_SCAN))),
        ArchiveFormat::Zip | ArchiveFormat::SevenZ | ArchiveFormat::Rar => {
            Err(unsupported("this kind of archive cannot be read yet"))
        }
    }
}
