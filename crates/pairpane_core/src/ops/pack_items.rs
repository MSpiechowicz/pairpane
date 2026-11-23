//! The items of a pack job, found in the folder the job holds open: each is a name in that folder, or a path of names
//! below it (`a/b.txt`, what a list of search results gives), and is stored in the archive under that path. The folders
//! on the way are not entries of their own unless they are items too (the readers show a folder no entry names as one).
//!
//! An item is found step by step from the held folder, each step a look at the entry without following a link and (for
//! a folder) an open from its parent compared with what was seen, so a link on the way, or a folder swapped for another,
//! fails that item and not the job. The folders of the way are kept in a trail for the file to be opened again by
//! (see `tree`); only the folder an item is in is held open while it is walked, so a long list holds no more
//! descriptors than a short one.

use std::io;
use std::sync::Arc;

use super::held::{Held, Node};
use super::pack::Plan;
use super::plan::Kind;
use super::report::refusal;
use super::transfer::{Run, Stop};
use super::tree::{Spot, Trail};

/// An item found below the root: the folder it is in, held, the entry, and the folders on the way.
pub(super) struct Found {
    pub(super) parent: Arc<Held>,
    pub(super) node: Node,
    pub(super) trail: Trail,
    /// How many folders the item is below the root.
    pub(super) depth: usize,
}

impl Run<'_> {
    /// Finds the item `rel` (a name, or a `/` separated path of names) below `root`.
    ///
    /// # Errors
    ///
    /// Fails when a step is gone, is not a folder (a link is not one), or is not the folder that was seen.
    pub(super) fn find(&self, root: &Arc<Held>, rel: &str) -> io::Result<Found> {
        let mut parent = Arc::clone(root);
        let mut trail = Trail::default();
        let (mut start, mut depth) = (0, 0);

        for (end, _) in rel.match_indices('/') {
            let folder = parent.look(&rel[start..end])?;
            if folder.kind != Kind::Dir {
                return Err(refusal(
                    io::ErrorKind::InvalidInput,
                    "a folder on the way to the item is a link or not a folder",
                ));
            }

            // A test swaps the folder here, between the look at it and the open.
            if let Some(before) = &self.inner.hooks.before_open {
                before(&rel[..end]);
            }

            parent = Arc::new(parent.subfolder(&folder)?);
            trail = trail.below(&folder);
            start = end + 1;
            depth += 1;
        }

        let node = parent.look(&rel[start..])?;

        Ok(Found {
            parent,
            node,
            trail,
            depth,
        })
    }

    /// The folder of the job's items, held, and the items in it in name order: the order of the entries of an archive does
    /// not depend on the order the items were selected in.
    pub(super) fn sorted_items(&mut self) -> Result<(Arc<Held>, Vec<Node>), Stop> {
        let root = self.hold_source()?;
        let mut items = self.look_up_items(&root);
        items.sort_by(|a, b| a.name.cmp(&b.name));

        Ok((root, items))
    }

    /// The folder of the job's items, held, and their names (see the module's text) in order, the same for any order they
    /// were selected in.
    pub(super) fn sorted_names(&mut self) -> Result<(Arc<Held>, Vec<String>), Stop> {
        let root = self.hold_source()?;
        let mut names = self.job.request.names.clone();
        names.sort();

        Ok((root, names))
    }

    /// Checks the folders and the names of the job and holds its source folder open.
    fn hold_source(&mut self) -> Result<Arc<Held>, Stop> {
        let request = &self.job.request;
        let from = self.open_folders(&[&request.from, &request.to])?;

        Ok(Arc::new(Held::root(&from)?))
    }

    /// Finds the items `names` below the folder `root` one by one, and walks them to plan the entries. An item that cannot be
    /// found is reported and left out.
    pub(super) fn plan_entries(&mut self, root: &Arc<Held>, names: &[String]) -> Result<Plan, Stop> {
        let mut plan = Plan::new(self, root, String::new(), self.pack_limits());

        self.visited.clear();
        for name in names {
            self.checkpoint()?;

            match self.find(root, name) {
                Ok(found) => {
                    let at = Spot {
                        parent: &found.parent,
                        node: &found.node,
                        rel: name,
                        trail: &found.trail,
                    };

                    self.walk_spot(&at, found.depth, &mut plan)?;
                }
                Err(error) => self.fail(name, &error),
            }
        }

        Ok(plan)
    }
}
