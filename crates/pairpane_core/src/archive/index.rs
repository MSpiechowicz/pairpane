//! The index of an archive: its entries in archive order and the tree of folders they make, so a pane can list any
//! folder at once and an extraction can find everything below one.
//!
//! Rules of the tree. Folders that no entry names (`a/` of a lone `a/b.txt`) exist as implicit folders. An entry with
//! the same path as an earlier one replaces it (the last wins, as when the archive is extracted over itself). A folder
//! wins over a file of the same path. An entry with a name that [`names::normalize`] refuses is kept in
//! [`ArchiveIndex::entries`] and counted in [`ArchiveIndex::refused`], but is not in the tree and so can never be listed
//! or selected. A link or special entry is in the tree (a pane lists it) and is skipped, and counted, by [`Selection`].

use std::collections::HashMap;

use crate::model::ArchiveFormat;

use super::entries::{EntryType, Flow, Place, Record};
use super::names::{self, Unsafe};
use super::{MAX_INDEX_ENTRIES, MAX_INDEX_NAME_BYTES};

/// One entry of an archive, with its name checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    /// The normalized path (`a/b/c`, see [`names::normalize`]); `""` for the archive's own root entry; the name as
    /// written when it is [`refused`](IndexEntry::refused).
    pub path: String,
    pub kind: EntryType,
    pub size: u64,
    pub modified: Option<i64>,
    /// Permission bits as the archive stores them (setuid, setgid and sticky not yet stripped).
    pub mode: Option<u32>,
    pub link: Option<String>,
    pub encrypted: bool,
    /// Its position among the entries of the archive: the index into [`ArchiveIndex::entries`], and what an
    /// [`EntrySource`](super::EntrySource) is asked for.
    pub ordinal: u32,
    pub place: Place,
    /// Why the name was refused; the entry is then never extracted.
    pub refused: Option<Unsafe>,
}

impl IndexEntry {
    /// The last part of the path.
    pub fn name(&self) -> &str {
        names::file_name(&self.path)
    }

    /// Whether extraction writes it: a safe file or folder (not a link or a special entry).
    pub fn writable(&self) -> bool {
        self.refused.is_none() && !self.path.is_empty() && matches!(self.kind, EntryType::File | EntryType::Dir)
    }
}

#[derive(Debug)]
struct Node {
    name: String,
    dir: bool,
    entry: Option<u32>,
    children: Vec<u32>,
}

/// The entries of an archive and the folder tree, built once (see [`Builder`]) and then only read, so any number of
/// threads share one behind an `Arc`.
#[derive(Debug)]
pub struct ArchiveIndex {
    format: ArchiveFormat,
    entries: Vec<IndexEntry>,
    nodes: Vec<Node>,
    paths: HashMap<String, u32>,
    refused: Vec<u32>,
    truncated: bool,
}

/// One name in a folder of the archive.
#[derive(Debug, Clone, Copy)]
pub struct Child<'a> {
    pub name: &'a str,
    pub dir: bool,
    /// The entry that makes it; `None` for a folder that only exists because of what is below it.
    pub entry: Option<&'a IndexEntry>,
}

/// An entry chosen for extraction and where it goes.
#[derive(Debug, Clone, Copy)]
pub struct Selected<'a> {
    pub entry: &'a IndexEntry,
    /// Its path below the folder it was selected from (`name/rest`), with the selected name first.
    pub relative: &'a str,
}

/// What [`ArchiveIndex::select`] chose.
#[derive(Debug)]
pub struct Selection<'a> {
    /// The files and folders to write, in archive order.
    pub items: Vec<Selected<'a>>,
    /// Links and special entries that are left out.
    pub skipped: u32,
    /// Entries whose names are refused and that lie in what was selected.
    pub refused: u32,
}

impl Selection<'_> {
    /// The ordinals of the items, ascending: what [`EntrySource::extract`](super::EntrySource::extract) wants.
    pub fn ordinals(&self) -> Vec<u32> {
        self.items.iter().map(|item| item.entry.ordinal).collect()
    }

    /// Whether the content of any chosen entry is encrypted.
    pub fn any_encrypted(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.entry.encrypted && item.entry.kind == EntryType::File)
    }
}

impl ArchiveIndex {
    pub fn format(&self) -> ArchiveFormat {
        self.format
    }

    /// Every entry, in archive order, refused ones and shadowed ones included.
    pub fn entries(&self) -> &[IndexEntry] {
        &self.entries
    }

    /// The archive held more than the index limits allow, or could not be read to its end; the index is what was read.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// How many entries have a refused name.
    pub fn refused(&self) -> usize {
        self.refused.len()
    }

    /// Whether `dir` (`""` is the root, `a/b` a folder) is a folder of the archive.
    pub fn has_dir(&self, dir: &str) -> bool {
        self.node(dir).is_some_and(|node| self.nodes[node as usize].dir)
    }

    /// The names in the folder `dir`, in archive order; `None` when `dir` is not a folder of the archive.
    pub fn list(&self, dir: &str) -> Option<Vec<Child<'_>>> {
        let node = self.node(dir).filter(|node| self.nodes[*node as usize].dir)?;

        Some(
            self.nodes[node as usize]
                .children
                .iter()
                .map(|child| self.child(*child))
                .collect(),
        )
    }

    /// The file or folder `name` in the folder `dir`.
    pub fn find(&self, dir: &str, name: &str) -> Option<Child<'_>> {
        let path = if dir.is_empty() {
            name.to_owned()
        } else {
            format!("{dir}/{name}")
        };

        self.node(&path).map(|node| self.child(node))
    }

    /// Chooses what extracting `names` from the folder `dir` writes: those names and everything below the folders among
    /// them, or, when `names` is empty, everything in `dir`. `None` when `dir` or one of the names is not there.
    pub fn select(&self, dir: &str, names: &[String]) -> Option<Selection<'_>> {
        let folder = self.node(dir).filter(|node| self.nodes[*node as usize].dir)?;
        let mut roots: Vec<u32> = Vec::new();

        if names.is_empty() {
            roots.extend(self.nodes[folder as usize].children.iter().copied());
        } else {
            for name in names {
                roots.push(*self.paths.get(&join(dir, name))?);
            }
        }

        let prefix = if dir.is_empty() { 0 } else { dir.len() + 1 };
        let mut picked: Vec<&IndexEntry> = Vec::new();
        let mut skipped = 0;
        let mut stack = roots;

        while let Some(id) = stack.pop() {
            let node = &self.nodes[id as usize];
            stack.extend(node.children.iter().copied());

            let Some(entry) = node.entry.map(|ordinal| &self.entries[ordinal as usize]) else {
                continue;
            };

            if entry.writable() {
                picked.push(entry);
            } else {
                skipped += 1;
            }
        }

        picked.sort_by_key(|entry| entry.ordinal);

        let refused = self.refused_below(dir, names);

        Some(Selection {
            items: picked
                .into_iter()
                .map(|entry| Selected {
                    entry,
                    relative: &entry.path[prefix..],
                })
                .collect(),
            skipped,
            refused,
        })
    }

    fn refused_below(&self, dir: &str, names: &[String]) -> u32 {
        let below = |raw: &str, folder: &str| {
            folder.is_empty() || raw.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))
        };
        let wanted: Vec<String> = if names.is_empty() {
            vec![dir.to_owned()]
        } else {
            names.iter().map(|name| join(dir, name)).collect()
        };

        let count = self
            .refused
            .iter()
            .filter(|ordinal| {
                let raw = self.entries[**ordinal as usize].path.as_str();

                wanted.iter().any(|folder| below(raw, folder))
            })
            .count();

        u32::try_from(count).unwrap_or(u32::MAX)
    }

    fn node(&self, path: &str) -> Option<u32> {
        if path.is_empty() {
            Some(0)
        } else {
            self.paths.get(path).copied()
        }
    }

    fn child(&self, node: u32) -> Child<'_> {
        let node = &self.nodes[node as usize];

        Child {
            name: &node.name,
            dir: node.dir,
            entry: node.entry.map(|ordinal| &self.entries[ordinal as usize]),
        }
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

/// Builds an [`ArchiveIndex`] from the records of a walk, within the limits [`MAX_INDEX_ENTRIES`] and
/// [`MAX_INDEX_NAME_BYTES`].
#[derive(Debug)]
pub struct Builder {
    index: ArchiveIndex,
    name_bytes: usize,
}

impl Builder {
    pub fn new(format: ArchiveFormat) -> Self {
        let root = Node {
            name: String::new(),
            dir: true,
            entry: None,
            children: Vec::new(),
        };

        Self {
            index: ArchiveIndex {
                format,
                entries: Vec::new(),
                nodes: vec![root],
                paths: HashMap::new(),
                refused: Vec::new(),
                truncated: false,
            },
            name_bytes: 0,
        }
    }

    /// Adds the next entry of the archive. [`Flow::Stop`] says the limits are reached: the entry was not added and the
    /// index is marked truncated.
    pub fn push(&mut self, record: Record) -> Flow {
        let index = &mut self.index;

        if index.entries.len() >= MAX_INDEX_ENTRIES
            || self.name_bytes.saturating_add(record.name.len()) > MAX_INDEX_NAME_BYTES
        {
            index.truncated = true;

            return Flow::Stop;
        }

        self.name_bytes += record.name.len();
        let ordinal = index.entries.len() as u32;
        let (path, refused) = match names::normalize(&record.name) {
            Ok(path) => (path.unwrap_or_default(), None),
            Err(why) => (record.name, Some(why)),
        };

        index.entries.push(IndexEntry {
            path,
            kind: record.kind,
            size: record.size,
            modified: record.modified,
            mode: record.mode,
            link: record.link,
            encrypted: record.encrypted,
            ordinal,
            place: record.place,
            refused,
        });

        if refused.is_some() {
            index.refused.push(ordinal);
        } else {
            index.place(ordinal);
        }

        Flow::Continue
    }

    /// The index. `cut` says the walk ended early (see [`Walked::cut`](super::entries::Walked::cut)).
    pub fn finish(mut self, cut: bool) -> ArchiveIndex {
        self.index.truncated |= cut;

        self.index
    }
}

impl ArchiveIndex {
    /// Puts the entry `ordinal` (its path is normalized) in the tree.
    fn place(&mut self, ordinal: u32) {
        let entry = &self.entries[ordinal as usize];
        let (path, kind) = (entry.path.clone(), entry.kind);

        if path.is_empty() {
            return;
        }

        let last_end = path.len();
        let mut parent = 0_u32;
        let mut start = 0;

        while start < last_end {
            let end = path[start..].find('/').map_or(last_end, |at| start + at);
            let key = &path[..end];
            let is_last = end == last_end;

            let id = match self.paths.get(key) {
                Some(id) => *id,
                None => self.add_node(parent, key, &path[start..end]),
            };

            if is_last {
                self.settle(id, ordinal, kind);
            } else {
                // Something is below it, so it is a folder; a file that held its path is shadowed.
                let node = &mut self.nodes[id as usize];

                if !node.dir {
                    node.dir = true;
                    node.entry = None;
                }
            }

            parent = id;
            start = end + 1;
        }
    }

    fn add_node(&mut self, parent: u32, key: &str, name: &str) -> u32 {
        let id = self.nodes.len() as u32;

        self.nodes.push(Node {
            name: name.to_owned(),
            dir: false,
            entry: None,
            children: Vec::new(),
        });
        self.nodes[parent as usize].children.push(id);
        self.paths.insert(key.to_owned(), id);

        id
    }

    /// The entry names the node `id`: a later file replaces an earlier one, and a folder is never replaced by a file.
    fn settle(&mut self, id: u32, ordinal: u32, kind: EntryType) {
        let node = &mut self.nodes[id as usize];

        if kind == EntryType::Dir {
            node.dir = true;
            node.entry = Some(ordinal);
        } else if !node.dir {
            node.entry = Some(ordinal);
        }
    }
}
