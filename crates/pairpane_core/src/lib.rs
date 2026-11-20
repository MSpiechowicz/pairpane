//! Core of Pairpane, a dual-pane file manager. No Tauri dependency; the desktop shell is a thin
//! layer of IPC commands over this crate.
//!
//! # Layering
//!
//! 1. [`model`]: serde + ts-rs DTOs that form the IPC contract (generated TypeScript bindings).

pub mod model;
