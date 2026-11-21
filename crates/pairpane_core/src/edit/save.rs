//! Saving the text of the editor.

use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::model::{EditSaveOptions, EditSaved};
use crate::ops::{Partial, create_new, run_token};
use crate::view::{Refusals, open_regular, refusal};

use super::{disk, text};

/// The words a save refuses a name that is not a regular file in.
const SAVER: Refusals = Refusals {
    link: "the editor saves regular files, not links",
    folder: "the editor saves files, not folders",
    other: "the editor saves regular files only",
};

/// Names tried for the temporary file before giving up.
const MAX_TEMP_TRIES: u32 = 100;

/// Writes `text` over the file `name` of `folder` as `options` say (see [`text::encode`] for what it refuses before
/// the disk is touched). The file must be the one at `options.revision`, unless `options.force`; a file that changed is
/// answered with [`EditSaved::Changed`] and left alone, and so is one that is gone (with `force` it is made again).
/// A link, a folder and anything that is not a regular file are refused, and so is a file the user cannot write.
pub fn save(folder: &Path, name: &str, text: &str, options: &EditSaveOptions) -> io::Result<EditSaved> {
    let bytes = text::encode(text, options)?;
    let target = folder.join(name);

    let seen = match open_regular(folder, name, &SAVER) {
        Ok(opened) => Some(opened),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };

    match seen {
        Some((file, meta)) => {
            if !disk::writable(&target, &meta) {
                return Err(refusal(io::ErrorKind::PermissionDenied, "the file is read-only"));
            }

            if !options.force && disk::revision(&meta) != options.revision {
                return Ok(EditSaved::Changed { gone: false });
            }

            replace(folder, &target, &bytes, &meta, &file)?;
        }
        None => {
            if !options.force {
                return Ok(EditSaved::Changed { gone: true });
            }

            match create(&target, &bytes) {
                // Someone made it in the meantime: not the file that was asked to be made again.
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    return Ok(EditSaved::Changed { gone: false });
                }
                written => written?,
            }
        }
    }

    // What the next open will see, so the next save compares with it.
    let now = fs::symlink_metadata(&target)?;

    Ok(EditSaved::Saved {
        size: now.len(),
        revision: disk::revision(&now),
    })
}

/// Makes the file that is gone, which no one else is to have made.
fn create(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(target)?;

    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    if written.is_err() {
        drop(file);
        let _ = fs::remove_file(target);
    }

    written
}

/// Puts `bytes` in place of the file `target`, which is `seen` (`checked` is the handle it was seen through): as a new
/// file put over it when that changes nothing about the file but its content, otherwise in the file itself. So it is
/// too when the system could not put the new file over it and says the file was left as it was (on Windows, a file that
/// another program holds open); the temporary file is gone by then.
fn replace(folder: &Path, target: &Path, bytes: &[u8], seen: &Metadata, checked: &File) -> io::Result<()> {
    match temp_in(folder) {
        Ok((partial, file)) => {
            if disk::may_rename(seen, checked, &file)? {
                match swap(folder, target, bytes, seen, partial, file) {
                    Err(error) if disk::replace_left_original(&error) => {
                        tracing::debug!(%error, "the file was not replaced; writing into it");

                        return in_place(target, bytes, seen);
                    }
                    done => return done,
                }
            }

            // The temporary file is removed again: it was only made to see what a new file would be.
            drop(file);
            drop(partial);
        }
        // No new file may be made here: the file may still be written.
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {}
        Err(error) => return Err(error),
    }

    in_place(target, bytes, seen)
}

/// Makes the temporary file next to the target, exclusively: a name that is taken is skipped, and never opened.
fn temp_in(folder: &Path) -> io::Result<(Partial, File)> {
    static NEXT: AtomicU32 = AtomicU32::new(0);

    for _ in 0..MAX_TEMP_TRIES {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = folder.join(format!(".pairpane-edit-{}-{number}.tmp", run_token()));

        match create_new(&path) {
            Ok(file) => return Ok((Partial::new(path), file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }

    Err(refusal(io::ErrorKind::Other, "the file cannot be saved"))
}

/// Writes `bytes` to the temporary file, with the permissions of the target (less setuid and setgid), flushes it and
/// puts it over the target (a rename; on Windows `ReplaceFileW`, which keeps the ACLs and streams), then flushes the
/// folder. Until the rename the target is as it was; the temporary file is removed again on
/// any failure.
fn swap(
    folder: &Path,
    target: &Path,
    bytes: &[u8],
    seen: &Metadata,
    mut partial: Partial,
    mut file: File,
) -> io::Result<()> {
    file.set_permissions(disk::kept_permissions(seen))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);

    disk::replace_with(&partial.path, target)?;
    partial.committed = true;

    sync_folder(folder);

    Ok(())
}

/// Flushes the folder so that the rename is on the disk; the text is saved already, so a folder that cannot be
/// flushed (not every system allows it) is not a failure.
#[cfg(unix)]
fn sync_folder(folder: &Path) {
    let _ = File::open(folder).and_then(|dir| dir.sync_all());
}

#[cfg(not(unix))]
fn sync_folder(_folder: &Path) {}

/// Writes `bytes` into the file `target` itself, which is `seen`: opened without following a link and checked to be
/// that very file, then written from the start and cut to length (so a failure half way leaves the old end, never an
/// empty file) and flushed.
fn in_place(target: &Path, bytes: &[u8], seen: &Metadata) -> io::Result<()> {
    let mut file = disk::open_for_write(target)?;
    let now = file.metadata()?;

    if !now.is_file() || !disk::is_seen(&now, seen) {
        return Err(refusal(
            io::ErrorKind::Other,
            "the file changed while it was being saved",
        ));
    }

    file.write_all(bytes)?;
    file.set_len(bytes.len() as u64)?;
    file.sync_all()
}
