//! Copying the items: folders, files and links, each into the target folder.
//!
//! A file is written to a temporary file next to its target (created exclusively, so nothing at that name is
//! ever opened), given its times and permissions, checked when asked to, and only then renamed into place. The
//! target is therefore either absent or complete, whatever happens in between.

use std::fs::{self, File, Metadata};
use std::io;
use std::path::{Path, PathBuf};

use super::conflict::Place;
use super::fast::{self, Start};
use super::meta;
use super::open;
use super::partial::Partial;
use super::plan::{Child, Kind};
use super::rename;
use super::report::refusal;
use super::transfer::{MAX_DEPTH, Run, Stop};

/// What became of one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Done {
    Copied,
    /// Left alone: skipped by an answer, or not something that can be copied.
    Left,
}

impl Run<'_> {
    /// Copies `node` (and what is below it) into the folder `to_dir`. `rel` is its path relative to the source
    /// folder, `depth` how far below an item the job started with it is (0 for such an item).
    pub(super) fn copy_node(&mut self, to_dir: &Path, node: &Child, rel: &str, depth: usize) -> Result<Done, Stop> {
        match node.kind {
            Kind::Dir => self.copy_folder(to_dir, node, rel, depth),
            Kind::File | Kind::Link => {
                self.begin_file(rel, if node.kind == Kind::File { node.meta.len() } else { 0 });
                let done = self.copy_leaf(to_dir, node, rel, depth);
                self.end_file();

                done
            }
            Kind::Other => {
                self.skipped += 1;

                Ok(Done::Left)
            }
        }
    }

    /// Makes the folder `path`. `false` when it existed already (the copy then merges into it); an error when
    /// something else is there, a link included (nothing is written through a link).
    pub(super) fn make_folder(&mut self, path: &Path, private: bool) -> io::Result<bool> {
        match open::create_folder(path, private) {
            Ok(()) => {
                self.created.push(path.to_owned());

                Ok(true)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if fs::symlink_metadata(path)?.is_dir() {
                    return Ok(false);
                }

                Err(refusal(
                    io::ErrorKind::AlreadyExists,
                    "a file or link has the name of this folder",
                ))
            }
            Err(error) => Err(error),
        }
    }

    fn copy_folder(&mut self, to_dir: &Path, node: &Child, rel: &str, depth: usize) -> Result<Done, Stop> {
        if depth >= MAX_DEPTH {
            return Err(refusal(io::ErrorKind::InvalidInput, "the folders are nested too deeply").into());
        }
        if !self.first_visit(node) {
            self.skipped += 1;

            return Ok(Done::Left);
        }

        let target = to_dir.join(&node.name);
        let created = self.make_folder(&target, true)?;
        let children = self.children_of(node, rel)?;

        for child in &children {
            self.checkpoint()?;
            let child_rel = format!("{rel}/{}", child.rel());

            let done = self.copy_node(&target, child, &child_rel, depth + 1);
            self.settle(&child_rel, done)?;
        }

        // Last, as adding the contents changes the folder's time (a folder that already existed is left as it is).
        if created && let Err(error) = meta::apply_folder(&target, &node.meta) {
            self.fail(rel, &error);
        }

        // Only an empty folder goes: what was skipped or failed stays where it was.
        if self.moving() {
            let _ = fs::remove_dir(&node.path);
        }

        // Complete: a cancel later on leaves it where it is (it may be the only copy left of an empty folder).
        if created {
            self.created.finish(&target);
        }

        Ok(Done::Copied)
    }

    /// Copies a file or a link: asks what to do when the target exists, then writes and commits it.
    fn copy_leaf(&mut self, to_dir: &Path, node: &Child, rel: &str, depth: usize) -> Result<Done, Stop> {
        // A file is opened once, and its metadata read at once: reading it changes its access time.
        let (mut source, meta) = if node.kind == Kind::File {
            let file = open::source(&node.path, &node.meta)?;
            let meta = file.metadata()?;

            (Some(file), meta)
        } else {
            (None, node.meta.clone())
        };

        let target = to_dir.join(&node.name);
        let mut place = self.place(rel, &meta, &node.path, &target)?;

        if place == Place::Skip {
            self.skipped += 1;

            return Ok(Done::Left);
        }

        if self.moving() && depth == 0 && self.move_in_place(&node.path, &place)? {
            return Ok(Done::Copied);
        }

        let (temp, mut guard) = match &mut source {
            Some(file) => self.copy_to_temp(file, &meta, to_dir)?,
            None => self.link_to_temp(node, to_dir)?,
        };

        if !self.commit(&temp, &mut place, rel, &meta, &node.path)? {
            self.skipped += 1;

            return Ok(Done::Left);
        }
        guard.committed = true;

        if self.moving() {
            self.remove_source(node, place.path())?;
        }

        Ok(Done::Copied)
    }

    /// For a move: renames the top-level item `from` to where `place` says, which copies nothing. `false` when
    /// the target is on another disk, and the item has to be copied after all.
    fn move_in_place(&mut self, from: &Path, place: &Place) -> io::Result<bool> {
        let renamed = match place {
            Place::New(to) => self.rename_noreplace(from, to),
            Place::Replace(to) => self.rename_replace(from, to),
            Place::Skip => return Ok(false),
        };

        match renamed {
            Ok(()) => Ok(true),
            Err(error) if rename::crosses_devices(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Writes the copy of `source` to a new temporary file in `dir`, with its times and permissions, checked when
    /// the job asks for it. Returns the guard that removes the file unless it is committed.
    fn copy_to_temp(&mut self, source: &mut File, meta: &Metadata, dir: &Path) -> Result<(PathBuf, Partial), Stop> {
        let clone = !self.inner.hooks.buffered_only;
        let (temp, (mut copy, start)) = self.make_temp(dir, |temp| fast::create(source, temp, clone))?;
        let guard = Partial::new(temp.clone());

        match start {
            Start::Cloned => self.add_bytes(meta.len()),
            Start::Empty => self.fill(source, &mut copy, meta.len())?,
        }
        self.checkpoint()?;

        if self.inner.hooks.fail_after_data {
            return Err(io::Error::other("the disk failed (a test asked for it)").into());
        }
        if self.inner.hooks.taken_after_data {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists).into());
        }

        if self.job.request.verify {
            self.verify(source, &mut copy)?;
        }

        meta::apply_file(&copy, meta)?;

        Ok((temp, guard))
    }

    /// Makes the link `node` is copied as in a new temporary file in `dir`, as it is, never followed.
    fn link_to_temp(&mut self, node: &Child, dir: &Path) -> Result<(PathBuf, Partial), Stop> {
        let destination = fs::read_link(&node.path)?;
        let (temp, ()) = self.make_temp(dir, |temp| make_link(node, &destination, temp))?;
        let guard = Partial::new(temp.clone());

        Ok((temp, guard))
    }

    /// Renames the finished temporary file `temp` into place. When the name was taken since the question (or
    /// the lack of one), that is a conflict again. `false` when the answer is to skip.
    fn commit(
        &mut self,
        temp: &Path,
        place: &mut Place,
        rel: &str,
        meta: &Metadata,
        source: &Path,
    ) -> Result<bool, Stop> {
        loop {
            let renamed = match place {
                Place::New(target) => rename::noreplace(temp, target),
                Place::Replace(target) => rename::replace(temp, target),
                Place::Skip => return Ok(false),
            };

            match renamed {
                Ok(()) => return Ok(true),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let target = place.path().to_owned();
                    *place = self.place(rel, meta, source, &target)?;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

/// Makes the link `temp` to `destination`, the target of the link `node`. Fails when anything is at `temp`.
fn make_link(node: &Child, destination: &Path, temp: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let _ = node;

        std::os::unix::fs::symlink(destination, temp)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{FileTypeExt, symlink_dir, symlink_file};

        if node.meta.file_type().is_symlink_dir() {
            symlink_dir(destination, temp)
        } else {
            symlink_file(destination, temp)
        }
    }
}

impl Run<'_> {
    /// Deletes the source of a move once its copy is committed at `target`: a file or a link, never a folder's
    /// contents.
    ///
    /// Only when both identities were read and differ: if `target` is the source itself, reached by another name,
    /// the commit replaced it with the copy, which is then the only one left. When that cannot be told, the source
    /// is kept and the item fails with the reason.
    fn remove_source(&self, node: &Child, target: &Path) -> io::Result<()> {
        let (Some(source_id), Some(target_id)) = (self.id_of_path(&node.path), self.id_of_path(target)) else {
            return Err(refusal(
                io::ErrorKind::Other,
                "Kept the original: could not confirm the copy is a different file",
            ));
        };

        if source_id == target_id {
            return Ok(());
        }

        match fs::remove_file(&node.path) {
            // A link to a folder is removed like a folder on Windows.
            Err(_) if cfg!(windows) && node.kind == Kind::Link => fs::remove_dir(&node.path),
            result => result,
        }
    }
}
