//! Core of Pairpane, a dual-pane file manager. No Tauri dependency; the desktop shell is a thin
//! layer of IPC commands over this crate.
//!
//! # Layering
//!
//! 1. [`model`]: serde + ts-rs DTOs that form the IPC contract (generated TypeScript bindings).
//! 2. Rules that need no Tauri, tested here: [`update`] (in-app updater).

pub mod model;
pub mod update;
