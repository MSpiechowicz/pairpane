//! The built-in editor: opening a text file whole and saving it back, in the encoding, the byte order mark and the
//! line breaks it had (or the user chose). The text lives in the page while it is edited; nothing is held open here,
//! so a save finds the file again by its folder and name.
//!
//! # API
//!
//! - [`open`]: reads the regular file `name` of a folder as text ([`EditOpened`]): its text with every line break as
//!   `\n`, the encoding found (or asked for), the byte order mark, the most common line break and whether the file mixes
//!   them, whether the user may not write it, and a `revision` that tells later whether it changed. A file bigger than
//!   [`MAX_EDIT`], a file with a NUL byte that is not UTF-16 and bytes that are not valid in the encoding are told as
//!   such and are never read with replacement characters.
//! - [`check_regular`]: whether a path is a file an external editor may be given (a link to one is).
//! - [`save`]: writes the text back ([`EditSaved`]), unless the file changed since the `revision` it was read at (the
//!   answer says so, and a save with `force` overrides) or is gone.
//!
//! # Rules
//!
//! Links are refused on open and on save, and nothing is opened through one. A save is atomic where it can be: the
//! text goes to a new file next to the target (`.pairpane-edit-<run>-<n>.tmp`, created exclusively), is flushed to
//! the disk and renamed over the target (on Windows put over it by `ReplaceFileW`), and the folder is flushed too. A
//! file with other names (a hard link), a file with extended attributes (which hold POSIX ACLs), a file of another
//! user, a file whose group a new file next to it would not keep and a folder where no new file can be made are
//! written in place instead (through a no-follow handle that is checked to be the file that was seen). The
//! permissions of the replaced file are kept, but for the setuid and setgid bits of a file that is replaced. Error texts are fixed lowercase English and never hold a path or any
//! of the text ([`crate::view::refusal`]).
//!
//! # Known limits, accepted
//!
//! The check of the revision and the write are not one step: a change made by another program in between is lost, as
//! with any editor. A file with extended attributes (which hold POSIX ACLs) is written in place, and on Windows a save
//! replaces the file with `ReplaceFileW`, which keeps ACLs, alternate streams and attributes; on Windows a hard link is
//! not detected, so such a file is replaced and its other names keep the old text.

mod disk;
mod open;
mod save;
mod text;
#[cfg(windows)]
mod win_replace;

#[cfg(test)]
mod open_tests;
#[cfg(test)]
mod save_tests;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod text_tests;

pub use open::{check_regular, open, open_bytes};
pub use save::save;

/// Whether the paths `a` and `b` are one file (a hard link, or a link to it, counts): the shell refuses a save into a
/// file of its own that the user keeps, whatever it is called. One that cannot be looked at counts as the same.
pub fn is_same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    disk::same_file(a, b)
}

/// The biggest file the editor opens, in bytes; a bigger one is told as too large.
pub const MAX_EDIT: usize = 8 * 1024 * 1024;

/// The biggest text a save takes, in bytes of UTF-8: a file of [`MAX_EDIT`] bytes in a one-byte encoding (windows-1252
/// has `€` at one byte) is up to three times that as UTF-8, and a user may add a megabyte to it.
pub const MAX_SAVE_TEXT: usize = 3 * MAX_EDIT + 1024 * 1024;
