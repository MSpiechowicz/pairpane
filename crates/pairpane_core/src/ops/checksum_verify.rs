//! Checking files against a checksum file. Every line is checked on its own and reports on its own: a file that
//! does not match, one that is missing and a line that cannot be read are item errors, and the job goes on.
//!
//! The paths in the file are relative to the folder the checksum file is in, and a path that is absolute, climbs
//! (`..`) or holds a name that is not plain is never opened. The way to a file is walked without following a link
//! (see `listed`): a link on it, or at the file, is refused.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::model::{HashAlgorithm, JobPhase};

use super::digest::{Listed, algorithm_of_length, algorithm_of_name, hex_len, parse_line, stays_inside};
use super::listed::open_listed;
use super::open;
use super::plan::Kind;
use super::report::refusal;
use super::task::{Scene, as_text};
use super::transfer::{Run, Stop};

/// The largest checksum file that is read, in bytes.
const MAX_FILE: u64 = 16 * 1024 * 1024;

/// The longest line that is read, in bytes; a longer one cannot be read.
const MAX_LINE: usize = 64 * 1024;

/// The most lines (not counting empty ones) a checksum file may have; a file of the largest size would otherwise take
/// hundreds of megabytes of memory as lines.
const MAX_LINES: usize = 1_000_000;

/// The UTF-8 byte order mark some editors put at the start of a file.
const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// One line of the file: its number (from 1) and what it says, or `None` when it cannot be read.
struct Line {
    number: usize,
    listed: Option<Listed>,
}

impl Run<'_> {
    /// Checks the files that the checksum file in `scene.items` (exactly one) lists.
    pub(super) fn verify_checksums(&mut self, scene: &Scene) -> Result<(), Stop> {
        let Some(item) = scene.items.first() else {
            return Ok(());
        };
        if item.kind != Kind::File {
            return Err(refusal(io::ErrorKind::InvalidInput, "the checksum file must be a regular file").into());
        }

        let mut file = open::source(&item.path, &item.meta)?;
        let mut bytes = Vec::new();
        file.by_ref().take(MAX_FILE + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE {
            return Err(refusal(io::ErrorKind::InvalidInput, "the checksum file is too large").into());
        }

        let lines = read_lines(&bytes)?;
        let algorithm = algorithm_of(as_text(&item.name)?, &lines)?;

        self.phase(JobPhase::Scanning);
        for line in &lines {
            self.checkpoint()?;
            let size = line
                .listed
                .as_ref()
                .and_then(|listed| open_listed(&scene.from, &listed.path).ok())
                .map_or(0, |(_, meta)| meta.len());

            self.found(size);
        }

        self.phase(JobPhase::Hashing);
        for line in &lines {
            self.checkpoint()?;

            match &line.listed {
                Some(listed) if listed.hex.len() == hex_len(algorithm) => {
                    self.verify_line(&scene.from, listed, algorithm)?;
                }
                _ => self.unreadable(line.number),
            }
        }

        Ok(())
    }

    fn unreadable(&mut self, number: usize) {
        let label = format!("line {number}");

        self.begin_file(&label, 0);
        self.fail(
            &label,
            &refusal(io::ErrorKind::InvalidInput, "this line cannot be read"),
        );
        self.end_file();
    }

    /// Checks the file one line lists; a failure of the file is the line's item error.
    fn verify_line(&mut self, base: &Path, listed: &Listed, algorithm: HashAlgorithm) -> Result<(), Stop> {
        let opened = open_listed(base, &listed.path);
        self.begin_file(&listed.path, opened.as_ref().map_or(0, |(_, meta)| meta.len()));

        let checked = match opened {
            Ok((file, _)) => self.check_file(file, listed, algorithm),
            Err(error) => Err(Stop::Failed(error)),
        };
        self.end_file();

        match checked {
            Err(Stop::Failed(error)) => {
                self.fail(&listed.path, &error);

                Ok(())
            }
            other => other,
        }
    }

    fn check_file(&mut self, mut file: File, listed: &Listed, algorithm: HashAlgorithm) -> Result<(), Stop> {
        if self.digest_file(&mut file, algorithm)? == listed.hex {
            return Ok(());
        }

        Err(refusal(io::ErrorKind::InvalidData, "the checksum does not match").into())
    }
}

/// Which algorithm the file uses: by the extension of its name, else by the length of the first hash it holds.
fn algorithm_of(name: &str, lines: &[Line]) -> Result<HashAlgorithm, Stop> {
    algorithm_of_name(name)
        .or_else(|| {
            lines
                .iter()
                .find_map(|line| line.listed.as_ref())
                .and_then(|listed| algorithm_of_length(listed.hex.len()))
        })
        .ok_or_else(|| refusal(io::ErrorKind::InvalidData, "the kind of checksum cannot be told").into())
}

/// The lines of the checksum file's `bytes`, without the empty ones, after a byte order mark that starts the file. A
/// line that is not text, is too long, is not a line of a checksum file or lists a path that leaves the folder has no
/// `listed`. More than [`MAX_LINES`] lines fail the whole file.
fn read_lines(bytes: &[u8]) -> Result<Vec<Line>, Stop> {
    let bytes = bytes.strip_prefix(BOM).unwrap_or(bytes);
    let mut lines = Vec::new();

    for (index, raw) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        if lines.len() == MAX_LINES {
            return Err(refusal(io::ErrorKind::InvalidInput, "the checksum file has too many lines").into());
        }

        let listed = std::str::from_utf8(raw)
            .ok()
            .filter(|_| raw.len() <= MAX_LINE)
            .and_then(parse_line)
            .filter(|listed| stays_inside(&listed.path));

        lines.push(Line {
            number: index + 1,
            listed,
        });
    }

    Ok(lines)
}
