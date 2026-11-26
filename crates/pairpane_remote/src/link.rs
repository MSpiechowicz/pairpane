//! One open connection to a server: the SSH session and the SFTP channel on it, and how one is made.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use pairpane_core::model::{SecretKind, SecretReason};
use russh::client::{self, Handle};
use russh::keys::PublicKey;
use russh::{ChannelMsg, Disconnect};
use russh_sftp::client::RawSftpSession;
use tokio::net::TcpStream;

use crate::agent::AgentSource;
use crate::allowed::Allowed;
use crate::auth::{self, Credentials, Ending, KeyFile, Secrets};
use crate::config::PoolConfig;
use crate::error::{fail, from_sftp, from_ssh, from_tcp, lost};
use crate::known_hosts::Name;
use crate::runtime::{limited, within};
use crate::verify::{Outcome, Policy, Verifier};

/// How long a goodbye may take.
const GOODBYE: Duration = Duration::from_millis(500);

/// The most messages skipped while waiting for the answer to the SFTP request.
const MAX_SKIPPED: usize = 16;

/// Where to connect and as whom, with everything a connection (and a later one in its place) needs.
#[derive(Debug, Clone)]
pub(crate) struct Target {
    /// What `known_hosts` is asked about and what is connected to: a host name or an address, checked by
    /// [`valid_host`](crate::known_hosts::valid_host).
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) user: String,
    /// The key files to try, in order.
    pub(crate) key_files: Vec<KeyFile>,
    /// What `known_hosts` calls the server: the host and port, or the `HostKeyAlias`.
    pub(crate) known_as: Name,
    /// The ways of signing in that the user's configuration allows.
    pub(crate) allowed: Allowed,
    /// Where the ssh-agent is for this host.
    pub(crate) agent: AgentSource,
}

/// A connection that is open.
pub(crate) struct Link {
    handle: Handle<Verifier>,
    pub(crate) sftp: Arc<RawSftpSession>,
    /// The key the server proved itself with.
    pub(crate) host_key: PublicKey,
}

impl Link {
    pub(crate) fn is_open(&self) -> bool {
        !self.handle.is_closed()
    }

    /// Says goodbye and closes; does not wait for an answer for long.
    pub(crate) async fn close(&self) {
        let _ = self.sftp.close_session();
        let _ = limited(GOODBYE, async {
            self.handle
                .disconnect(Disconnect::ByApplication, "", "en")
                .await
                .map_err(|_| lost())
        })
        .await;
    }
}

/// How an attempt to open a connection ended.
pub(crate) enum Opened {
    Open {
        link: Arc<Link>,
        /// The folder the server starts in.
        home: String,
    },
    Secret {
        kind: SecretKind,
        wrong: bool,
        other: Option<SecretKind>,
        reason: Option<SecretReason>,
    },
}

/// Connects to `target`, signs in and starts SFTP.
pub(crate) async fn open(
    config: &PoolConfig,
    target: &Target,
    policy: Policy,
    secrets: &Secrets,
) -> io::Result<Opened> {
    let timeouts = &config.timeouts;
    let socket = limited(timeouts.connect, async {
        let socket = TcpStream::connect((target.host.as_str(), target.port))
            .await
            .map_err(|error| from_tcp(&error))?;

        // Small requests and answers: do not wait to fill a packet.
        let _ = socket.set_nodelay(true);

        Ok(socket)
    })
    .await?;

    let outcome = Outcome::default();
    let client_config = Arc::new(client::Config {
        keepalive_interval: Some(timeouts.keepalive),
        keepalive_max: 3,
        inactivity_timeout: None,
        preferred: policy.preferred(&target.known_as),
        ..client::Config::default()
    });
    let verifier = Verifier::new(
        &target.host,
        target.port,
        target.known_as.clone(),
        policy,
        outcome.clone(),
    );

    let opened = within(timeouts.handshake, &outcome.asking, async {
        let mut handle = client::connect_stream(client_config, socket, verifier)
            .await
            .map_err(|error| {
                outcome
                    .refusal()
                    .map_or_else(|| from_ssh(&error), |refusal| refusal.into_error())
            })?;
        let credentials = Credentials {
            user: &target.user,
            agent: &target.agent,
            key_files: &target.key_files,
            secrets,
            allowed: &target.allowed,
        };

        match auth::sign_in(&mut handle, &credentials).await? {
            Ending::Secret {
                kind,
                wrong,
                other,
                reason,
            } => {
                let _ = handle.disconnect(Disconnect::ByApplication, "", "en").await;

                Ok(Opened::Secret {
                    kind,
                    wrong,
                    other,
                    reason,
                })
            }
            Ending::In => {
                let host_key = outcome.key().ok_or_else(lost)?;
                let (sftp, home) = start_sftp(&handle, config).await?;

                Ok(Opened::Open {
                    link: Arc::new(Link { handle, sftp, host_key }),
                    home,
                })
            }
        }
    })
    .await;

    opened.map_err(|error| after_the_question(&outcome, error))
}

/// What a connection that went while the user was being asked about a new key says, in place of "the connection was
/// lost": the server's own time for a sign-in ran out, and nothing the user did was wrong.
fn after_the_question(outcome: &Outcome, error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::ConnectionAborted && outcome.was_asked() {
        return fail(
            io::ErrorKind::ConnectionAborted,
            "the server closed the connection while waiting for an answer; connect again",
        );
    }

    error
}

/// The SFTP channel of a signed-in session, and the folder it starts in.
async fn start_sftp(handle: &Handle<Verifier>, config: &PoolConfig) -> io::Result<(Arc<RawSftpSession>, String)> {
    let mut channel = handle.channel_open_session().await.map_err(|_| lost())?;

    channel.request_subsystem(true, "sftp").await.map_err(|_| lost())?;

    let mut skipped = 0;

    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => break,
            Some(ChannelMsg::Failure) | None => {
                return Err(fail(io::ErrorKind::Unsupported, "the server has no SFTP service"));
            }
            Some(_) if skipped < MAX_SKIPPED => skipped += 1,
            Some(_) => return Err(fail(io::ErrorKind::InvalidData, "the server sent something unexpected")),
        }
    }

    let sftp = RawSftpSession::new(channel.into_stream());

    // The caller's own limit is the one that counts; this one only frees what a stalled request holds.
    sftp.set_timeout(config.timeouts.request.as_secs() + 2);
    sftp.init().await.map_err(from_sftp)?;

    let home = sftp
        .realpath(".")
        .await
        .map_err(from_sftp)?
        .files
        .into_iter()
        .next()
        .map(|file| file.filename)
        .ok_or_else(|| {
            fail(
                io::ErrorKind::InvalidData,
                "the server did not say where the home folder is",
            )
        })?;

    Ok((Arc::new(sftp), home))
}
