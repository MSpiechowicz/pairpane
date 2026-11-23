//! Moving items to the Trash (the Recycle Bin on Windows) and putting them back.
//!
//! Linux and the other freedesktop systems do it here (see [`freedesktop`]): the items are renamed into the Trash
//! folder of their own drive and put back from the one they went to. Windows and macOS leave the work to the system
//! through the `trash` crate ([`shell`]): Windows can also list the Recycle Bin and restore from it, so the last move
//! can be undone there. macOS cannot, and [`CAN_UNTRASH`] says so.

use std::path::PathBuf;

use super::items::ItemFailure;

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod freedesktop;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
use freedesktop as system;

#[cfg(not(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))))]
mod shell;
#[cfg(not(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))))]
use shell as system;

#[cfg(all(
    test,
    not(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))
))]
pub(super) use shell::{pick, windows_comparable};

#[cfg(all(test, windows))]
pub(super) use shell::comparable;

#[cfg(all(test, unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
pub(super) use freedesktop::{home_trash, purge_below};

/// Whether [`untrash`] can work here: freedesktop systems and Windows list and restore.
pub const CAN_UNTRASH: bool = cfg!(any(
    windows,
    all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    )
));

/// The text for a move to the Trash that the system refused for a reason that is not told apart.
#[cfg(not(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))))]
const NOT_MOVED: &str = if cfg!(windows) {
    "the system could not move it to the Recycle Bin"
} else {
    "the system could not move it to the Trash"
};

/// A move to the Trash that reached the Trash with at least one item, as much as [`untrash`] needs to find them again.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Trashed {
    /// The folder the items were in.
    pub folder: PathBuf,
    /// The names of the items that are in the Trash now.
    pub names: Vec<String>,
    /// The earliest second (Unix time) the Trash can have stamped on them. Only where the items are found by their
    /// time (Windows); 0 on the systems that record where each went.
    pub from: i64,
    /// The latest second the Trash can have stamped on them (see `from`).
    pub to: i64,
    /// Where each of the items went, in the order of `names`.
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
    placed: Vec<freedesktop::Placed>,
}

/// Why [`untrash`] did not put the items back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UntrashFailure {
    /// This system cannot list or restore (macOS).
    Unsupported,
    /// None of the items is in the Trash any more: it was emptied, or they were put back by hand.
    NothingFound,
    /// An item with the name `name` is back at the original place already (a new one of the same name was made).
    /// The items that were put back before it stay where they are.
    InTheWay { name: String },
    /// The folder the items came from is gone or is another folder now (it was replaced). Nothing is put back, and
    /// no folder is made.
    FolderChanged,
    /// The Trash could not be read or restored from, for another reason (logged).
    Other,
}

/// Moves the items `names` of `folder` to the Trash one by one, stopping at the first that fails (see
/// [`super::trash`]).
pub(super) fn trash(folder: &std::path::Path, names: &[String]) -> (Trashed, Result<(), ItemFailure>) {
    system::trash(folder, names)
}

/// Puts the items of `record` back where they were (see [`super::untrash`]).
pub(super) fn untrash(record: &Trashed) -> Result<usize, UntrashFailure> {
    system::untrash(record)
}

/// Moves the item `name` of the folder `folder`, held open as `held` (Unix), to the Trash, and fails it when the Trash
/// refuses (nothing is deleted for good instead).
///
/// On Linux and the other freedesktop systems the item is renamed from the held folder, which the path must still name. On
/// macOS the system takes a path: the path must lead to the held folder right before, and a swap after that still wins.
/// On Windows the path is all there is.
pub(super) fn trash_held(folder: &std::path::Path, held: &super::held::Held, name: &str) -> std::io::Result<()> {
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
    {
        freedesktop::trash_held(folder, &held.dir, name)
    }

    #[cfg(all(unix, any(target_os = "macos", target_os = "ios", target_os = "android")))]
    {
        use std::os::unix::fs::MetadataExt;

        let named = std::fs::metadata(std::fs::canonicalize(folder)?)?;
        let here = held.dir.metadata()?;
        if (named.dev(), named.ino()) != (here.dev(), here.ino()) {
            return Err(super::report::refusal(
                std::io::ErrorKind::Other,
                "the folder changed while it was being walked",
            ));
        }

        shell::trash_named(folder, name)
    }

    #[cfg(windows)]
    {
        let _ = held;

        shell::trash_named(folder, name)
    }
}
