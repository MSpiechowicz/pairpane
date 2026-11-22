//! Writing a checksum file: every file in the items is read once and listed with its hash, in the format of
//! `sha256sum` (see `digest`). Links are skipped, never followed. The file is built in memory, written to a temporary
//! file and put in place with a rename that refuses to replace, so an existing file is never touched.

use std::fs::File;
use std::io::{self, Read, Write};

use crate::model::{HashAlgorithm, JobPhase};

use super::digest::{Digester, format_line};
use super::held::Held;
use super::partial::Partial;
use super::report::refusal;
use super::task::{Scene, as_text, create_plain, require_free};
use super::transfer::{Run, Stop};
use super::tree::{Spot, Visitor};

/// Bytes read between two checkpoints.
const CHUNK: usize = 1024 * 1024;

/// The walk of the items: a line for every file (see [`Run::walk_items`]).
struct Listing {
    algorithm: HashAlgorithm,
    lines: String,
}

impl Visitor for Listing {
    fn file(&mut self, run: &mut Run<'_>, at: &Spot<'_>) -> Result<(), Stop> {
        run.begin_file(at.rel, at.node.size);
        let done = self.hash_file(run, at);
        run.end_file();

        done
    }

    /// A link is not read, and counts as a finished file.
    fn link(&mut self, run: &mut Run<'_>, rel: &str) {
        run.begin_file(rel, 0);
        run.end_file();
    }
}

impl Listing {
    fn hash_file(&mut self, run: &mut Run<'_>, at: &Spot<'_>) -> Result<(), Stop> {
        // A name that is not text cannot be written to the file as it is.
        as_text(&at.node.name)?;

        let mut file = at.parent.open_file(at.node)?;
        let hex = run.digest_file(&mut file, self.algorithm)?;
        self.lines.push_str(&format_line(&hex, at.rel));

        Ok(())
    }
}

impl Run<'_> {
    /// Writes the checksum file `output` into the folder of the items, with a line for every file in the items of the
    /// job (folders are walked by handle, in name order).
    pub(super) fn write_checksums(
        &mut self,
        scene: &Scene,
        algorithm: HashAlgorithm,
        output: &str,
    ) -> Result<(), Stop> {
        let target = scene.to.join(output);
        require_free(&target)?;

        let parent = Held::root(&scene.from)?;
        let items = self.look_up_items(&parent);

        self.phase(JobPhase::Scanning);
        self.scan_nodes(&parent, &items)?;

        self.phase(JobPhase::Hashing);
        let mut listing = Listing {
            algorithm,
            lines: String::new(),
        };
        self.walk_items(&parent, &items, &mut listing)?;
        let lines = listing.lines;

        if lines.is_empty() {
            return Err(refusal(io::ErrorKind::InvalidInput, "there is no file to take a checksum of").into());
        }

        let (temp, mut file) = self.make_temp(&scene.to, create_plain)?;
        let mut guard = Partial::new(temp.clone());

        file.write_all(lines.as_bytes())?;
        drop(file);
        self.checkpoint()?;

        self.commit_new(&temp, &target)?;
        guard.committed = true;

        Ok(())
    }

    /// The hash of `file`, read from its start, stopping between chunks for a pause or a cancel.
    pub(super) fn digest_file(&mut self, file: &mut File, algorithm: HashAlgorithm) -> Result<String, Stop> {
        let mut digester = Digester::new(algorithm);
        let mut buffer = std::mem::take(&mut self.buffer);
        buffer.resize(CHUNK, 0);

        loop {
            self.checkpoint()?;

            let read = match file.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            };

            digester.update(&buffer[..read]);
            self.add_bytes(read as u64);
        }

        self.buffer = buffer;

        Ok(digester.finish())
    }
}
