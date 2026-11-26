//! Remote sites for Pairpane: browsing a server over SFTP, behind the file-system layer's [`RemoteFs`] trait. No Tauri
//! dependency; the app (`src-tauri`) wires a [`Pool`] to its commands and to the question it asks about a new host key.
//!
//! - [`Pool`]: connect (a user may have to give a password or a key's passphrase), the [`RemoteFs`] of a connected site,
//!   disconnect, the hosts of `~/.ssh/config`, shut down.
//! - Host keys are checked against `~/.ssh/known_hosts` and the system's `ssh_known_hosts` (see [`HostKeyPrompt`]); a
//!   key that changed is never trusted.
//! - Sign-in tries the ssh-agent, the key files, then the password. A secret is used for what it was asked for: a
//!   passphrase only opens a key file here and is never sent to the server, a password is only sent.
//! - A connection that drops is opened again by the next call, without a question; every call ends within its time
//!   limit ([`Timeouts`]) even when the server is silent.
//! - Errors are `io::Error`s with fixed messages: no host, user, path or secret is in one, so they can be shown and
//!   logged as they are. Secrets are held as [`Password`](pairpane_core::archive::Password) (zeroized, hidden from `Debug`)
//!   and are never logged.
//!
//! - Where a host leads is asked of OpenSSH itself (`ssh -G`, a fixed program with an explicit argument list, never a
//!   shell; see [`PoolConfig::ssh_program`]): the `HostName`, `User`, `Port`, `IdentityFile`s, the `known_hosts` files, the
//!   `HostKeyAlias`, `StrictHostKeyChecking`, `HashKnownHosts`, `IdentityAgent`, `BatchMode`, `NumberOfPasswordPrompts` and
//!   the options that switch a way of signing in on or off are its answer (`ssh -G` runs with the app's environment). A host it would reach through a proxy (`ProxyJump`, `ProxyCommand`), or whose keys are judged by a
//!   `RevokedHostKeys` file or a `KnownHostsCommand`, is refused with `Unsupported`, never connected to directly. Without
//!   OpenSSH the host, user and port are used as typed, but only for a user who has no `~/.ssh/config`: with one, every
//!   connect is refused until OpenSSH is installed.
//!
//! Not supported yet: a proxy, a two-step sign-in, the Windows ssh-agents, and key certificates.

mod agent;
mod allowed;
mod auth;
mod config;
mod error;
mod known_hosts;
mod link;
mod names;
mod pattern;
mod plan;
mod pool;
mod questions;
mod regular;
mod resolve;
mod runtime;
mod sftp;
mod sftp_files;
mod site;
mod suggestions;
mod terms;
mod verify;

#[cfg(any(test, feature = "test-server"))]
pub mod test_server;

#[cfg(test)]
mod known_hosts_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod verify_tests;

pub use agent::AgentSource;
pub use auth::Secret;
pub use config::{PoolConfig, Timeouts};
pub use pool::{MAX_SITES, Pool};
pub use verify::{HostKeyPrompt, UnknownHost};
