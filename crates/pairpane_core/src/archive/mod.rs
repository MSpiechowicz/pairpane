//! Archives: reading the entries of a zip, tar (plain or compressed), gzip, 7z or RAR file without extracting anything,
//! and, later, handing out the data of chosen entries. Nothing here touches the disk; extraction and packing are `ops`'s
//! job, fed by an [`EntrySource`] and feeding the [`write`] writers.
//!
//! # API
//!
//! - [`detect`], [`by_magic`], [`by_extension`] ([`kind`]): which [`ArchiveFormat`] a file is.
//! - [`Input`]: an opened archive file (a [`ReadAt`] handle and, where there is one, its path).
//! - [`records`]: walks the entries as [`Record`]s, calling a visitor; the viewer lists through it.
//! - [`build_index`]: the [`ArchiveIndex`] of an archive: entries in archive order plus the tree of folders, with
//!   [`ArchiveIndex::list`], [`ArchiveIndex::find`] and [`ArchiveIndex::select`] (what an extraction of chosen names
//!   writes). Names are checked on the way in ([`names::normalize`]); an entry with an unsafe name is kept but flagged,
//!   and never reachable through the tree.
//! - [`IndexCache`] and [`Fingerprint`] ([`cache`]): the last four indexes, keyed by the file's state.
//! - [`open_source`]: an [`EntrySource`], which calls a visitor with the data of the wanted entries (ascending
//!   ordinals) as bounded readers ([`Declared`] keeps each to the size it declares).
//! - [`write`]: the writers a pack job uses (zip, tar.zst, 7z) over a `Write`; they read nothing of the disk and are fed
//!   entries with readers by `ops`.
//! - [`rewrite`]: an archive written again with entries added and some left out (what adding to an archive does).
//! - [`Password`]: held in memory only, zeroized, `Debug` hides it. A password that an archive needs and lacks (or that is
//!   wrong) surfaces as an error made by [`password_error`] and recognised by [`password_fault`].
//! - [`check_password`]: whether a password opens an archive, judged cheaply (a zip by the verifier or check byte of its
//!   first encrypted entry; [`zip::check_password`]); what `archive_password` answers with.
//!
//! # Formats and who reads them
//!
//! Zip's central directory ([`zip::directory`]) and entry data ([`zip::ZipSource`]: stored, deflate, bzip2, zstd and LZMA,
//! ZipCrypto and WinZip AES, entries that overlap refused), tar and `.tar.gz` ([`tar`]) and gzip ([`gz`]) are read here. A format
//! that is not read yet fails with `Unsupported` in [`records`] / [`open_source`]; those two functions (in `open.rs`) and
//! [`tar::stream::open`] are where a new reader is wired in. The `.tar.xz`/`.zst`/`.bz2` decoders, 7z and
//! RAR are added there.
//!
//! # Rules
//!
//! Every error text is fixed and holds no path or password. Every size is capped by a constant below, whatever the
//! archive declares: nothing is allocated from a declared count, and a walk or a read stops when its `cancel` flag is
//! raised (`Interrupted`).

pub mod cache;
pub mod entries;
pub mod error;
pub mod file;
pub mod gz;
pub mod index;
pub mod kind;
pub mod names;
mod open;
pub mod password;
pub mod rar;
pub mod rewrite;
pub mod sevenz;
pub mod tar;
pub mod write;
pub mod zip;

#[cfg(test)]
mod cache_tests;
#[cfg(test)]
mod compressed_tests;
#[cfg(test)]
mod gz_tests;
#[cfg(test)]
mod index_tests;
#[cfg(test)]
mod kind_tests;
#[cfg(test)]
mod names_tests;
#[cfg(test)]
mod open_tests;
#[cfg(test)]
mod sevenz_tests;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod walk_tests;

pub use cache::{Fingerprint, IndexCache};
pub use entries::{Cancellable, Declared, EntrySource, EntryType, Flow, Place, Record, Visit, Walked};
pub use error::{PasswordFault, cancelled, fault_text, password_error, password_fault, unreadable, unsupported};
pub use file::{FileReader, ReadAt};
pub use index::{ArchiveIndex, Builder, Child, IndexEntry, Selected, Selection};
pub use kind::{HEAD_BYTES, by_extension, by_magic, detect};
pub use names::{Unsafe, normalize};
pub use open::{Input, build_index, open_source, records};
pub use password::{Password, check_password};

/// The most entries an index holds; an archive with more is indexed up to here and marked truncated (a zip that declares
/// more is refused).
pub const MAX_INDEX_ENTRIES: usize = 200_000;

/// The most bytes the names of an index's entries take together.
pub const MAX_INDEX_NAME_BYTES: usize = 64 * 1024 * 1024;

/// The most bytes of a compressed tar that are decoded to index it, or of a gzip file to extract; past it the index is
/// marked truncated. Reading is cancellable all the way.
pub const MAX_INDEX_SCAN: u64 = 64 * 1024 * 1024 * 1024;

/// The most bytes of one entry that is opened in the viewer (the entry is extracted into a temporary file first).
pub const MAX_VIEW_ENTRY: u64 = 512 * 1024 * 1024;

/// How much free space an extraction leaves on the disk it writes to.
pub const MIN_FREE_RESERVE: u64 = 256 * 1024 * 1024;

/// The longest name (whole path) of an entry, in bytes; a longer one is refused.
pub const MAX_NAME_BYTES: usize = 4096;
