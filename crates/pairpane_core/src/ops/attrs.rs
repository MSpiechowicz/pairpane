//! Changing permissions and times of the items, and of what is inside the selected folders when asked to.
//!
//! Nothing is changed through a link: a link (and a pipe, a socket or a device) in the selection, or inside a
//! selected folder, is skipped and counted, never opened. A walk does not cross into another filesystem on Unix.
//! A folder is changed before its contents when it can still be listed afterwards, otherwise after them, so
//! permissions that take the access away (`000`) and give it back both work on a whole tree. Times are set last, as
//! changing the contents does not touch them but a later change of the folder's own entry might.
//! The walk goes by handle on Unix (see `held`), and the platform part of a change is in `attrs_apply`.

use std::io;

use crate::model::{AttributeChange, JobPhase};

use super::attrs_apply::stays_open;
use super::held::{Held, Node};
use super::plan::Kind;
use super::report::refusal;
use super::task::Scene;
use super::transfer::{MAX_DEPTH, Run, Stop};

impl Run<'_> {
    /// Changes the items of `scene` as `change` says.
    pub(super) fn change_attributes(&mut self, scene: &Scene, change: &AttributeChange) -> Result<(), Stop> {
        let parent = Held::root(&scene.from)?;
        let items = self.look_up_items(&parent);

        self.phase(JobPhase::Scanning);
        self.count_files(&parent, &items, change.recursive)?;

        self.phase(JobPhase::Changing);
        for item in &items {
            self.checkpoint()?;
            let rel = item.rel();

            let done = self.change_node(&parent, item, &rel, change, item.device, 0);
            self.settle(&rel, done)?;
        }

        Ok(())
    }

    /// Counts the files and links the job will go through (folders are not counted), for its progress.
    fn count_files(&mut self, parent: &Held, items: &[Node], recursive: bool) -> Result<(), Stop> {
        if recursive {
            return self.scan_nodes(parent, items);
        }

        for item in items {
            match item.kind {
                Kind::File => self.found(item.size),
                Kind::Link => self.found(0),
                Kind::Dir | Kind::Other => {}
            }
        }

        Ok(())
    }

    /// Changes `node`, which is in the folder held by `parent`, and what is below it when `change` is recursive. `rel`
    /// is its path relative to the folder of the items, `device` the filesystem the walk stays on.
    fn change_node(
        &mut self,
        parent: &Held,
        node: &Node,
        rel: &str,
        change: &AttributeChange,
        device: Option<u64>,
        depth: usize,
    ) -> Result<(), Stop> {
        match node.kind {
            Kind::Dir => self.change_folder(parent, node, rel, change, device, depth),
            Kind::File => {
                self.begin_file(rel, node.size);
                let done = change_item(parent, node, change);
                self.end_file();

                Ok(done?)
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

    fn change_folder(
        &mut self,
        parent: &Held,
        node: &Node,
        rel: &str,
        change: &AttributeChange,
        device: Option<u64>,
        depth: usize,
    ) -> Result<(), Stop> {
        if depth >= MAX_DEPTH {
            return Err(refusal(io::ErrorKind::InvalidInput, "the folders are nested too deeply").into());
        }

        // Another filesystem is left alone (a mount inside the folder), and so is a folder reached twice.
        let elsewhere = depth > 0 && node.device != device;
        if elsewhere || !self.first_visit_node(node) {
            self.skipped += 1;

            return Ok(());
        }

        let first = change.recursive && stays_open(change, node.mode);

        if first && let Err(error) = parent.set_permissions(node, change) {
            self.fail(rel, &error);
        }
        if change.recursive {
            self.change_contents(parent, node, rel, change, device, depth)?;
        }
        if !first && let Err(error) = parent.set_permissions(node, change) {
            self.fail(rel, &error);
        }

        parent.set_times(node, change)?;

        Ok(())
    }

    /// Changes what is in the folder `node`. A folder that cannot be listed or held open is reported, and what is in
    /// it is left as it is.
    fn change_contents(
        &mut self,
        parent: &Held,
        node: &Node,
        rel: &str,
        change: &AttributeChange,
        device: Option<u64>,
        depth: usize,
    ) -> Result<(), Stop> {
        let (held, children) = match self.open_and_list(parent, node, rel) {
            Ok(both) => both,
            Err(Stop::Failed(error)) => {
                self.fail(rel, &error);

                return Ok(());
            }
            Err(Stop::Cancelled) => return Err(Stop::Cancelled),
        };

        for child in &children {
            self.checkpoint()?;
            let child_rel = format!("{rel}/{}", child.rel());

            let done = self.change_node(&held, child, &child_rel, change, device, depth + 1);
            self.settle(&child_rel, done)?;
        }

        Ok(())
    }
}

/// Changes one file: its permissions, then its times.
fn change_item(parent: &Held, node: &Node, change: &AttributeChange) -> io::Result<()> {
    parent.set_permissions(node, change)?;

    parent.set_times(node, change)
}
