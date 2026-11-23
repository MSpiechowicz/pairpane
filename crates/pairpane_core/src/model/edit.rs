use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The line break a file is saved with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LineEnding {
    /// `\n`.
    Lf,
    /// `\r\n`.
    Crlf,
    /// `\r`.
    Cr,
}

/// A text file opened in the editor: its whole text and how it is stored, to be saved the same way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EditDocument {
    pub name: String,
    /// The text, with every line break as `\n`.
    pub text: String,
    /// The label of the encoding it was read with (`UTF-8`, `windows-1250`, `UTF-16LE`).
    pub encoding: String,
    /// Whether the file starts with a byte order mark (UTF-8 and UTF-16 only).
    pub bom: bool,
    /// The most common line break of the file (`lf` when it has none).
    pub line_ending: LineEnding,
    /// Whether the file mixes line breaks; saving writes every one as `line_ending`.
    pub mixed_endings: bool,
    /// Whether the user cannot write the file: the editor shows it but cannot save it.
    pub read_only: bool,
    /// Size in bytes when it was read.
    #[ts(type = "number")]
    pub size: u64,
    /// What the file was when it was read, to tell on save whether it changed since. Opaque to the page.
    pub revision: String,
}

/// The answer to opening a file in the editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum EditOpened {
    /// The file, as text.
    Text { document: EditDocument },
    /// The file is bigger than the editor takes (`MAX_EDIT`).
    TooLarge {
        #[ts(type = "number")]
        size: u64,
    },
    /// The file is not text (it holds a NUL byte and is not UTF-16).
    Binary,
    /// The bytes are not valid in `encoding` (the detected one, or the one asked for); another may be tried.
    Undecodable { encoding: String },
}

/// How to save the editor's text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EditSaveOptions {
    /// The label of the encoding to write.
    pub encoding: String,
    /// Whether to start the file with a byte order mark (UTF-8 and UTF-16 only).
    pub bom: bool,
    /// The line break to write for every line break of the text.
    pub line_ending: LineEnding,
    /// The `revision` the text was read at (from [`EditDocument`] or the last [`EditSaved::Saved`]).
    pub revision: String,
    /// Save even when the file changed (or went) since `revision`.
    pub force: bool,
}

/// The answer to saving the editor's text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum EditSaved {
    /// Written; the file is now `size` bytes at `revision`.
    Saved {
        #[ts(type = "number")]
        size: u64,
        revision: String,
    },
    /// Not written: the file changed since `revision` (`gone`: it was deleted or replaced). Save again with `force`.
    Changed { gone: bool },
}

/// The external editor the user's settings file names, as the settings page shows it (it never changes it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EditorSetting {
    pub program: String,
    pub args: Vec<String>,
}

/// The answer to opening a file in the external editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExternalEdit {
    /// The external editor was started on the file.
    Launched,
    /// No external editor is set: the built-in one is to be used.
    NotConfigured,
}
