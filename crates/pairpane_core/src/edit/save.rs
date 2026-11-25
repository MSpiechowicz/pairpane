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

                        return in_place_kept(folder, target, bytes, (seen, checked));
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

    in_place_kept(folder, target, bytes, (seen, checked))
}

/// Writes `bytes` into the file itself (see [`write_into`]), with the old content kept beside the file until the new one is on the disk (#76): a copy is made first
/// (`.pairpane-edit-…-` and the file's name, so one left by a crash or a power cut says whose it is) and flushed; a write
/// that fails puts the old content back and removes the copy, and the copy is removed once the new content is flushed.
/// Where no file can be made beside it (a folder the user cannot write to) the file is written without a copy, as before;
/// a copy that cannot be made for another reason (a full disk) refuses the save before the file is touched.
fn in_place_kept(folder: &Path, target: &Path, bytes: &[u8], (seen, checked): (&Metadata, &File)) -> io::Result<()> {
    let old = read_whole(checked, seen)?;
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    let kept = match backup_in(folder, &name, &old) {
        Ok(kept) => Some(kept),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => None,
        Err(error) => {
            return Err(io::Error::new(
                error.kind(),
                format!("the old content could not be kept while saving, so nothing was written: {error}"),
            ));
        }
    };

    // A file that is not the one seen any more is left alone, and so is the copy of the one that was.
    let mut file = match open_checked(target, seen) {
        Ok(file) => file,
        Err(error) => {
            if let Some(kept) = kept {
                let _ = fs::remove_file(&kept);
            }
            return Err(error);
        }
    };

    match write_into(&mut file, bytes) {
        Ok(()) => {
            if let Some(kept) = kept {
                let _ = fs::remove_file(&kept);
            }
            Ok(())
        }
        Err(error) => {
            // The old content goes back into the same file, through the same handle; the copy goes only once that worked.
            #[cfg(test)]
            FAIL_IN_PLACE.with(|fail| fail.set(false));
            let restored = write_into(&mut file, &old);
            match (restored, kept) {
                (Ok(()), Some(kept)) => {
                    let _ = fs::remove_file(&kept);
                    Err(error)
                }
                (Ok(()), None) => Err(error),
                (Err(_), Some(kept)) => Err(io::Error::new(
                    error.kind(),
                    format!(
                        "{error}; the old content is kept in {}",
                        kept.file_name()
                            .map_or_else(String::new, |kept| kept.to_string_lossy().into_owned())
                    ),
                )),
                (Err(_), None) => Err(error),
            }
        }
    }
}

/// The whole content of the file open as `file`, which is `seen`.
fn read_whole(file: &File, seen: &Metadata) -> io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};

    let mut reader = file;
    reader.seek(SeekFrom::Start(0))?;
    let mut old = Vec::with_capacity(usize::try_from(seen.len()).unwrap_or(0));
    reader.read_to_end(&mut old)?;

    Ok(old)
}

/// Writes `old` to a new copy beside the file `name` in `folder`, flushed, and gives its path.
fn backup_in(folder: &Path, name: &str, old: &[u8]) -> io::Result<std::path::PathBuf> {
    static NEXT: AtomicU32 = AtomicU32::new(0);

    for _ in 0..MAX_TEMP_TRIES {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let prefix = format!(".pairpane-edit-{}-{number}", run_token());
        let named = format!("{prefix}-{name}");
        let path = folder.join(if crate::ops::name_fits(&named) {
            named
        } else {
            format!("{prefix}.bak")
        });

        match create_new(&path) {
            Ok(mut file) => {
                let mut partial = Partial::new(path.clone());
                file.write_all(old)?;
                file.sync_all()?;
                partial.committed = true;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }

    Err(refusal(
        io::ErrorKind::Other,
        "no free name for a copy of the file was found",
    ))
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

/// Opens the file `target` to write into it: without following a link, and checked to be `seen`, that very file.
fn open_checked(target: &Path, seen: &Metadata) -> io::Result<File> {
    let file = disk::open_for_write(target)?;
    let now = file.metadata()?;

    if !now.is_file() || !disk::is_seen(&now, seen) {
        return Err(refusal(
            io::ErrorKind::Other,
            "the file changed while it was being saved",
        ));
    }

    Ok(file)
}

/// Writes `bytes` into the open file from the start and cuts it to length (so a failure half way leaves the old end,
/// never an empty file), then flushes it.
fn write_into(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    use std::io::{Seek, SeekFrom};

    file.seek(SeekFrom::Start(0))?;

    #[cfg(test)]
    if FAIL_IN_PLACE.with(std::cell::Cell::get) {
        // Half of the new content is written, as a full disk would leave it.
        file.write_all(&bytes[..bytes.len() / 2])?;
        return Err(io::Error::other("the disk is full (a test asked for it)"));
    }

    file.write_all(bytes)?;
    file.set_len(bytes.len() as u64)?;
    file.sync_all()
}

#[cfg(test)]
thread_local! {
    /// Makes the write into the file itself fail half way, on this thread.
    pub(super) static FAIL_IN_PLACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
