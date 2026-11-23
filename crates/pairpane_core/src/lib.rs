//! Core of Pairpane, a dual-pane file manager. No Tauri dependency; the desktop shell is a thin
//! layer of IPC commands over this crate.
//!
//! # Layering
//!
//! 1. [`model`]: serde + ts-rs DTOs that form the IPC contract (generated TypeScript bindings).
//! 2. Rules that need no Tauri, tested here: [`listing`] (folder listings and change watching), [`volumes`]
//!    (the drive list), [`ops`] (copy and move jobs), [`state`] (what is saved between runs), [`settings`] (the file the user edits), [`update`] (in-app updater),
//!    [`view`] (the file viewer), [`edit`] (the built-in text editor), [`archive`] (reading archives), [`find`] (searching a folder tree), [`gitstatus`] (the git status column, read in process by gitoxide),
//!    [`compare`] (comparing two folder trees),
//!    [`rename`] (the rule and the preview of a multi-rename), [`shell`] (the command line: a line run in the user's shell).

pub mod archive;
pub mod compare;
pub mod edit;
pub mod find;
pub mod gitstatus;
pub mod listing;
pub mod model;
pub mod ops;
pub mod process;
pub mod rename;
pub mod settings;
pub mod shell;
pub mod state;
pub mod update;
pub mod view;
pub mod volumes;
