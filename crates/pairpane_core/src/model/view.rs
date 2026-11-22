use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What an opened file is, decided from its first bytes (and its name where the bytes cannot tell). `mime` is the
/// type the webview is told to expect, taken from a fixed table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ViewKind {
    Text,
    Markdown,
    Image {
        mime: String,
    },
    Pdf,
    Audio {
        mime: String,
    },
    Video {
        mime: String,
    },
    Archive {
        format: ArchiveFormat,
    },
    /// Anything else: shown as hex only.
    Binary,
}

/// The archive formats whose entries are listed (by the viewer and by a pane that opens an archive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ArchiveFormat {
    /// A zip (`.zip`, `.jar`).
    Zip,
    Tar,
    /// A tar inside gzip (`.tar.gz`, `.tgz`).
    TarGz,
    /// A tar inside XZ (`.tar.xz`, `.txz`).
    TarXz,
    /// A tar inside Zstandard (`.tar.zst`, `.tzst`).
    TarZst,
    /// A tar inside bzip2 (`.tar.bz2`, `.tbz2`, `.tbz`).
    TarBz2,
    /// A 7z archive.
    SevenZ,
    /// A RAR archive; read by the `unrar` program of the system, so only a pane opens one.
    Rar,
    /// One gzip-compressed file (`.gz` that is not a tar): an archive of a single entry.
    Gz,
}

/// A viewer session that was opened on a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ViewOpened {
    /// The handle every later viewer command takes.
    pub session: u32,
    pub name: String,
    /// Size in bytes when it was opened.
    #[ts(type = "number")]
    pub size: u64,
    /// Last modification, in milliseconds since the Unix epoch; `null` when the filesystem does not report it.
    #[ts(type = "number | null")]
    pub modified: Option<i64>,
    pub kind: ViewKind,
    /// The label of the detected text encoding (`UTF-8`, `windows-1250`, `UTF-16LE`); empty for what is not text.
    pub encoding: String,
}

/// A window of a text file: `lines` lines (at most 1000), read from or up to `offset`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TextQuery {
    /// An encoding label (`UTF-8`, `windows-1250`, `UTF-16LE`).
    pub encoding: String,
    /// A byte offset in the file. Forward windows start here, backward windows end here.
    #[ts(type = "number")]
    pub offset: u64,
    /// Return the lines that end at `offset` instead of the ones that start there.
    pub backward: bool,
    /// How many lines are wanted; clamped to 1..=1000.
    pub lines: u32,
    /// Move `offset` to the start of the line it falls in (for an offset from a scroll bar). Without it the offset is
    /// taken as it is (it was the start or the end of an earlier window).
    pub snap: bool,
}

/// What a text query found. The lines are in file order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TextWindow {
    /// The byte offset of the first line, or where the window would start if it holds none.
    #[ts(type = "number")]
    pub start: u64,
    /// The byte offset right after the last line (a following window starts here).
    #[ts(type = "number")]
    pub end: u64,
    pub lines: Vec<TextLine>,
    /// There is no text before `start` (the file's start, after a byte order mark).
    pub at_start: bool,
    /// There is no text from `end` on.
    pub at_end: bool,
}

/// One line, or one piece of a very long line, without its line ending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TextLine {
    /// The byte offset of the first byte of `text` in the file.
    #[ts(type = "number")]
    pub offset: u64,
    pub text: String,
    /// The line goes on in the next entry (it was cut because it is long).
    pub continues: bool,
}

/// What a search looks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum SearchNeedle {
    /// Text, encoded in the encoding of the query before it is looked for.
    Text { text: String, case_sensitive: bool },
    /// These bytes, as they are.
    Hex { bytes: Vec<u8> },
}

/// A search from a byte offset, in one direction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchQuery {
    pub needle: SearchNeedle,
    /// An encoding label; used for a text needle only.
    pub encoding: String,
    /// Forward: the first match that starts at or after this offset. Backward: the last match that starts before it.
    #[ts(type = "number")]
    pub from: u64,
    pub backward: bool,
}

/// Where a search found its needle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchHit {
    /// The byte offset of the first byte of the match.
    #[ts(type = "number")]
    pub offset: u64,
    /// The length of the match in bytes.
    #[ts(type = "number")]
    pub len: u64,
}

/// The entries of an archive, in archive order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArchiveListing {
    pub format: ArchiveFormat,
    pub entries: Vec<ArchiveEntry>,
    /// More entries exist than were listed (or the archive could not be read to its end).
    pub truncated: bool,
}

/// One entry of an archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArchiveEntry {
    /// The path inside the archive, as written there (not valid Unicode shows `�`).
    pub name: String,
    /// The size of the content in bytes.
    #[ts(type = "number")]
    pub size: u64,
    pub dir: bool,
    /// Last modification, in milliseconds since the Unix epoch (a zip stores local time, read here as UTC); `null`
    /// when the archive has none.
    #[ts(type = "number | null")]
    pub modified: Option<i64>,
}
