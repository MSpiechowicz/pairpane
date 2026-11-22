//! Reading an ignore file within its limits and compiling its lines.

use std::fs;
use std::io::{ErrorKind, Read};
use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

use super::{MAX_LINE, MAX_LINES};

/// The biggest ignore file that is read, in bytes.
pub(super) const MAX_BYTES: u64 = 256 * 1024;

/// The path of `name` in the folder `below` the one searched, names joined with `/`; `None` when a name is not valid Unicode.
pub(super) fn listed(below: &Path, name: &str) -> Option<String> {
    let mut listed = String::new();

    for part in below.components() {
        listed.push_str(part.as_os_str().to_str()?);
        listed.push('/');
    }
    listed.push_str(name);

    Some(listed)
}

/// Whether the name `name` of the folder `dir` is there, whatever it is (a link is not followed): one look, where opening
/// it step by step costs a call for each folder on the way. An empty file holds no rule, so it is as good as not there:
/// it takes no room and is never counted as left out. (A file system that reports a size of 0 for a file that has some,
/// as `/proc` does, has its ignore files taken as empty: no project keeps its rules there.)
pub(super) fn present(dir: &Path, name: &str) -> bool {
    match fs::symlink_metadata(dir.join(name)) {
        Ok(found) => !(found.is_file() && found.len() == 0),
        Err(error) => error.kind() != ErrorKind::NotFound,
    }
}

/// What `file` holds up to [`MAX_BYTES`]; `None` when it holds more, or cannot be read.
pub(super) fn read_limited(file: fs::File) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut limited = file.take(MAX_BYTES + 1);

    limited.read_to_end(&mut bytes).ok()?;

    (bytes.len() as u64 <= MAX_BYTES).then_some(bytes)
}

/// The lines of `bytes`: a final newline ends the last line, it does not start another.
fn lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let body = bytes.strip_suffix(b"\n").unwrap_or(bytes);

    body.split(|byte| *byte == b'\n').filter(|_| !bytes.is_empty())
}

/// How many lines `bytes` has; `None` when there are too many or a line is too long.
pub(super) fn count_lines(bytes: &[u8]) -> Option<usize> {
    let mut count = 0;

    for line in lines(bytes) {
        count += 1;
        if count > MAX_LINES || line.len() > MAX_LINE {
            return None;
        }
    }

    Some(count)
}

/// The rules the lines of `bytes` (already within the limits) make, as if the file was in `dir`. `None` when the rules
/// cannot be built. A line that is not a valid pattern is left out, as git does.
pub(super) fn parse(dir: &Path, bytes: &[u8]) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(dir);

    for line in lines(bytes) {
        builder.add_line(None, &String::from_utf8_lossy(line)).ok();
    }

    builder.build().ok()
}
