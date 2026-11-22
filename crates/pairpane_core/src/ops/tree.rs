//! Walking the tree below the items of a job, without ever following a link: the walk the checksum file and the pack job
//! share. It goes through [`Held`] handles (a handle per folder on Unix, paths that are looked at again on Windows; see
//! `held`), in a stable order (names sorted in every folder, a folder before what is in it), walks each folder once
//! (a bind mount can show one twice) and not deeper than [`MAX_DEPTH`].
//!
//! What is done with what the walk meets is the [`Visitor`]'s: it is given each file and each folder it enters. A link
//! or a special file is never given: it is counted as skipped. An item that fails (a folder that cannot be read, a visitor
//! that refuses it) is reported under its path and the walk goes on, unless the visitor says the job cannot go on.
//!
//! A [`Reopen`] opens a file again later, from the root of the walk down the folders it was found in, each step relative
//! to a handle and no-follow, so a file can be read long after the walk left its folder without a handle held open for
//! every folder in between.

use std::fs::File;
use std::io;
use std::sync::Arc;

use super::held::{Held, Node};
use super::plan::Kind;
use super::report::refusal;
use super::transfer::{MAX_DEPTH, Run, Stop};

/// The folders from the root of the walk down to a node, as the walk saw them.
#[derive(Clone, Default)]
pub(super) struct Trail(Option<Arc<Step>>);

struct Step {
    up: Trail,
    folder: Node,
}

impl Trail {
    /// This trail with the folder `folder` below it.
    pub(super) fn below(&self, folder: &Node) -> Self {
        Self(Some(Arc::new(Step {
            up: self.clone(),
            folder: folder.clone(),
        })))
    }

    /// The folders, the topmost first.
    fn folders(&self) -> Vec<&Node> {
        let mut folders = Vec::new();
        let mut at = &self.0;

        while let Some(step) = at {
            folders.push(&step.folder);
            at = &step.up.0;
        }
        folders.reverse();

        folders
    }
}

/// What the visitor is given: a file or a folder of the tree.
pub(super) struct Spot<'a> {
    /// The folder the node is in.
    pub(super) parent: &'a Held,
    pub(super) node: &'a Node,
    /// The path of the node relative to the folder of the items, `/` separated (a name that is not text is shown as its
    /// lossy text).
    pub(super) rel: &'a str,
    /// The folders between the root of the walk and `parent`, `parent` included.
    pub(super) trail: &'a Trail,
}

/// What a walk does with what it meets. Only the file is required.
pub(super) trait Visitor {
    /// A regular file.
    fn file(&mut self, run: &mut Run<'_>, at: &Spot<'_>) -> Result<(), Stop>;

    /// A folder the walk is about to go into, which was opened and listed. Failing here leaves the folder out, with all
    /// that is below it.
    fn folder(&mut self, _run: &mut Run<'_>, _at: &Spot<'_>) -> Result<(), Stop> {
        Ok(())
    }

    /// A link, which is skipped (already counted by the walk).
    fn link(&mut self, _run: &mut Run<'_>, _rel: &str) {}

    /// Whether the visitor has met what ends the job: a failure of an item is then not reported and skipped but ends the
    /// walk.
    fn fatal(&self) -> bool {
        false
    }
}

/// A file of a walk, to open again later (see the module's text).
pub(super) struct Reopen {
    root: Arc<Held>,
    trail: Trail,
    node: Node,
}

impl Reopen {
    /// The file `at`, found below `root`.
    pub(super) fn new(root: &Arc<Held>, at: &Spot<'_>) -> Self {
        Self {
            root: Arc::clone(root),
            trail: at.trail.clone(),
            node: at.node.clone(),
        }
    }

    /// Opens the file for reading: it must still be the one that was seen, and no link is followed on the way.
    pub(super) fn open(&self) -> io::Result<File> {
        let mut folder: Option<Held> = None;

        for step in self.trail.folders() {
            let next = folder.as_ref().unwrap_or(&self.root).subfolder(step)?;
            folder = Some(next);
        }

        folder.as_ref().unwrap_or(&self.root).open_file(&self.node)
    }
}

impl Run<'_> {
    /// Walks the `items` of the folder `parent` (in the order given), giving what is below them to `visitor`.
    pub(super) fn walk_items(&mut self, parent: &Held, items: &[Node], visitor: &mut impl Visitor) -> Result<(), Stop> {
        self.visited.clear();
        let top = Trail::default();

        for item in items {
            let rel = item.rel();
            let at = Spot {
                parent,
                node: item,
                rel: &rel,
                trail: &top,
            };

            self.walk_spot(&at, 0, visitor)?;
        }

        Ok(())
    }

    /// Walks the item `at`, which is `depth` folders below the root of the walk, giving it and what is below it to `visitor`.
    /// A failure is reported against its path and the walk goes on (unless the visitor calls it fatal).
    pub(super) fn walk_spot(&mut self, at: &Spot<'_>, depth: usize, visitor: &mut impl Visitor) -> Result<(), Stop> {
        self.checkpoint()?;

        let done = self.walk_node(at.parent, at.node, at.rel, at.trail, depth, visitor);

        self.settle_walked(at.rel, done, visitor)
    }

    fn walk_node(
        &mut self,
        parent: &Held,
        node: &Node,
        rel: &str,
        trail: &Trail,
        depth: usize,
        visitor: &mut impl Visitor,
    ) -> Result<(), Stop> {
        let at = Spot {
            parent,
            node,
            rel,
            trail,
        };

        match node.kind {
            Kind::Dir => self.walk_folder(&at, depth, visitor),
            Kind::File => visitor.file(self, &at),
            Kind::Link => {
                self.skipped += 1;
                visitor.link(self, rel);

                Ok(())
            }
            Kind::Other => {
                self.skipped += 1;

                Ok(())
            }
        }
    }

    fn walk_folder(&mut self, at: &Spot<'_>, depth: usize, visitor: &mut impl Visitor) -> Result<(), Stop> {
        if depth >= MAX_DEPTH {
            return Err(refusal(io::ErrorKind::InvalidInput, "the folders are nested too deeply").into());
        }
        if !self.first_visit_node(at.node) {
            self.skipped += 1;

            return Ok(());
        }

        let (held, mut children) = self.open_and_list(at.parent, at.node, at.rel)?;
        visitor.folder(self, at)?;

        // The order of a folder is the system's own; the walk goes by name, the same way every time.
        children.sort_by(|a, b| a.name.cmp(&b.name));
        let below = at.trail.below(at.node);

        for child in &children {
            self.checkpoint()?;
            let child_rel = format!("{}/{}", at.rel, child.rel());

            let done = self.walk_node(&held, child, &child_rel, &below, depth + 1, visitor);
            self.settle_walked(&child_rel, done, visitor)?;
        }

        Ok(())
    }

    /// [`Run::settle`] for a walk: a failure the visitor calls fatal ends the walk too.
    fn settle_walked(&mut self, rel: &str, done: Result<(), Stop>, visitor: &impl Visitor) -> Result<(), Stop> {
        match done {
            Err(failed @ Stop::Failed(_)) if visitor.fatal() => Err(failed),
            done => self.settle(rel, done),
        }
    }
}
