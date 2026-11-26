//! The commands of SFTP sites: the suggestions of `~/.ssh/config`, connecting, and disconnecting. Listing a folder of a
//! connected site is `panes/remote.rs`.
//!
//! The work is `pairpane_remote`'s [`Pool`], which owns a runtime of its own; every call of it blocks, so each runs on a
//! blocking thread, never on the async runtime, and at most [`CONNECTS`] connects are outstanding. The pool is made when
//! the app starts; when it cannot be (no home folder, no threads) the app still runs and every command here answers
//! with a fixed error.
//!
//! A host key that `known_hosts` does not know is asked about in a native dialog (the page cannot answer it). A key that
//! differs from the file's is refused by the pool, with no way past it from here. A secret the page sends is wrapped
//! into a zeroized [`Password`] before anything else, and is never logged; the errors are the pool's fixed texts, which
//! hold no host, user, path or secret.

use std::io;
use std::sync::Arc;

use pairpane_core::archive::Password;
use pairpane_core::model::{ConnectOutcome, ConnectTarget, IpcErrorCode, SecretKind, SiteId, SshHost};
use pairpane_remote::{HostKeyPrompt, Pool, PoolConfig, Secret, UnknownHost};
use tauri::async_runtime::spawn_blocking;
use tauri::{AppHandle, State};

use super::edit::ask_user;
use super::items::blocking;
use super::limit::ThreadCap;
use crate::error::{CommandError, CommandResult};

#[cfg(test)]
pub(super) mod fixtures;
#[cfg(test)]
mod tests;

/// Connects that may be outstanding at once (a user is asked for a password, a server does not answer): beyond this a
/// connect is refused as busy, so the page cannot fill the blocking pool with them.
const CONNECTS: usize = 4;

/// The longest host, key kind and fingerprint the question shows. A host is what the user typed; the others are the
/// server's, and a fingerprint is far shorter than this.
const MAX_HOST_SHOWN: usize = 255;
const MAX_KEY_SHOWN: usize = 128;

/// The servers the app is connected to.
pub struct Remotes {
    /// `None` when the pool could not be made.
    pool: Option<Arc<Pool>>,
    connects: ThreadCap,
}

impl Remotes {
    /// The pool of the real user (`~/.ssh`, the ssh-agent); without one, the commands fail but the app runs.
    pub fn start() -> Self {
        match PoolConfig::from_env().and_then(Pool::new) {
            Ok(pool) => Self::with(pool),
            Err(error) => {
                tracing::warn!(kind = ?error.kind(), "remote sites are not available");

                Self {
                    pool: None,
                    connects: ThreadCap::default(),
                }
            }
        }
    }

    pub fn with(pool: Pool) -> Self {
        Self {
            pool: Some(Arc::new(pool)),
            connects: ThreadCap::default(),
        }
    }

    /// Closes every connection (about a second at the most). Blocks: not for an async task.
    pub fn shut_down(&self) {
        if let Some(pool) = &self.pool {
            pool.shut_down();
        }
    }

    pub(super) fn pool(&self) -> CommandResult<Arc<Pool>> {
        self.pool.clone().ok_or_else(|| {
            CommandError::new(
                IpcErrorCode::Unsupported,
                "remote sites are not available in this session",
            )
        })
    }

    /// Connects to `target`, asking `prompt` about a new host key. `secret` is the answer to an earlier `secretNeeded`
    /// and `kind` the kind it is (what that asked for, or the `other` it offered; the page sends both, or neither): a
    /// passphrase only opens a key file here, and a password is only sent to the server, so the two must not be mixed up.
    pub(super) async fn connect(
        &self,
        target: ConnectTarget,
        secret: Option<String>,
        kind: Option<SecretKind>,
        prompt: Arc<dyn HostKeyPrompt>,
    ) -> CommandResult<ConnectOutcome> {
        // First of all: from here the secret is zeroized when dropped, whatever refuses the command.
        let secret = secret.map(Password::new);
        let secret = match (secret, kind) {
            (Some(value), Some(kind)) => Some(Secret { kind, value }),
            (None, None) => None,
            _ => {
                return Err(CommandError::new(
                    IpcErrorCode::InvalidInput,
                    "a secret and what it answers are sent together",
                ));
            }
        };
        let pool = self.pool()?;
        let permit = self
            .connects
            .acquire(CONNECTS)
            .ok_or_else(|| CommandError::new(IpcErrorCode::Busy, "a connection is being made; try again shortly"))?;

        blocking(permit, move || pool.connect(&target, secret, prompt))
            .await?
            .map_err(|error| connect_error(&error))
    }
}

impl Remotes {
    /// The hosts `~/.ssh/config` names; none when there is no file or no pool.
    pub(super) async fn hosts(&self) -> CommandResult<Vec<SshHost>> {
        let Ok(pool) = self.pool() else {
            return Ok(Vec::new());
        };

        spawn_blocking(move || pool.hosts())
            .await
            .map_err(|_| CommandError::new(IpcErrorCode::Internal, "the hosts could not be read"))
    }

    /// Closes the connection of `site`; a site that is not connected is not an error.
    pub(super) async fn disconnect(&self, site: SiteId) -> CommandResult<()> {
        let Ok(pool) = self.pool() else {
            return Ok(());
        };

        spawn_blocking(move || pool.disconnect(site))
            .await
            .map_err(|_| CommandError::new(IpcErrorCode::Internal, "the connection could not be closed"))
    }
}

/// What the user is asked about a server key that `known_hosts` does not know. The dialog shows it as it is (its own
/// code doubles what its backend reads as a format); the text is cut so that none of its parts can fill the screen.
pub(super) fn question_text(question: &UnknownHost) -> String {
    let outcome = if question.saved {
        "OK saves it to the known_hosts file"
    } else {
        "OK trusts it for this session only (ssh_config saves no known hosts)"
    };

    format!(
        "Trust this server?\n\n\
         The authenticity of {host}:{port} cannot be established.\n\n\
         Key type: {algorithm}\n\
         Fingerprint: {fingerprint}\n\n\
         Only go on if you expect this key. {outcome} and connects; Cancel connects to nothing.",
        host = shown(&question.host, MAX_HOST_SHOWN),
        port = question.port,
        algorithm = shown(&question.algorithm, MAX_KEY_SHOWN),
        fingerprint = shown(&question.fingerprint, MAX_KEY_SHOWN),
    )
}

/// `text` for the dialog: control characters (a line break could fake a line of the question) become `?`, and
/// more than `max` characters are cut with an ellipsis.
fn shown(text: &str, max: usize) -> String {
    let mut shown: String = text
        .chars()
        .take(max)
        .map(|c| if c.is_control() { '?' } else { c })
        .collect();

    if text.chars().nth(max).is_some() {
        shown.push('…');
    }

    shown
}

/// The question about a new host key, in the shell's native dialog.
struct DialogPrompt {
    app: AppHandle,
}

impl HostKeyPrompt for DialogPrompt {
    fn trust_new(&self, question: &UnknownHost) -> bool {
        ask_user(&self.app, question_text(question))
    }
}

/// What the page is told when connecting failed: the pool's own text (fixed, with no host, user, path or secret in it),
/// with the code its kind says. A refused sign-in or host key is `forbidden`, and keeps its text.
pub(super) fn connect_error(error: &io::Error) -> CommandError {
    let code = match error.kind() {
        io::ErrorKind::InvalidInput => IpcErrorCode::InvalidInput,
        io::ErrorKind::Unsupported => IpcErrorCode::Unsupported,
        io::ErrorKind::PermissionDenied => IpcErrorCode::Forbidden,
        io::ErrorKind::TimedOut => IpcErrorCode::Timeout,
        // Too many sites are connected: disconnecting one makes room.
        io::ErrorKind::QuotaExceeded => IpcErrorCode::Busy,
        _ => IpcErrorCode::Io,
    };

    tracing::debug!(kind = ?error.kind(), "connecting to a server failed");

    CommandError::fixed(code, error.to_string())
}

/// The hosts `~/.ssh/config` names, for the suggestions of the Connect dialog; none when there is no file or no pool.
#[tauri::command]
pub async fn remote_hosts(remotes: State<'_, Remotes>) -> CommandResult<Vec<SshHost>> {
    remotes.hosts().await
}

/// Connects to `target`, or says what it needs to (`secretNeeded`: ask the user, then connect again with the `secret`
/// and its `kind`, the one that was asked for or the `other` that was offered). A server connected already is not connected twice: its site is answered. A new host key is asked about in a dialog.
#[tauri::command]
pub async fn remote_connect(
    app: AppHandle,
    remotes: State<'_, Remotes>,
    target: ConnectTarget,
    secret: Option<String>,
    kind: Option<SecretKind>,
) -> CommandResult<ConnectOutcome> {
    remotes
        .connect(target, secret, kind, Arc::new(DialogPrompt { app }))
        .await
}

/// Closes the connection of `site`. A site that is not connected is not an error: the pane that shows it may be
/// closing it a second time.
#[tauri::command]
pub async fn remote_disconnect(remotes: State<'_, Remotes>, site: SiteId) -> CommandResult<()> {
    remotes.disconnect(site).await
}
