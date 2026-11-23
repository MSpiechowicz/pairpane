//! What the parent and the helper say to each other (`helper`, `child`): the request on the helper's standard input and the
//! answer on its standard output, both small and made of nothing but integers and length-prefixed byte strings, so that
//! reading one cannot ask for more than its length allows.
//!
//! - The request is the folder asked for (a real path), the [`Limits`] and the [`Layout`] that the parent looked at: the exact
//!   folders the helper may read, which it confines itself to and which gix is to open, each with the device and inode it had
//!   when it was looked at ([`Grant`]), so that the helper can tell a folder that was swapped since. It is at most
//!   [`MAX_REQUEST`] bytes.
//! - The answer is a tag, and for a status the repository flag and the marked names. It holds no path: an error is only its
//!   kind (see [`GitError`]). The parent reads at most [`MAX_ANSWER`] bytes of it and never trusts it: a name that is not one
//!   plain name, a name told twice, a mark it does not know, more marks than the entries of the status may be (`Limits::entries`),
//!   a count that the bytes cannot hold or bytes after the end are an answer that cannot be read (`GitError::Repository`).

use std::borrow::Cow;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gix::bstr::ByteSlice;

use super::layout::{Grant, Layout};
use super::{GitError, Limits};
use crate::listing::platform::is_plain_name;
use crate::model::{GitEntry, GitMark, GitStatus};

/// The most a request may hold, in bytes.
pub(super) const MAX_REQUEST: u64 = 1024 * 1024;

/// The most an answer may hold, in bytes: a name is at most 4 KiB and the marks are as many as the entries of a folder.
pub(super) const MAX_ANSWER: u64 = 8 * 1024 * 1024;

/// The longest time limit a request may carry: the processor-time limit of the helper is set from it.
pub(super) const MAX_TIME: Duration = Duration::from_secs(60);

/// The longest name an answer may hold.
const MAX_NAME: usize = 4 * 1024;

/// The version of the request, which is its first byte.
const VERSION: u8 = 1;

/// What a status is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Request {
    /// The folder, as a real path.
    pub(super) folder: PathBuf,
    pub(super) limits: Limits,
    pub(super) layout: Layout,
    /// The folders of the layout that the helper may read, with what they were when they were looked at.
    pub(super) grants: Vec<Grant>,
}

fn invalid(text: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, text)
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> io::Result<()> {
    let length = u32::try_from(bytes.len()).map_err(|_| invalid("a string is too long"))?;
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(bytes);

    Ok(())
}

fn put_path(out: &mut Vec<u8>, path: &Path) -> io::Result<()> {
    let bytes = gix::path::into_bstr(Cow::Borrowed(path)).map_err(|_| invalid("a path is not text"))?;

    put_bytes(out, bytes.as_bytes())
}

/// A reader of the request or the answer: each read takes from what is left and fails when it is not there.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        let (head, rest) = self.0.split_at_checked(count).ok_or_else(|| invalid("it ends early"))?;
        self.0 = rest;

        Ok(head)
    }

    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| invalid("it ends early"))?,
        ))
    }

    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().map_err(|_| invalid("it ends early"))?,
        ))
    }

    /// A string: its length, then its bytes, which have to be there (nothing is reserved for a length that is not).
    fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let length = usize::try_from(self.u32()?).map_err(|_| invalid("a string is too long"))?;

        self.take(length)
    }

    /// An absolute path.
    fn path(&mut self) -> io::Result<PathBuf> {
        let bytes = self.bytes()?;
        let path = gix::path::from_byte_slice(bytes).map_err(|_| invalid("a path is not text"))?;
        if !path.is_absolute() || bytes.contains(&0) {
            return Err(invalid("a path is not absolute"));
        }

        Ok(path.to_path_buf())
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Request {
    pub(super) fn encode(&self) -> io::Result<Vec<u8>> {
        let mut out = vec![VERSION];
        let time = u64::try_from(self.limits.time.as_millis()).map_err(|_| invalid("the time is too long"))?;
        let entries = u64::try_from(self.limits.entries).map_err(|_| invalid("too many entries"))?;
        for number in [time, entries, self.limits.compare_bytes] {
            out.extend_from_slice(&number.to_le_bytes());
        }

        put_path(&mut out, &self.folder)?;
        let layout = &self.layout;
        for path in [&layout.dot_git, &layout.git_dir, &layout.common_dir, &layout.workdir] {
            put_path(&mut out, path)?;
        }
        out.extend_from_slice(
            &u32::try_from(layout.alternates.len())
                .map_err(|_| invalid("too many"))?
                .to_le_bytes(),
        );
        for path in &layout.alternates {
            put_path(&mut out, path)?;
        }
        out.extend_from_slice(&layout.info_rules.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(self.grants.len())
                .map_err(|_| invalid("too many"))?
                .to_le_bytes(),
        );
        for grant in &self.grants {
            put_path(&mut out, &grant.path)?;
            out.extend_from_slice(&grant.device.to_le_bytes());
            out.extend_from_slice(&grant.inode.to_le_bytes());
        }

        if out.len() as u64 > MAX_REQUEST {
            return Err(invalid("the request is too long"));
        }

        Ok(out)
    }

    /// The request in `bytes`, or an error for one that is not exactly a request (the helper then ends without an answer).
    pub(super) fn decode(bytes: &[u8]) -> io::Result<Self> {
        let mut input = Reader(bytes);
        if input.u8()? != VERSION {
            return Err(invalid("another version"));
        }

        let time = Duration::from_millis(input.u64()?);
        let entries = usize::try_from(input.u64()?).map_err(|_| invalid("too many entries"))?;
        let compare_bytes = input.u64()?;
        if time > MAX_TIME {
            return Err(invalid("the time is too long"));
        }

        let folder = input.path()?;
        let dot_git = input.path()?;
        let git_dir = input.path()?;
        let common_dir = input.path()?;
        let workdir = input.path()?;
        let mut alternates = Vec::new();
        for _ in 0..input.u32()? {
            alternates.push(input.path()?);
        }
        let info_rules = input.u64()?;
        let mut grants = Vec::new();
        for _ in 0..input.u32()? {
            grants.push(Grant {
                path: input.path()?,
                device: input.u64()?,
                inode: input.u64()?,
            });
        }
        if !input.is_empty() {
            return Err(invalid("there is more than the request"));
        }

        Ok(Self {
            folder,
            limits: Limits {
                time,
                entries,
                compare_bytes,
            },
            layout: Layout {
                dot_git,
                git_dir,
                common_dir,
                workdir,
                alternates,
                info_rules,
            },
            grants,
        })
    }
}

const TAG_STATUS: u8 = 0;
const TAG_TOO_BIG: u8 = 1;
const TAG_TIMED_OUT: u8 = 2;
const TAG_CANCELLED: u8 = 3;
const TAG_REPOSITORY: u8 = 4;
const TAG_NOT_FOUND: u8 = 5;
const TAG_NO_PERMISSION: u8 = 6;
const TAG_NOT_A_FOLDER: u8 = 7;
const TAG_FOLDER: u8 = 8;

fn mark_code(mark: GitMark) -> u8 {
    match mark {
        GitMark::Modified => 0,
        GitMark::Staged => 1,
        GitMark::Untracked => 2,
        GitMark::Ignored => 3,
        GitMark::Conflicted => 4,
    }
}

fn mark_of(code: u8) -> Option<GitMark> {
    Some(match code {
        0 => GitMark::Modified,
        1 => GitMark::Staged,
        2 => GitMark::Untracked,
        3 => GitMark::Ignored,
        4 => GitMark::Conflicted,
        _ => return None,
    })
}

/// The bytes the helper writes for `result`.
pub(super) fn encode_answer(result: &Result<GitStatus, GitError>) -> Vec<u8> {
    let status = match result {
        Ok(status) => status,
        Err(error) => {
            return vec![match error {
                GitError::TooBig => TAG_TOO_BIG,
                GitError::TimedOut => TAG_TIMED_OUT,
                GitError::Cancelled => TAG_CANCELLED,
                GitError::Repository => TAG_REPOSITORY,
                GitError::Folder(io::ErrorKind::NotFound) => TAG_NOT_FOUND,
                GitError::Folder(io::ErrorKind::PermissionDenied) => TAG_NO_PERMISSION,
                GitError::Folder(io::ErrorKind::NotADirectory) => TAG_NOT_A_FOLDER,
                GitError::Folder(_) => TAG_FOLDER,
            }];
        }
    };

    let mut out = vec![TAG_STATUS, u8::from(status.repo)];
    let count = u32::try_from(status.marks.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&count.to_le_bytes());
    for entry in &status.marks {
        out.push(mark_code(entry.mark));
        // A name that cannot be put is left out: the answer is a hint of what changed, never a reason to fail.
        let _ = put_bytes(&mut out, entry.name.as_bytes());
    }

    out
}

/// The result an answer stands for, with at most `marks` marks. `Err(GitError::Repository)` for anything that is not exactly an
/// answer (see the module).
pub(super) fn decode_answer(bytes: &[u8], marks: usize) -> Result<GitStatus, GitError> {
    let mut input = Reader(bytes);
    let unreadable = |_: io::Error| GitError::Repository;

    let tag = input.u8().map_err(unreadable)?;
    let error = match tag {
        TAG_STATUS => return decode_status(input, marks),
        TAG_TOO_BIG => GitError::TooBig,
        TAG_TIMED_OUT => GitError::TimedOut,
        TAG_CANCELLED => GitError::Cancelled,
        TAG_REPOSITORY => GitError::Repository,
        TAG_NOT_FOUND => GitError::Folder(io::ErrorKind::NotFound),
        TAG_NO_PERMISSION => GitError::Folder(io::ErrorKind::PermissionDenied),
        TAG_NOT_A_FOLDER => GitError::Folder(io::ErrorKind::NotADirectory),
        TAG_FOLDER => GitError::Folder(io::ErrorKind::Other),
        _ => return Err(GitError::Repository),
    };

    // An error stands alone.
    if input.is_empty() {
        Err(error)
    } else {
        Err(GitError::Repository)
    }
}

fn decode_status(mut input: Reader<'_>, most: usize) -> Result<GitStatus, GitError> {
    let unreadable = |_: io::Error| GitError::Repository;

    let repo = match input.u8().map_err(unreadable)? {
        0 => false,
        1 => true,
        _ => return Err(GitError::Repository),
    };
    // A status has at most one mark for each entry it walked, and so no more than its cap on entries.
    let count = usize::try_from(input.u32().map_err(unreadable)?).map_err(|_| GitError::Repository)?;
    if count > most {
        return Err(GitError::Repository);
    }
    let mut marks: Vec<GitEntry> = Vec::new();

    for _ in 0..count {
        let mark = mark_of(input.u8().map_err(unreadable)?).ok_or(GitError::Repository)?;
        let name = input.bytes().map_err(unreadable)?;
        let name = std::str::from_utf8(name).map_err(|_| GitError::Repository)?;
        if name.len() > MAX_NAME || !is_plain_name(name) {
            return Err(GitError::Repository);
        }

        marks.push(GitEntry {
            name: name.to_owned(),
            mark,
        });
    }
    if !input.is_empty() || (!repo && !marks.is_empty()) {
        return Err(GitError::Repository);
    }

    // At most one entry for a name.
    marks.sort_by(|left, right| left.name.cmp(&right.name));
    if marks.windows(2).any(|pair| pair[0].name == pair[1].name) {
        return Err(GitError::Repository);
    }

    Ok(GitStatus {
        repo,
        marks,
        note: None,
    })
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
