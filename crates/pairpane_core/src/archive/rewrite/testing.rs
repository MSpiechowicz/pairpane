//! Helpers of the rewrite tests: archives of every format made by the writers, a rewrite of one in memory, and what the
//! readers say of the result.

use std::io::{self, Cursor};

use crate::archive::testing::idle;
use crate::archive::write::testing::{Item, Seen, expected, options, pack, read_back};
use crate::archive::write::{Entry, Options, SevenZWriter, TarWriter};
use crate::archive::{ArchiveIndex, Input, Password, build_index};
use crate::model::{ArchiveFormat, PackFormat, PackLevel};

use super::Old;

/// The password of the encrypted archives of the tests.
pub(crate) const SECRET: &str = "secret ✓";

pub(crate) fn secret() -> Password {
    Password::new(SECRET.to_owned())
}

/// The formats a tar comes in.
pub(crate) const TARS: [ArchiveFormat; 5] = [
    ArchiveFormat::Tar,
    ArchiveFormat::TarGz,
    ArchiveFormat::TarXz,
    ArchiveFormat::TarZst,
    ArchiveFormat::TarBz2,
];

/// A copy of `item`.
pub(crate) fn copy(item: &Item) -> Item {
    Item {
        name: item.name.clone(),
        mode: item.mode,
        modified: item.modified,
        content: item.content.clone(),
    }
}

/// The format of the writers that makes `format` (a tar in any compression is the tar writer's).
pub(crate) fn packed_as(format: ArchiveFormat) -> PackFormat {
    match format {
        ArchiveFormat::Zip => PackFormat::Zip,
        ArchiveFormat::SevenZ => PackFormat::SevenZ,
        _ => PackFormat::TarZst,
    }
}

/// `items` in an archive of `format`, encrypted with `password` (zip and 7z; the names of a 7z too when `names`).
pub(crate) fn archive(format: ArchiveFormat, items: &[Item], password: Option<&str>, names: bool) -> Vec<u8> {
    if TARS.contains(&format) {
        let cancel = idle();
        let mut writer =
            TarWriter::with_format(format, Cursor::new(Vec::new()), PackLevel::Normal, &cancel).expect("a tar");

        for item in items {
            writer.add(item.entry()).expect("added");
        }

        return writer.finish().expect("finished").into_inner();
    }

    let options = Options {
        encrypt_names: names,
        ..options(PackLevel::Normal, password)
    };

    pack(packed_as(format), &options, items).expect("packed")
}

/// The index of `bytes`, which is an archive of `format`.
pub(crate) fn index(bytes: &Vec<u8>, format: ArchiveFormat, password: Option<&Password>) -> io::Result<ArchiveIndex> {
    build_index(
        format,
        &Input {
            file: bytes,
            path: None,
        },
        &idle(),
        password,
    )
}

/// The ordinal of the entry `path` of `bytes`.
pub(crate) fn ordinal(bytes: &Vec<u8>, format: ArchiveFormat, path: &str) -> u32 {
    index(bytes, format, Some(&secret()))
        .expect("an index")
        .entries()
        .iter()
        .find(|entry| entry.path == path)
        .expect("the entry")
        .ordinal
}

/// The archive `bytes` of `format` rewritten by `rewriter` (see [`rewrite_with`]) without the entries `left_out`, then
/// with `added`.
pub(crate) fn rewrite_by(
    bytes: &Vec<u8>,
    format: ArchiveFormat,
    (left_out, added): (&[u32], &[Item]),
    password: Option<&Password>,
    rewriter: impl for<'a> FnOnce(Cursor<Vec<u8>>, Old<'a>, Vec<Entry<'a>>) -> io::Result<Cursor<Vec<u8>>>,
) -> io::Result<Vec<u8>> {
    let cancel = idle();
    let index = index(bytes, format, password)?;
    let mut progress = |_: u64| Ok(());
    let old = Old {
        index: &index,
        file: bytes,
        cancel: &cancel,
        password,
        left_out,
        progress: &mut progress,
    };

    Ok(rewriter(Cursor::new(Vec::new()), old, added.iter().map(Item::entry).collect())?.into_inner())
}

/// [`rewrite_by`] with the rewrite of the crate.
pub(crate) fn rewrite_with(
    bytes: &Vec<u8>,
    format: ArchiveFormat,
    changes: (&[u32], &[Item]),
    password: Option<&Password>,
) -> io::Result<Vec<u8>> {
    rewrite_by(bytes, format, changes, password, super::rewrite)
}

/// What the readers say of `bytes`, by path.
pub(crate) fn seen(bytes: &Vec<u8>, format: ArchiveFormat, password: Option<&Password>) -> Vec<Seen> {
    let mut seen = read_back(bytes, format, password).expect("an archive that reads back");
    seen.sort_by(|a, b| a.path.cmp(&b.path));

    seen
}

/// What an archive of `format` should read back as when it has the items `items`, by path.
pub(crate) fn wanted(format: ArchiveFormat, items: &[Item]) -> Vec<Seen> {
    let mut wanted = expected(packed_as(format), items);
    wanted.sort_by(|a, b| a.path.cmp(&b.path));

    wanted
}

/// The items `items` but those named in `gone`, then `added`.
pub(crate) fn changed(items: &[Item], gone: &[&str], added: &[Item]) -> Vec<Item> {
    items
        .iter()
        .filter(|item| !gone.contains(&item.name.as_str()))
        .chain(added)
        .map(copy)
        .collect()
}

/// A 7z of `items` that closes a block at 1 KB of input.
pub(crate) fn sevenz_in_blocks(items: &[Item], password: Option<&str>, names: bool) -> Vec<u8> {
    let cancel = idle();
    let options = Options {
        encrypt_names: names,
        ..options(PackLevel::Normal, password)
    };
    let mut writer = SevenZWriter::new(Cursor::new(Vec::new()), &options, &cancel)
        .expect("a writer")
        .with_block(1000);

    for item in items {
        writer.add(item.entry()).expect("added");
    }

    writer.finish().expect("finished").into_inner()
}
