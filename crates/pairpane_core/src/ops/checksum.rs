//! Writing a checksum file: every file in the items is read once and listed with its hash, in the format of
//! `sha256sum` (see `digest`). Links are skipped, never followed. The file is built in memory, written to a temporary
//! file and put in place with a rename that refuses to replace, so an existing file is never touched.

use std::fs::File;
use std::io::{self, Read, Write};

use crate::model::{HashAlgorithm, JobPhase};

use super::digest::{Digester, format_line};
use super::held::{Held, Node};
use super::partial::Partial;
use super::plan::Kind;
use super::report::refusal;
use super::task::{Scene, as_text, create_plain, require_free};
use super::transfer::{MAX_DEPTH, Run, Stop};

/// Bytes read between two checkpoints.
const CHUNK: usize = 1024 * 1024;

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
        let mut lines = String::new();
        for item in &items {
            self.checkpoint()?;
            let rel = item.rel();

            let done = self.hash_node(&parent, item, &rel, algorithm, &mut lines, 0);
            self.settle(&rel, done)?;
        }

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

    /// Adds the lines for `node` (and what is below it) to `lines`. `rel` is its path relative to the folder of the
    /// items.
    fn hash_node(
        &mut self,
        parent: &Held,
        node: &Node,
        rel: &str,
        algorithm: HashAlgorithm,
        lines: &mut String,
        depth: usize,
    ) -> Result<(), Stop> {
        match node.kind {
            Kind::Dir => self.hash_folder(parent, node, rel, algorithm, lines, depth),
            Kind::File => {
                self.begin_file(rel, node.size);
                let done = self.hash_file(parent, node, rel, algorithm, lines);
                self.end_file();

                done
            }
            Kind::Link => {
                self.skipped += 1;
                self.begin_file(rel, 0);
                self.end_file();

                Ok(())
            }
            Kind::Other => {
                self.skipped += 1;

                Ok(())
            }
        }
    }

    fn hash_folder(
        &mut self,
        parent: &Held,
        node: &Node,
        rel: &str,
        algorithm: HashAlgorithm,
        lines: &mut String,
        depth: usize,
    ) -> Result<(), Stop> {
        if depth >= MAX_DEPTH {
            return Err(refusal(io::ErrorKind::InvalidInput, "the folders are nested too deeply").into());
        }
        if !self.first_visit_node(node) {
            self.skipped += 1;

            return Ok(());
        }

        let (held, mut children) = self.open_and_list(parent, node, rel)?;

        // The order of a folder is the system's own; the file lists names in order, the same way every time.
        children.sort_by(|a, b| a.name.cmp(&b.name));

        for child in &children {
            self.checkpoint()?;
            let child_rel = format!("{rel}/{}", child.rel());

            let done = self.hash_node(&held, child, &child_rel, algorithm, lines, depth + 1);
            self.settle(&child_rel, done)?;
        }

        Ok(())
    }

    fn hash_file(
        &mut self,
        parent: &Held,
        node: &Node,
        rel: &str,
        algorithm: HashAlgorithm,
        lines: &mut String,
    ) -> Result<(), Stop> {
        // A name that is not text cannot be written to the file as it is.
        as_text(&node.name)?;

        let mut file = parent.open_file(node)?;
        let hex = self.digest_file(&mut file, algorithm)?;
        lines.push_str(&format_line(&hex, rel));

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
