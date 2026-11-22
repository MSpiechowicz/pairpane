//! IPC commands. Each is a thin adapter over `pairpane_core`; names are the contract in
//! `ui/src/lib/ipc/commands.ts` and must also be listed in `build.rs` and `capabilities/main.json`.

pub mod app;
pub mod archives;
pub mod edit;
pub mod find;
pub mod items;
mod limit;
pub mod ops;
pub mod panes;
pub mod sizes;
pub mod state;
pub mod updates;
pub mod view;
pub mod volumes;

#[cfg(test)]
mod testing;
