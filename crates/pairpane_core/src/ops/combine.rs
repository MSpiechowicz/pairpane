//! Joining the parts of a split file: `name.001`, `name.002` and so on, read in order until one is missing, into one
//! temporary file that is put in place with a rename that refuses to replace. When a `name.crc` file of Total Commander
//! is beside the parts, the joined file must match its size and CRC-32 or nothing is kept.

use std::fs::{self, Metadata};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::listing::platform;
use crate::model::JobPhase;

use super::MAX_PARTS;
use super::meta;
use super::open;
use super::partial::Partial;
use super::report::refusal;
use super::split::{has_more, part_name};
use super::task::{Scene, as_text, derived_name, require_free};
use super::transfer::{Run, Stop};

/// The largest `.crc` file that is read, in bytes.
const MAX_CRC_FILE: u64 = 4 * 1024;

/// What a `.crc` file says about the joined file.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct CrcFile {
    pub(super) filename: Option<String>,
    pub(super) size: Option<u64>,
    pub(super) crc32: Option<u32>,
}

/// Reads the text of a `.crc` file: `key=value` lines (`filename`, `size`, `crc32`; the keys in any case). `None` when
/// it says nothing about the size or the CRC-32, or a value is not what its key needs.
pub(super) fn parse_crc(text: &str) -> Option<CrcFile> {
    let mut found = CrcFile::default();

    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();

        match key.trim().to_ascii_lowercase().as_str() {
            "filename" => found.filename = Some(value.to_owned()),
            "size" => found.size = Some(value.parse().ok()?),
            "crc32" => found.crc32 = Some(u32::from_str_radix(value, 16).ok()?),
            _ => {}
        }
    }

    (found.size.is_some() || found.crc32.is_some()).then_some(found)
}

impl Run<'_> {
    /// Joins the parts of the file whose first part is the item in `scene.items` (exactly one) into `scene.to`.
    pub(super) fn combine_parts(&mut self, scene: &Scene) -> Result<(), Stop> {
        let Some(item) = scene.items.first() else {
            return Ok(());
        };
        let name = as_text(&item.name)?;
        let Some(base) = name.strip_suffix(".001").filter(|base| !base.is_empty()) else {
            return Err(refusal(
                io::ErrorKind::InvalidInput,
                "choose the first part of the file: its name ends with .001",
            )
            .into());
        };

        self.phase(JobPhase::Scanning);
        let parts = self.find_parts(&scene.from, base)?;
        let crc = read_crc(&scene.from, base)?;

        let output = match crc.as_ref().and_then(|crc| crc.filename.as_deref()) {
            Some(named) if may_name_the_file(named, base) => named.to_owned(),
            _ => base.to_owned(),
        };
        let target = scene.to.join(&output);
        require_free(&target)?;

        self.phase(JobPhase::Combining);
        let total = parts.iter().map(|(_, meta)| meta.len()).sum();
        self.begin_file(&output, total);
        let done = self.join_parts(scene, &parts, crc.as_ref(), &target);
        self.end_file();

        done
    }

    /// The parts `base.001`, `base.002` and so on, up to the first that is missing, each a regular file (a link is
    /// refused, not followed). Counts them as the files of the job.
    fn find_parts(&mut self, from: &Path, base: &str) -> Result<Vec<(PathBuf, Metadata)>, Stop> {
        let mut parts = Vec::new();

        for number in 1..=MAX_PARTS {
            self.checkpoint()?;
            let path = from.join(part_name(base, number));

            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(error) if error.kind() == io::ErrorKind::NotFound && number > 1 => break,
                Err(error) => return Err(error.into()),
            };
            if !meta.is_file() {
                return Err(refusal(io::ErrorKind::InvalidInput, "a part is not a regular file").into());
            }

            self.found(meta.len());
            parts.push((path, meta));
        }

        Ok(parts)
    }

    fn join_parts(
        &mut self,
        scene: &Scene,
        parts: &[(PathBuf, Metadata)],
        crc: Option<&CrcFile>,
        target: &Path,
    ) -> Result<(), Stop> {
        let (temp, mut output) = self.make_temp(&scene.to, open::create_new)?;
        let mut guard = Partial::new(temp.clone());
        let mut hasher = crc32fast::Hasher::new();
        let mut total = 0_u64;

        for (path, meta) in parts {
            let mut part = open::source(path, meta)?;

            // Each part is read as long as it was when it was opened, and not a byte more.
            let len = part.metadata()?.len();
            let read = self.pump(&mut part, &mut output, len, &mut hasher)?;
            if read != len || has_more(&mut part)? {
                return Err(refusal(io::ErrorKind::Other, "a part changed while it was being read").into());
            }
            total += read;
        }

        let matches = crc.is_none_or(|crc| {
            crc.size.is_none_or(|size| size == total)
                && crc.crc32.is_none_or(|expected| expected == hasher.clone().finalize())
        });
        if !matches {
            return Err(refusal(
                io::ErrorKind::InvalidData,
                "the combined file does not match its .crc file",
            )
            .into());
        }

        if let Some((_, first)) = parts.first() {
            meta::apply_file(&output, first)?;
        }
        output.flush()?;
        drop(output);

        self.commit_new(&temp, target)?;
        guard.committed = true;

        Ok(())
    }
}

/// Whether the name in a `.crc` file may be the name of the joined file whose parts are `base.001` and so on: a plain
/// name that fits, does not start with a dot (so no `.profile` is made), and has the extension of `base` (case does
/// not matter; neither has one is fine), so a `.crc` file cannot turn `photo.jpg` into `photo.exe`.
fn may_name_the_file(named: &str, base: &str) -> bool {
    let extension = |name: &str| {
        name.rsplit_once('.')
            .map(|(_, extension)| extension.to_ascii_lowercase())
    };

    platform::is_plain_name(named)
        && super::name_fits(named)
        && !named.starts_with('.')
        && extension(named) == extension(base)
}

/// The `.crc` file beside the parts of `base`, when there is one. A `.crc` that is no regular file, is too big or
/// says nothing usable is an error: the joined file could not be checked, and the user did not ask for that.
fn read_crc(from: &Path, base: &str) -> Result<Option<CrcFile>, Stop> {
    let path = from.join(derived_name(base, ".crc")?);

    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !meta.is_file() || meta.len() > MAX_CRC_FILE {
        return Err(unreadable_crc().into());
    }

    let mut bytes = Vec::new();
    open::source(&path, &meta)?.take(MAX_CRC_FILE).read_to_end(&mut bytes)?;

    String::from_utf8(bytes)
        .ok()
        .and_then(|text| parse_crc(&text))
        .map(Some)
        .ok_or_else(|| unreadable_crc().into())
}

fn unreadable_crc() -> io::Error {
    refusal(io::ErrorKind::InvalidData, "the .crc file cannot be read")
}
