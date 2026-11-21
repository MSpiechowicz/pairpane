//! Splitting a file into parts: `name.001`, `name.002` and so on, each a whole temporary file put in place with a
//! rename that refuses to replace, and last the `name.crc` file in the format of Total Commander, which `combine`
//! (and Total Commander) checks the joined file against. Whatever the job made is removed again when it is cancelled
//! or fails.

use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use crate::model::JobPhase;

use super::meta;
use super::open;
use super::partial::Partial;
use super::plan::Kind;
use super::report::refusal;
use super::task::{Scene, as_text, create_plain, derived_name, require_free};
use super::transfer::{Run, Stop};
use super::{MAX_PARTS, MIN_PART_SIZE};

/// The parts (and the `.crc` file) a split has put in place so far: removed again unless the split finished.
#[derive(Default)]
struct Made {
    paths: Vec<PathBuf>,
    keep: bool,
}

impl Drop for Made {
    fn drop(&mut self) {
        if self.keep {
            return;
        }

        for path in &self.paths {
            let _ = fs::remove_file(path);
        }
    }
}

/// The name of part `number` of `name`: at least three digits.
pub(super) fn part_name(name: &str, number: u64) -> String {
    format!("{name}.{number:03}")
}

/// The text of the `.crc` file of a file `name` of `size` bytes whose CRC-32 is `crc`: three lines ending in CRLF.
pub(super) fn crc_text(name: &str, size: u64, crc: u32) -> String {
    format!("filename={name}\r\nsize={size}\r\ncrc32={crc:08X}\r\n")
}

impl Run<'_> {
    /// Cuts the file in `scene.items` (exactly one) into parts of `part_size` bytes in `scene.to`.
    pub(super) fn split_file(&mut self, scene: &Scene, part_size: u64) -> Result<(), Stop> {
        let Some(item) = scene.items.first() else {
            return Ok(());
        };
        if item.kind != Kind::File {
            return Err(refusal(io::ErrorKind::InvalidInput, "only a regular file can be split").into());
        }

        let name = as_text(&item.name)?;
        let size = item.meta.len();
        if size == 0 {
            return Err(refusal(io::ErrorKind::InvalidInput, "an empty file cannot be split").into());
        }
        if part_size < MIN_PART_SIZE {
            return Err(refusal(io::ErrorKind::InvalidInput, "the parts would be too small").into());
        }

        let parts = size.div_ceil(part_size);
        if parts > MAX_PARTS {
            return Err(refusal(
                io::ErrorKind::InvalidInput,
                "the file would need more than 999 parts; choose a bigger part size",
            )
            .into());
        }

        // Every name is known and free before the first byte is written.
        // (`.001` and `.crc` are as long as every other part's ending, so one check of the length serves all.)
        let crc_name = derived_name(name, ".crc")?;
        derived_name(name, ".001")?;
        for number in 1..=parts {
            require_free(&scene.to.join(part_name(name, number)))?;
        }
        require_free(&scene.to.join(&crc_name))?;

        self.phase(JobPhase::Scanning);
        self.found(size);

        self.phase(JobPhase::Splitting);
        self.begin_file(name, size);
        let done = self.write_parts(scene, item_source(item)?, name, parts, part_size, &crc_name);
        self.end_file();

        done
    }

    fn write_parts(
        &mut self,
        scene: &Scene,
        mut source: (fs::File, fs::Metadata),
        name: &str,
        parts: u64,
        part_size: u64,
        crc_name: &str,
    ) -> Result<(), Stop> {
        let mut made = Made::default();
        let mut crc = crc32fast::Hasher::new();
        let mut total = 0_u64;
        let size = source.1.len();

        for number in 1..=parts {
            self.checkpoint()?;

            let (temp, mut part) = self.make_temp(&scene.to, open::create_new)?;
            let mut guard = Partial::new(temp.clone());

            total += self.pump(&mut source.0, &mut part, part_size, &mut crc)?;
            meta::apply_file(&part, &source.1)?;
            drop(part);

            let target = scene.to.join(part_name(name, number));
            self.commit_new(&temp, &target)?;
            guard.committed = true;
            made.paths.push(target);
        }

        // The file must be what was cut: the same size, and nothing left over after the last part.
        if total != size || has_more(&mut source.0)? {
            return Err(refusal(io::ErrorKind::Other, "the file changed while it was being split").into());
        }

        let (temp, mut file) = self.make_temp(&scene.to, create_plain)?;
        let mut guard = Partial::new(temp.clone());

        file.write_all(crc_text(name, total, crc.finalize()).as_bytes())?;
        drop(file);

        let target = scene.to.join(crc_name);
        self.commit_new(&temp, &target)?;
        guard.committed = true;
        made.paths.push(target);

        made.keep = true;

        Ok(())
    }
}

/// Opens the file `item` for reading, refusing a link, and returns it with its metadata as it is now.
fn item_source(item: &super::plan::Child) -> io::Result<(fs::File, fs::Metadata)> {
    let file = open::source(&item.path, &item.meta)?;
    let meta = file.metadata()?;

    Ok((file, meta))
}

pub(super) fn has_more(file: &mut fs::File) -> io::Result<bool> {
    let mut byte = [0_u8; 1];

    loop {
        match file.read(&mut byte) {
            Ok(read) => return Ok(read > 0),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}
