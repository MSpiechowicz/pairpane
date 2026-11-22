//! Walking folders by handle: the checksum file and the attribute change go through a tree holding each folder open, so
//! a folder swapped for a link while the walk is under way cannot lead it somewhere else.
//!
//! Unix: the job's folder is opened once (following a link there, as the pane showed it), and from then on every step
//! is relative to a handle: an entry is looked at (`fstatat`, no-follow), a subfolder is opened from its parent
//! (`openat`, no-follow, folders only) and compared with what was seen, a folder is listed through its own handle, and
//! a file is opened from its folder (`openat`, no-follow) and compared. No path is ever passed to an open or a listing,
//! so there is no path to swap. Windows keeps the path-based walk (a path is held, and the steps read it): a folder
//! swapped for a link between the look and the open is the race the other Windows jobs document.

use std::ffi::OsString;
use std::io;

use super::plan::Kind;
use super::transfer::{MAX_DEPTH, Run, Stop};

#[cfg(unix)]
use std::fs::File;
#[cfg(windows)]
use std::fs::{File, Metadata};
#[cfg(windows)]
use std::path::PathBuf;

use super::open;
#[cfg(unix)]
use super::report::refusal;

/// One entry of a folder, as the walk saw it. A link is a link whatever it points at.
#[derive(Clone)]
pub(super) struct Node {
    pub(super) name: OsString,
    pub(super) kind: Kind,
    /// The size in bytes (what a file holds; of no use for the rest).
    pub(super) size: u64,
    /// The Unix permission bits (`0o755`, with setuid, setgid and sticky); 0 on Windows.
    pub(super) mode: u32,
    /// The last modification in milliseconds since the Unix epoch (negative before it), when the filesystem says.
    pub(super) modified: Option<i64>,
    /// The filesystem it is on (Unix).
    pub(super) device: Option<u64>,
    /// Device and inode (Unix): what the entry is, to compare with what a handle opened.
    #[cfg(unix)]
    pub(super) id: Option<(u64, u64)>,
    #[cfg(windows)]
    pub(super) path: PathBuf,
    #[cfg(windows)]
    pub(super) meta: Metadata,
}

impl Node {
    /// The name as the job reports it (for a top-level item, its path relative to the folder of the items).
    pub(super) fn rel(&self) -> String {
        self.name.to_string_lossy().into_owned()
    }
}

/// What listing a folder found.
#[derive(Default)]
pub(super) struct Entries {
    pub(super) nodes: Vec<Node>,
    /// Entries that could not be read.
    pub(super) failed: Vec<io::Error>,
    /// Entries left out because Windows would read their name as something else.
    pub(super) skipped: u32,
}

#[cfg(unix)]
fn changed() -> io::Error {
    refusal(io::ErrorKind::Other, "it changed while the folder was being walked")
}

/// A folder held open (a path on Windows).
#[cfg(unix)]
pub(super) struct Held {
    pub(super) dir: File,
}

#[cfg(windows)]
pub(super) struct Held {
    path: PathBuf,
}

#[cfg(unix)]
impl Node {
    /// The entry `name` as `stat` describes it.
    #[allow(
        clippy::unnecessary_cast,
        reason = "the device, inode, mode and size are as wide as these types on some systems and narrower on others"
    )]
    pub(super) fn of_stat(name: OsString, stat: &rustix::fs::Stat) -> Self {
        use rustix::fs::FileType;

        let kind = match FileType::from_raw_mode(stat.st_mode) {
            FileType::Directory => Kind::Dir,
            FileType::RegularFile => Kind::File,
            FileType::Symlink => Kind::Link,
            _ => Kind::Other,
        };
        let (device, inode) = (stat.st_dev as u64, stat.st_ino as u64);

        Self {
            name,
            kind,
            size: u64::try_from(stat.st_size).unwrap_or(0),
            mode: stat.st_mode as u32 & 0o7777,
            modified: Some((stat.st_mtime as i64).saturating_mul(1000) + stat.st_mtime_nsec as i64 / 1_000_000),
            device: Some(device),
            id: Some((device, inode)),
        }
    }

    /// Whether `stat` describes this very entry: the same kind, device and inode.
    pub(super) fn is(&self, stat: &rustix::fs::Stat) -> bool {
        let now = Self::of_stat(OsString::new(), stat);

        now.kind == self.kind && now.id == self.id
    }
}

#[cfg(unix)]
impl Held {
    /// Opens the folder the job's items are in, which the pane showed under `path`: a link to a folder is followed.
    pub(super) fn root(path: &std::path::Path) -> io::Result<Self> {
        Ok(Self {
            dir: open::root_folder(path)?,
        })
    }

    /// The entry `name` of this folder, looked at without following a link.
    pub(super) fn look(&self, name: &str) -> io::Result<Node> {
        use rustix::fs::{AtFlags, statat};

        let stat = statat(&self.dir, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io::Error::from)?;

        Ok(Node::of_stat(OsString::from(name), &stat))
    }

    /// Opens the subfolder `node` of this folder, which must still be the one that was seen: a link, or another
    /// folder in its place, is refused.
    pub(super) fn subfolder(&self, node: &Node) -> io::Result<Self> {
        let dir = self.open_entry(node)?;

        Ok(Self { dir })
    }

    /// Opens the regular file `node` of this folder for reading, which must still be the one that was seen.
    pub(super) fn open_file(&self, node: &Node) -> io::Result<File> {
        if node.kind != Kind::File {
            return Err(refusal(io::ErrorKind::InvalidInput, "it is not a regular file"));
        }

        self.open_entry(node)
    }

    /// Opens `node` without following a link, as the kind it was seen as (a pipe is never waited on, a terminal is never
    /// taken), and checks that what was opened is that entry.
    pub(super) fn open_entry(&self, node: &Node) -> io::Result<File> {
        use rustix::fs::{Mode, OFlags, fstat, openat};

        let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        match node.kind {
            Kind::Dir => flags |= OFlags::DIRECTORY,
            Kind::File => flags |= OFlags::NONBLOCK | OFlags::NOCTTY,
            Kind::Link | Kind::Other => return Err(changed()),
        }

        let file = File::from(openat(&self.dir, &node.name, flags, Mode::empty()).map_err(io::Error::from)?);
        if !node.is(&fstat(&file).map_err(io::Error::from)?) {
            return Err(changed());
        }

        Ok(file)
    }

    /// Lists this folder through its handle.
    pub(super) fn entries(&self) -> io::Result<Entries> {
        use rustix::fs::{AtFlags, Dir, statat};
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let mut dir = Dir::read_from(&self.dir).map_err(io::Error::from)?;
        dir.rewind();
        let mut found = Entries::default();

        for entry in dir {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    found.failed.push(io::Error::from(error));
                    continue;
                }
            };
            let name = OsStr::from_bytes(entry.file_name().to_bytes());
            if name == "." || name == ".." {
                continue;
            }

            match statat(&self.dir, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => found.nodes.push(Node::of_stat(name.to_owned(), &stat)),
                Err(error) => found.failed.push(io::Error::from(error)),
            }
        }

        Ok(found)
    }
}

/// Milliseconds since the Unix epoch (negative before it).
#[cfg(windows)]
fn millis(time: std::time::SystemTime) -> Option<i64> {
    use std::time::UNIX_EPOCH;

    let (sign, span) = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => (1, after),
        Err(before) => (-1, before.duration()),
    };

    i64::try_from(span.as_millis()).ok().map(|millis| sign * millis)
}

#[cfg(windows)]
impl From<super::plan::Child> for Node {
    fn from(child: super::plan::Child) -> Self {
        Self {
            size: child.meta.len(),
            mode: 0,
            modified: child.meta.modified().ok().and_then(millis),
            device: crate::listing::platform::device(&child.meta),
            name: child.name,
            kind: child.kind,
            path: child.path,
            meta: child.meta,
        }
    }
}

#[cfg(windows)]
impl Held {
    pub(super) fn root(path: &std::path::Path) -> io::Result<Self> {
        Ok(Self { path: path.to_owned() })
    }

    pub(super) fn look(&self, name: &str) -> io::Result<Node> {
        super::plan::Child::lookup(&self.path, name).map(Node::from)
    }

    pub(super) fn subfolder(&self, node: &Node) -> io::Result<Self> {
        Ok(Self {
            path: node.path.clone(),
        })
    }

    pub(super) fn open_file(&self, node: &Node) -> io::Result<File> {
        open::source(&node.path, &node.meta)
    }

    pub(super) fn entries(&self) -> io::Result<Entries> {
        let listing = super::plan::read_children(&self.path)?;

        Ok(Entries {
            nodes: listing.children.into_iter().map(Node::from).collect(),
            failed: listing.failed,
            skipped: listing.skipped,
        })
    }
}

impl Run<'_> {
    /// Looks up the items of the job in the folder `held`; one that is gone is reported and left out.
    pub(super) fn look_up_items(&mut self, held: &Held) -> Vec<Node> {
        let job = self.job;
        let mut items = Vec::with_capacity(job.request.names.len());

        for name in &job.request.names {
            match held.look(name) {
                Ok(node) => items.push(node),
                Err(error) => self.fail(name, &error),
            }
        }

        items
    }

    /// Opens the folder `node` of `held` and lists it, for a walk: what could not be read is reported under `rel`, and
    /// what was left out is counted as skipped. Returns the folder's own handle, which the entries are relative to.
    pub(super) fn open_and_list(&mut self, held: &Held, node: &Node, rel: &str) -> Result<(Held, Vec<Node>), Stop> {
        // A test swaps the folder here, between the look at it and the open.
        if let Some(before) = &self.inner.hooks.before_open {
            before(rel);
        }

        let folder = held.subfolder(node)?;
        let entries = folder.entries()?;

        self.skipped += entries.skipped;
        for error in &entries.failed {
            self.fail(rel, error);
        }

        Ok((folder, entries.nodes))
    }

    /// Whether the walk may enter the folder `node`: not one walked already (a bind mount can show one folder twice).
    pub(super) fn first_visit_node(&mut self, node: &Node) -> bool {
        #[cfg(unix)]
        {
            node.id
                .is_none_or(|id| self.visited.insert(super::identity::FileId::of_ids(id)))
        }
        #[cfg(windows)]
        {
            let _ = node;

            true
        }
    }

    /// Counts the files, links and bytes below `nodes` (in the folder `held`), so the progress has its total.
    pub(super) fn scan_nodes(&mut self, held: &Held, nodes: &[Node]) -> Result<(), Stop> {
        self.visited.clear();

        for node in nodes {
            self.checkpoint()?;
            self.scan_held(held, node, 0)?;
        }

        self.visited.clear();

        Ok(())
    }

    fn scan_held(&mut self, held: &Held, node: &Node, depth: usize) -> Result<(), Stop> {
        match node.kind {
            Kind::File => self.found(node.size),
            Kind::Link => self.found(0),
            Kind::Other => {}
            Kind::Dir => {
                if depth >= MAX_DEPTH || !self.first_visit_node(node) {
                    return Ok(());
                }

                // What cannot be read now is reported by the walk that follows, which reads it again.
                let Ok(folder) = held.subfolder(node) else {
                    return Ok(());
                };
                let Ok(entries) = folder.entries() else {
                    return Ok(());
                };

                for child in &entries.nodes {
                    self.checkpoint()?;
                    self.scan_held(&folder, child, depth + 1)?;
                }
            }
        }

        Ok(())
    }
}
