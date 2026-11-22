//! Writing one file of a synchronization by handle (Unix): the folders of the target are entered one name at a time, each
//! opened from the handle of the one above with no link followed, and the temporary file is made in the last of them and
//! renamed there (`renameat`), so a folder swapped for a link after it was looked at leads nowhere: no path is passed to a
//! system call that could follow one, from the first folder of the target down.
//!
//! The target's own name is looked at through the handle (`fstatat`, no link followed) to give the item its terms (see
//! `sync_terms`), and looked at again right before a rename that replaces what is there; a name that is free is only
//! taken by a no-replace rename.
//!
//! What goes by path, as in every copy: the source file (opened with no link followed and compared with what was seen),
//! the folders this job made (named for removal if the job is cancelled while they are empty), and the free-space look.
//! macOS gives up the instant clone of the data here (`fclonefileat` is by path in the copy); the loop copies it.

use std::fs::File;
use std::io;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use rustix::fs::{AtFlags, FileType, Mode, OFlags, Stat, mkdirat, openat, renameat, statat, unlinkat};

use crate::listing::system_millis;
use crate::model::{EntryKind, SyncFacts};

use super::SyncPlan;
use super::fast;
use super::held::Held;
use super::identity::FolderId;
use super::open;
use super::plan::{Child, Kind};
use super::report::refusal;
use super::sync::split_item;
use super::sync_terms::{Seen, Terms};
use super::transfer::{Run, Stop};

/// The temporary file of a write, made in a folder held open: removed through that handle unless it was renamed.
struct HeldTemp {
    dir: File,
    name: String,
    committed: bool,
}

impl Drop for HeldTemp {
    fn drop(&mut self) {
        if !self.committed {
            let _ = unlinkat(&self.dir, self.name.as_str(), AtFlags::empty());
        }
    }
}

impl Held {
    /// Makes the subfolder `name` and opens it, with whether this call made it; one that is there by now is used as it is
    /// (nothing in it is ever replaced) and is not this job's to remove.
    fn make_subfolder(&self, name: &str) -> io::Result<(Self, bool)> {
        let made = match mkdirat(&self.dir, name, Mode::from_bits_truncate(0o777)) {
            Ok(()) => true,
            Err(errno) if errno == rustix::io::Errno::EXIST => false,
            Err(errno) => return Err(io::Error::from(errno)),
        };

        let node = self.look(name)?;
        if node.kind != Kind::Dir {
            return Err(refusal(
                io::ErrorKind::AlreadyExists,
                "a file or link has the name of a folder",
            ));
        }

        Ok((self.subfolder(&node)?, made))
    }

    /// Renames the file `from` of this folder to `to`, replacing what is there. Both are plain names.
    fn rename_replace(&self, from: &str, to: &str) -> io::Result<()> {
        renameat(&self.dir, from, &self.dir, to).map_err(io::Error::from)
    }

    /// What is at the name `name` of this folder now, looked at without following a link.
    fn seen(&self, name: &str) -> io::Result<Option<Seen>> {
        match statat(&self.dir, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => Ok(Some(seen_of(&stat))),
            Err(errno) if errno == rustix::io::Errno::NOENT => Ok(None),
            Err(errno) => Err(io::Error::from(errno)),
        }
    }
}

/// The facts and the identity of what `stat` describes, as a plan states them.
#[allow(
    clippy::unnecessary_cast,
    reason = "the device, inode, time and size are as wide as these types on some systems and narrower on others"
)]
fn seen_of(stat: &Stat) -> Seen {
    let kind = match FileType::from_raw_mode(stat.st_mode) {
        FileType::Directory => EntryKind::Dir,
        FileType::RegularFile => EntryKind::File,
        _ => EntryKind::Other,
    };
    let (seconds, nanos) = (stat.st_mtime as i64, stat.st_mtime_nsec as u64);
    let behind = Duration::from_secs(seconds.unsigned_abs());
    let time = if seconds >= 0 {
        UNIX_EPOCH.checked_add(behind)
    } else {
        UNIX_EPOCH.checked_sub(behind)
    }
    .and_then(|time| time.checked_add(Duration::from_nanos(nanos)));

    let facts = SyncFacts {
        kind,
        size: if kind == EntryKind::File {
            u64::try_from(stat.st_size).unwrap_or(0)
        } else {
            0
        },
        modified: time.and_then(system_millis),
    };

    (facts, Some(FolderId::of_ids((stat.st_dev as u64, stat.st_ino as u64))))
}

impl Run<'_> {
    /// Copies the file `source` (the item `rel`) below the folder `target_root`, by handle, when the target is what `plan`
    /// says; counts the item as skipped when it is not, or when it stops being so before the rename.
    pub(super) fn sync_file(
        &mut self,
        source: &Child,
        target_root: &Path,
        rel: &str,
        plan: &SyncPlan,
    ) -> Result<(), Stop> {
        let (parents, name) = split_item(rel);
        let Some((folder, path)) = self.enter_target(target_root, parents, plan.target.is_none())? else {
            return self.changed();
        };

        let Some(terms) = Terms::judge(plan.target.as_ref(), folder.seen(name)?) else {
            return self.changed();
        };
        self.set_terms(Some(terms));

        self.begin_file(rel, source.meta.len());
        let done = self.write_held(source, (&folder, &path), name, &terms);
        self.end_file();

        done
    }

    /// Enters the folders `parents` below `root`, each from the one above. Those that are missing are made when `make`
    /// says so, else the item is not where the plan saw it (`None`).
    fn enter_target(
        &mut self,
        root: &Path,
        parents: &str,
        make: bool,
    ) -> Result<Option<(Held, std::path::PathBuf)>, Stop> {
        let mut held = Held::root(root)?;
        let mut path = root.to_owned();
        let mut walked = String::new();
        let mut making = false;

        for part in parents.split('/').filter(|part| !part.is_empty()) {
            path.push(part);
            if !walked.is_empty() {
                walked.push('/');
            }
            walked.push_str(part);

            if !making {
                match held.look(part) {
                    Ok(node) if node.kind == Kind::Dir => {
                        // A test swaps the folder here, between the look at it and the open.
                        if let Some(before) = &self.inner.hooks.before_open {
                            before(&walked);
                        }
                        held = held.subfolder(&node)?;

                        continue;
                    }
                    Ok(_) => {
                        return Err(
                            refusal(io::ErrorKind::AlreadyExists, "a file or link has the name of a folder").into(),
                        );
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound && make => making = true,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                    Err(error) => return Err(error.into()),
                }
            }

            let made;
            (held, made) = held.make_subfolder(part)?;
            // Only a folder this job made is removed again if it stops; one made by someone else meanwhile is theirs.
            if made {
                self.created.push(path.clone());
            }
        }

        Ok(Some((held, path)))
    }

    /// Writes `source` to a temporary file in `folder` and puts it at `name` on `terms` (see `sync_terms`).
    fn write_held(
        &mut self,
        source: &Child,
        (folder, path): (&Held, &Path),
        name: &str,
        terms: &Terms,
    ) -> Result<(), Stop> {
        let mut file = open::source(&source.path, &source.meta)?;
        let meta = file.metadata()?;

        // The handle the guard removes the temporary file through is taken first, so no failure leaves one behind.
        let dir = folder.dir.try_clone()?;
        let (temp, mut copy) = self.make_temp(path, |temp| create_in(folder, temp))?;
        let name_of_temp = temp.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let mut guard = HeldTemp {
            dir,
            name: name_of_temp.clone(),
            committed: false,
        };

        if !self.inner.hooks.buffered_only && fast::clone_into(&copy, &file) {
            self.add_bytes(meta.len());
        } else {
            self.fill(&mut file, &mut copy, meta.len())?;
        }
        self.checkpoint()?;
        self.finish_copy(&mut file, &mut copy, &meta)?;

        let landed = match terms {
            Terms::Absent => match folder.rename_noreplace(&name_of_temp, name) {
                Ok(()) => true,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => false,
                Err(error) => return Err(error.into()),
            },
            // The last look, right before the rename that replaces.
            Terms::Same { .. } if terms.allows(folder.seen(name)?.as_ref()) => {
                folder.rename_replace(&name_of_temp, name)?;

                true
            }
            Terms::Same { .. } => false,
        };

        if landed {
            guard.committed = true;
        } else {
            self.skipped += 1;
        }

        Ok(())
    }
}

/// Makes the new file `temp` (its name only) in `folder`, for reading and writing, readable by the owner only until its
/// permissions are set. Fails when anything is there, a link too.
fn create_in(folder: &Held, temp: &Path) -> io::Result<File> {
    let name = temp.file_name().unwrap_or_default();
    let flags = OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;

    openat(&folder.dir, name, flags, Mode::from_bits_truncate(0o600))
        .map(File::from)
        .map_err(io::Error::from)
}
