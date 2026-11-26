//! What a [`Pool`](crate::Pool) is set up with: where the user's SSH files are and how long it waits for a server.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use crate::agent::AgentSource;
use crate::error::fail;
use crate::resolve::find_ssh;

/// The limits of a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timeouts {
    /// Opening the TCP connection.
    pub connect: Duration,
    /// The SSH handshake, the sign-in and the start of SFTP together; the time a question to the user takes is not
    /// counted.
    pub handshake: Duration,
    /// One SFTP request.
    pub request: Duration,
    /// How long a site may stay unused before its connection is closed (it opens again, without a question, when it is
    /// used next).
    pub idle: Duration,
    /// How often a quiet connection is asked whether it is alive; three unanswered asks end it.
    pub keepalive: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(15),
            handshake: Duration::from_secs(30),
            request: Duration::from_secs(30),
            idle: Duration::from_secs(600),
            keepalive: Duration::from_secs(15),
        }
    }
}

/// The settings of a [`Pool`](crate::Pool). Nothing is read from the real environment except by
/// [`from_env`](Self::from_env), so a test can point it all at a temporary folder.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// The user's home folder: `.ssh/config`, `.ssh/known_hosts` and the default keys are below it.
    pub home: PathBuf,
    /// The system's `ssh_known_hosts` (`/etc/ssh/ssh_known_hosts`): its keys are trusted and its revoked ones refused.
    /// Only read, never written; `None` for none. Used when there is no [`ssh_program`](Self::ssh_program) to say which
    /// files to read: with one, OpenSSH names them (`GlobalKnownHostsFile`).
    pub system_known_hosts: Option<PathBuf>,
    /// The OpenSSH client, which says where a host leads (`ssh -G`, run with an explicit argument list, never a shell).
    /// `None` for none: the host, user and port are then used as typed, with no `ssh_config`, and the default key files
    /// and `known_hosts` files.
    pub ssh_program: Option<PathBuf>,
    /// Look for the OpenSSH client again at every connect while there is none ([`ssh_program`](Self::ssh_program) is
    /// `None`), so that installing it needs no restart. Off for a pool that is to stay without one (a test).
    pub search_ssh: bool,
    /// A configuration file to give `ssh -G` (`-F`), in place of `~/.ssh/config` and the system's file. For tests; `None`
    /// for OpenSSH's own.
    pub ssh_config_file: Option<PathBuf>,
    /// Where the ssh-agent is: the app's own (`SSH_AUTH_SOCK`), which a host uses unless its `IdentityAgent` names another
    /// socket (or none).
    pub agent: AgentSource,
    /// The user name for a host that has none in the configuration or in the dialog.
    pub default_user: Option<String>,
    pub timeouts: Timeouts,
}

impl PoolConfig {
    /// The real user's home folder, `SSH_AUTH_SOCK`, local user name and OpenSSH client (the first of the well-known
    /// places, then `PATH`). An error when there is no home folder.
    pub fn from_env() -> io::Result<Self> {
        let home = std::env::home_dir().ok_or_else(|| fail(io::ErrorKind::NotFound, "there is no home folder"))?;
        let default_user = ["USER", "USERNAME", "LOGNAME"]
            .iter()
            .find_map(|name| std::env::var(name).ok())
            .filter(|user| !user.is_empty());

        Ok(Self {
            home,
            system_known_hosts: cfg!(unix).then(|| PathBuf::from("/etc/ssh/ssh_known_hosts")),
            ssh_program: find_ssh(),
            search_ssh: true,
            ssh_config_file: None,
            agent: AgentSource::Env,
            default_user,
            timeouts: Timeouts::default(),
        })
    }
}
