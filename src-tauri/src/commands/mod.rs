//! IPC commands. Each is a thin adapter over `pairpane_core`; names are the contract in
//! `ui/src/lib/ipc/commands.ts` and must also be listed in `build.rs` and `capabilities/main.json`.

pub mod app;
