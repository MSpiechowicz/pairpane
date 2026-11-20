//! Core of Pairpane, a dual-pane file manager. No Tauri dependency; the desktop shell is a thin
//! layer of IPC commands over this crate.
//!
//! # Layering
//!
//! 1. [`model`]: serde + ts-rs DTOs that form the IPC contract (generated TypeScript bindings).
//! 2. Rules that need no Tauri, tested here: [`listing`] (folder listings and change watching), [`volumes`]
//!    (the drive list), [`state`] (what is saved between runs), [`update`] (in-app updater).

pub mod listing;
pub mod model;
pub mod state;
pub mod update;
pub mod volumes;
