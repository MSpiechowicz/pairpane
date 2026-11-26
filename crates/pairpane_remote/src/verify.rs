//! Checking the key a server presents against `known_hosts`, and asking the user about a key that is new.
//!
//! A key that is in the file is accepted, one that is not goes to the app's [`HostKeyPrompt`] (Yes saves a line, No
//! writes nothing), and one that differs from the file's is refused with no question: there is no way to override it
//! from here. A connection that is opened again after it dropped is held to the key it had ([`Policy::Pinned`]).

use std::borrow::Cow;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use russh::client::{ChannelOpenHandle, Handler, Msg, Session};
use russh::keys::{Algorithm, HashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{Channel, ChannelOpenFailure, Preferred};

use crate::error::fail;
use crate::known_hosts::{self, KnownHosts, Name, Verdict};

/// A server key that `known_hosts` does not know, as the question shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownHost {
    /// The host name or address the user connects to.
    pub host: String,
    pub port: u16,
    /// The kind of key, as OpenSSH names it (`ssh-ed25519`).
    pub algorithm: String,
    /// The SHA-256 fingerprint, as OpenSSH shows it (`SHA256:` and base64).
    pub fingerprint: String,
    /// Whether a "Yes" is saved to a `known_hosts` file. `false` where the configuration has no file to save to
    /// (`UserKnownHostsFile none` or `/dev/null`): the key is then trusted for this run only.
    pub saved: bool,
}

/// What the app asks the user when a server's key is new. It is called on a thread that may block (a dialog is fine),
/// never from the app's UI thread, and at most once per connection.
pub trait HostKeyPrompt: Send + Sync {
    /// Whether to trust the key (and save it to `known_hosts` when `question.saved`). `false` also when the user closes the
    /// question.
    fn trust_new(&self, question: &UnknownHost) -> bool;
}

/// Handler methods that turn down a channel the server opens towards this client. Browsing opens its own channels and
/// has nothing to offer to the ones a server opens (forwarded ports, an agent, X11, a session, a direct connection): a
/// server that tries is refused, not answered.
macro_rules! refuse_channels {
    ($($name:ident($($ty:ty),*);)*) => {$(
        async fn $name(
            &mut self,
            _channel: Channel<Msg>,
            $(_: $ty,)*
            reply: ChannelOpenHandle,
            _session: &mut Session,
        ) -> Result<(), Self::Error> {
            reply.reject(ChannelOpenFailure::AdministrativelyProhibited).await;

            Ok(())
        }
    )*};
}

/// The most host keys the user accepted that are kept in memory.
const MAX_ACCEPTED: usize = 64;

/// The host keys the user accepted in this run, whether or not they could be saved to `known_hosts`: a "Yes" that could
/// not be written (a read-only `~/.ssh`) is not asked about again at the next step of the same sign-in.
#[derive(Debug, Default)]
pub(crate) struct Accepted {
    /// `known_hosts`' name of the server and the key in wire form, the oldest first.
    keys: Mutex<Vec<(String, Vec<u8>)>>,
}

impl Accepted {
    fn holds(&self, name: &str, blob: &[u8]) -> bool {
        let keys = self.keys.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        keys.iter().any(|(held, key)| held == name && key == blob)
    }

    fn add(&self, name: String, blob: Vec<u8>) {
        let mut keys = self.keys.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        if keys.len() >= MAX_ACCEPTED {
            keys.remove(0);
        }

        keys.push((name, blob));
    }
}

/// What is done with a server key that `known_hosts` does not hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NewKey {
    /// The user is asked (`StrictHostKeyChecking` ask, accept-new, no, off).
    Ask,
    /// Refused: `StrictHostKeyChecking yes`.
    Strict,
    /// Refused: `BatchMode yes`, which forbids every question.
    Batch,
}

impl NewKey {
    /// What `StrictHostKeyChecking yes` and `BatchMode yes` (as printed by `ssh -G`) make of it.
    pub(crate) fn of(strict: bool, batch_mode: bool) -> Self {
        if strict {
            Self::Strict
        } else if batch_mode {
            Self::Batch
        } else {
            Self::Ask
        }
    }
}

/// What a connection holds the server's key to.
pub(crate) enum Policy {
    /// `known_hosts` as it was read, and who to ask about a new key; a key that is accepted is appended to `path` (when
    /// there is one; as a hashed name when `hash`) and remembered in `accepted`. A new key is refused, not asked about,
    /// where `new_key` says so.
    Known {
        hosts: KnownHosts,
        path: Option<PathBuf>,
        hash: bool,
        new_key: NewKey,
        prompt: Arc<dyn HostKeyPrompt>,
        accepted: Arc<Accepted>,
    },
    /// Exactly this key: the one an open site was connected with.
    Pinned(PublicKey),
}

impl Policy {
    /// The host key kinds to ask the server for: the kinds this policy already holds a key of, so that a server that
    /// could present a known key does, and one that can only present another kind is asked about it. `Default` when
    /// nothing narrows the choice.
    pub(crate) fn preferred(&self, name: &Name) -> Preferred {
        let kinds: Vec<String> = match self {
            Self::Known { hosts, .. } => hosts.algorithms(name),
            Self::Pinned(key) => vec![key.algorithm().as_str().to_owned()],
        };
        let mut algorithms: Vec<Algorithm> = Vec::new();

        for kind in &kinds {
            match kind.parse::<Algorithm>() {
                // RSA keys are signed with SHA-2 where the server can; the key is the same.
                Ok(Algorithm::Rsa { .. }) => algorithms.extend([
                    Algorithm::Rsa {
                        hash: Some(HashAlg::Sha512),
                    },
                    Algorithm::Rsa {
                        hash: Some(HashAlg::Sha256),
                    },
                    Algorithm::Rsa { hash: None },
                ]),
                Ok(algorithm @ (Algorithm::Ed25519 | Algorithm::Ecdsa { .. })) => algorithms.push(algorithm),
                _ => {}
            }
        }

        if algorithms.is_empty() {
            return Preferred::default();
        }

        Preferred {
            key: Cow::Owned(algorithms),
            ..Preferred::default()
        }
    }
}

/// Why a key was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// `known_hosts` holds a different key, or revokes this one, on this line.
    Changed(usize),
    /// Another known_hosts file than the user's first (the system's, say) holds a different key, or revokes this one.
    ChangedSystem,
    /// The user said no.
    Declined,
    /// The key is not the one the site was opened with.
    NotPinned,
    /// The server offered a certificate, which is not supported.
    Certificate,
    /// `known_hosts` vouches for the server in a form that cannot be checked here.
    Unusable,
    /// `StrictHostKeyChecking yes`, and `known_hosts` does not hold the key.
    Strict,
    /// `BatchMode yes`, and `known_hosts` does not hold the key.
    Batch,
}

impl Refusal {
    pub(crate) fn into_error(self) -> io::Error {
        match self {
            Self::Changed(line) => io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "the server's host key differs from the one in known_hosts line {line}; the connection was refused"
                ),
            ),
            Self::ChangedSystem => fail(
                io::ErrorKind::PermissionDenied,
                "the server's host key differs from the one in another known_hosts file (the system's, or one that ssh_config names); the connection was refused",
            ),
            Self::Declined => fail(io::ErrorKind::PermissionDenied, "the server's host key was not trusted"),
            Self::NotPinned => fail(
                io::ErrorKind::PermissionDenied,
                "the server's host key is not the one this connection started with",
            ),
            Self::Certificate => fail(io::ErrorKind::PermissionDenied, "host certificates are not supported"),
            Self::Unusable => fail(
                io::ErrorKind::PermissionDenied,
                "known_hosts vouches for this server in a form Pairpane cannot check (a certificate authority or an unsupported key type); connect with ssh first",
            ),
            Self::Strict => fail(
                io::ErrorKind::PermissionDenied,
                "ssh_config requires a known host key for this server (StrictHostKeyChecking yes); add it with ssh first",
            ),
            Self::Batch => fail(
                io::ErrorKind::PermissionDenied,
                "ssh_config sets BatchMode, so Pairpane will not ask about a new host key; add it with ssh first",
            ),
        }
    }
}

/// What the check leaves behind for the code that opened the connection (the handler itself is gone into russh).
#[derive(Clone, Default)]
pub(crate) struct Outcome {
    refusal: Arc<Mutex<Option<Refusal>>>,
    key: Arc<Mutex<Option<PublicKey>>>,
    /// Set while the user is being asked, so the time does not count against the connection's limit.
    pub(crate) asking: Arc<AtomicBool>,
    /// The user was asked, and said yes.
    asked: Arc<AtomicBool>,
}

impl Outcome {
    pub(crate) fn refusal(&self) -> Option<Refusal> {
        *self.refusal.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The key that was accepted.
    pub(crate) fn key(&self) -> Option<PublicKey> {
        self.key.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// Whether the user was asked about the key and said yes.
    pub(crate) fn was_asked(&self) -> bool {
        self.asked.load(Ordering::Acquire)
    }

    fn refuse(&self, refusal: Refusal) {
        *self.refusal.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(refusal);
    }

    fn accept(&self, key: PublicKey) {
        *self.key.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(key);
    }
}

/// The handshake clock is stopped while this lives, even when the question panics.
struct Pause<'a>(&'a AtomicBool);

impl<'a> Pause<'a> {
    fn start(flag: &'a AtomicBool) -> Self {
        flag.store(true, Ordering::Release);

        Self(flag)
    }
}

impl Drop for Pause<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// The client handler of a connection: it only checks the server's key.
pub(crate) struct Verifier {
    host: String,
    port: u16,
    /// What `known_hosts` calls the server.
    name: Name,
    policy: Policy,
    outcome: Outcome,
}

impl Verifier {
    pub(crate) fn new(host: &str, port: u16, name: Name, policy: Policy, outcome: Outcome) -> Self {
        Self {
            host: host.to_owned(),
            port,
            name,
            policy,
            outcome,
        }
    }

    /// Whether to accept `key`, asking and saving as the policy says.
    async fn judge(&self, key: &PublicKey) -> Result<(), Refusal> {
        let (hosts, (path, hash), new_key, prompt, accepted) = match &self.policy {
            Policy::Pinned(pinned) if pinned == key => return Ok(()),
            Policy::Pinned(_) => return Err(Refusal::NotPinned),
            Policy::Known {
                hosts,
                path,
                hash,
                new_key,
                prompt,
                accepted,
            } => (hosts, (path, *hash), *new_key, prompt, accepted),
        };

        let blob = key.to_bytes().map_err(|_| Refusal::Certificate)?;
        let algorithm = key.algorithm().as_str().to_owned();

        match hosts.check(&self.name, &algorithm, &blob) {
            Verdict::Known => return Ok(()),
            Verdict::Changed { line } => return Err(Refusal::Changed(line)),
            Verdict::ChangedSystem => return Err(Refusal::ChangedSystem),
            Verdict::Unusable => return Err(Refusal::Unusable),
            Verdict::Unknown => {}
        }

        // Never asked, never trusted for the first time here: `ssh` would not connect either.
        match new_key {
            NewKey::Ask => {}
            NewKey::Strict => return Err(Refusal::Strict),
            NewKey::Batch => return Err(Refusal::Batch),
        }

        // Accepted earlier in this run, and the file could not take it.
        let name = self.name.as_str().to_owned();

        if accepted.holds(&name, &blob) {
            return Ok(());
        }

        let question = UnknownHost {
            host: self.host.clone(),
            port: self.port,
            algorithm: algorithm.clone(),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            saved: path.is_some(),
        };

        if !self.ask(Arc::clone(prompt), question).await {
            return Err(Refusal::Declined);
        }

        self.outcome.asked.store(true, Ordering::Release);

        accepted.add(name, blob.clone());
        self.save(path.clone(), hash, algorithm, blob).await;

        Ok(())
    }

    /// Shows the question on a blocking thread. The clock of the handshake stops when the question is on the screen, not
    /// when it is queued for a thread: a busy thread pool must not make a connection wait for free.
    async fn ask(&self, prompt: Arc<dyn HostKeyPrompt>, question: UnknownHost) -> bool {
        let asking = Arc::clone(&self.outcome.asking);
        let answer = tokio::task::spawn_blocking(move || {
            let _paused = Pause::start(&asking);

            prompt.trust_new(&question)
        })
        .await;

        answer.unwrap_or(false)
    }

    /// The user said yes: a line that cannot be written costs the memory of the answer, not this connection.
    async fn save(&self, path: Option<PathBuf>, hash: bool, algorithm: String, blob: Vec<u8>) {
        let Some(path) = path else {
            tracing::warn!(
                "there is no known_hosts file to save the host key to; it is trusted for this connection only"
            );

            return;
        };
        let name = self.name.clone();
        let saved =
            tokio::task::spawn_blocking(move || known_hosts::append(&path, &name, &algorithm, &blob, hash)).await;

        if !matches!(saved, Ok(Ok(()))) {
            tracing::warn!("the host key could not be saved to known_hosts; it is trusted for this connection only");
        }
    }
}

impl Handler for Verifier {
    type Error = russh::Error;

    async fn check_server_key(&mut self, presented: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = presented else {
            self.outcome.refuse(Refusal::Certificate);

            return Ok(false);
        };

        match self.judge(key).await {
            Ok(()) => {
                self.outcome.accept(key.clone());

                Ok(true)
            }
            Err(refusal) => {
                self.outcome.refuse(refusal);

                Ok(false)
            }
        }
    }

    refuse_channels! {
        server_channel_open_forwarded_tcpip(&str, u32, &str, u32);
        server_channel_open_forwarded_streamlocal(&str);
        server_channel_open_agent_forward();
        server_channel_open_session();
        server_channel_open_direct_tcpip(&str, u32, &str, u32);
        server_channel_open_direct_streamlocal(&str);
        server_channel_open_x11(&str, u32);
    }
}
