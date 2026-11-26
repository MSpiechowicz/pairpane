//! Signing in, in the order a user expects: the ssh-agent, the key files (the one the user named, the configured ones,
//! then `id_ed25519`, `id_ecdsa`, `id_rsa` where OpenSSH did not list them), and last the password, which the server may
//! take as a password or as the answer to a single hidden keyboard-interactive prompt. A two-step sign-in is not
//! supported: one prompt is answered, and a later round that asks something is not.
//!
//! What the user's `ssh_config` allows ([`Allowed`]) is kept to: a way that is off is not tried, `PreferredAuthentications`
//! sets the order of the ways (the agent comes before the key files, and the password before its keyboard-interactive
//! form), and the questions below are only asked for ways that are on.
//!
//! A secret is bound to what it was asked for ([`Secret`]): a passphrase only decrypts a local key file and is never
//! sent to the server, and a password is only sent to the server and never decrypts a key. When nothing works, the
//! answer says which kind to ask for next, whether the last one was wrong, and which other kind would also do.
//!
//! What is asked when nothing signed in (`locked`: an encrypted key file stayed shut; `password`: the server takes a
//! password or a keyboard-interactive prompt, and the configuration allows it). A key is only looked at when the server
//! takes public keys and the configuration allows them, so an encrypted key is the way in that OpenSSH would try first,
//! and the passphrase is asked for first, as it does:
//!
//! | situation                                          | asked      | other      | reason           |
//! |----------------------------------------------------|------------|------------|------------------|
//! | no key locked, password possible                   | password   | none       | none             |
//! | the same, after a passphrase that opened a key     | password   | none       | key not accepted |
//! | key locked, no password possible                   | passphrase | none       | none             |
//! | key locked, password possible, first question      | passphrase | password   | none             |
//! | the same, after a passphrase that opened nothing   | passphrase | password   | none             |
//! | the same, after a refused password                 | password   | passphrase | none             |
//!
//! Nothing is asked for a way the configuration switched off: with `PasswordAuthentication no` and
//! `KbdInteractiveAuthentication no` (or a `PreferredAuthentications` without them) there is no password question, with
//! `PubkeyAuthentication no` no key is opened and so no passphrase is asked; with neither way the sign-in is refused.
//!
//! The reason is given when the kind asked is not the one just answered: the passphrase opened a key and the server did
//! not accept it.
//!
//! With `BatchMode yes` or `NumberOfPasswordPrompts 0` nothing is asked: where the table says to ask, the sign-in is
//! refused instead (a fixed message), as `ssh` would not prompt either.
//!
//! Known limits, not mistakes: one passphrase is asked for all the locked keys (a key it does not open stays shut, and
//! the question comes again); the keys of the agent share the budget of [`MAX_KEY_TRIES`] with the key files, so an
//! agent that holds many keys can leave none of the budget for a key file (`ssh` lets the server's `MaxAuthTries` decide);
//! and only the methods of the table above are tried, in the order of `PreferredAuthentications`.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pairpane_core::archive::Password;
use pairpane_core::model::{SecretKind, SecretReason};
use russh::MethodKind;
use russh::client::{Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, decode_secret_key};
use zeroize::Zeroizing;

use crate::agent::{self, AgentSource};
use crate::allowed::{Allowed, Phase};
use crate::error::{fail, lost};
use crate::regular::{Opening, open_regular};
use crate::verify::Verifier;

/// The most public keys offered to a server: servers drop a client after about six failed tries (`MaxAuthTries`), and
/// the password needs one of them.
const MAX_KEY_TRIES: usize = 4;

/// The largest key file read.
const MAX_KEY_FILE: u64 = 64 * 1024;

/// How many rounds of questions a keyboard-interactive exchange may take.
const MAX_ROUNDS: usize = 3;

/// A secret the user gave, and what it was asked for (the `kind` of the [`ConnectOutcome::SecretNeeded`] it answers).
///
/// [`ConnectOutcome::SecretNeeded`]: pairpane_core::model::ConnectOutcome::SecretNeeded
#[derive(Debug, Clone)]
pub struct Secret {
    pub kind: SecretKind,
    pub value: Password,
}

/// The secrets a sign-in may use, each only in its own role.
#[derive(Debug, Clone, Default)]
pub(crate) struct Secrets {
    /// Sent to the server, as a password or as the answer to a hidden prompt.
    password: Option<Password>,
    /// Opens encrypted key files on this machine; never leaves it.
    passphrase: Option<Password>,
}

impl Secrets {
    pub(crate) fn of(secret: Option<Secret>) -> Self {
        let mut secrets = Self::default();

        match secret {
            Some(Secret {
                kind: SecretKind::Password,
                value,
            }) => secrets.password = Some(value),
            Some(Secret {
                kind: SecretKind::Passphrase,
                value,
            }) => secrets.passphrase = Some(value),
            None => {}
        }

        secrets
    }
}

/// A private key file to try.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyFile {
    pub(crate) path: PathBuf,
    /// The user asked for this file (the dialog or an `IdentityFile`), as against one of the default names.
    pub(crate) named: bool,
}

/// What the sign-in has to work with.
pub(crate) struct Credentials<'a> {
    pub(crate) user: &'a str,
    pub(crate) agent: &'a AgentSource,
    pub(crate) key_files: &'a [KeyFile],
    pub(crate) secrets: &'a Secrets,
    pub(crate) allowed: &'a Allowed,
}

/// How the sign-in ended, when it did not fail outright.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Ending {
    /// Signed in.
    In,
    /// A secret is needed (or the one given was not accepted).
    Secret {
        kind: SecretKind,
        wrong: bool,
        /// The other kind that would also let the user in.
        other: Option<SecretKind>,
        /// Why this kind is asked for after the other was answered.
        reason: Option<SecretReason>,
    },
}

/// What was learned along the way.
#[derive(Default)]
struct Progress {
    tries: usize,
    /// An encrypted key file could not be opened (no passphrase, or the wrong one).
    locked: bool,
    /// A passphrase was given and did not open one.
    bad_passphrase: bool,
    /// A passphrase was given and opened one (the server may still not take that key).
    opened: bool,
}

pub(crate) async fn sign_in(handle: &mut Handle<Verifier>, credentials: &Credentials<'_>) -> io::Result<Ending> {
    let methods = match handle.authenticate_none(credentials.user).await.map_err(|_| lost())? {
        russh::client::AuthResult::Success => return Ok(Ending::In),
        russh::client::AuthResult::Failure { remaining_methods, .. } => remaining_methods,
    };
    let mut progress = Progress::default();
    let password = credentials.secrets.password.as_ref();

    for phase in credentials.allowed.phases() {
        let signed_in = match (phase, password) {
            (Phase::PublicKey, _) if methods.contains(&MethodKind::PublicKey) => {
                public_keys(handle, credentials, &mut progress).await?
            }
            (Phase::Password, Some(password)) if methods.contains(&MethodKind::Password) => {
                try_password(handle, credentials.user, password).await
            }
            (Phase::Keyboard, Some(password)) if methods.contains(&MethodKind::KeyboardInteractive) => {
                try_keyboard_interactive(handle, credentials.user, password).await
            }
            _ => false,
        };

        if signed_in {
            return Ok(Ending::In);
        }
    }

    let password_possible = (credentials.allowed.allows(Phase::Password) && methods.contains(&MethodKind::Password))
        || (credentials.allowed.allows(Phase::Keyboard) && methods.contains(&MethodKind::KeyboardInteractive));

    question(
        &progress,
        credentials.secrets,
        password_possible,
        credentials.allowed.ask,
    )
}

/// The agent's keys (when the configuration allows the agent), then the key files.
async fn public_keys(
    handle: &mut Handle<Verifier>,
    credentials: &Credentials<'_>,
    progress: &mut Progress,
) -> io::Result<bool> {
    if credentials.allowed.agent && agent::sign_in(handle, credentials, &mut progress.tries, MAX_KEY_TRIES).await {
        return Ok(true);
    }

    try_key_files(handle, credentials, progress).await
}

/// What to ask for when nothing signed in; see the table at the top of this file. The passphrase of a locked key comes
/// first; after an answer the question is the kind that was answered (so that a wrong answer is reported for what it
/// was), with the other kind offered to switch to.
fn question(progress: &Progress, secrets: &Secrets, password_possible: bool, may_ask: bool) -> io::Result<Ending> {
    let ask = |kind, wrong, other, reason| {
        if !may_ask {
            return Err(fail(
                io::ErrorKind::PermissionDenied,
                "ssh_config forbids asking for a password or a passphrase (BatchMode, or NumberOfPasswordPrompts 0); use a key that needs none, or connect with ssh",
            ));
        }

        Ok(Ending::Secret {
            kind,
            wrong,
            other,
            reason,
        })
    };
    let password_given = secrets.password.is_some();

    match (progress.locked, password_possible) {
        (false, false) => Err(denied()),
        (false, true) => ask(
            SecretKind::Password,
            password_given,
            None,
            progress.opened.then_some(SecretReason::KeyNotAccepted),
        ),
        (true, false) => ask(SecretKind::Passphrase, progress.bad_passphrase, None, None),
        (true, true) if password_given => ask(SecretKind::Password, true, Some(SecretKind::Passphrase), None),
        (true, true) => ask(
            SecretKind::Passphrase,
            progress.bad_passphrase,
            Some(SecretKind::Password),
            None,
        ),
    }
}

fn denied() -> io::Error {
    fail(
        io::ErrorKind::PermissionDenied,
        "the server did not accept any way of signing in",
    )
}

async fn try_key_files(
    handle: &mut Handle<Verifier>,
    credentials: &Credentials<'_>,
    progress: &mut Progress,
) -> io::Result<bool> {
    for file in credentials.key_files {
        if progress.tries >= MAX_KEY_TRIES {
            break;
        }

        let key = match load_key(file.path.clone(), credentials.secrets.passphrase.clone()).await {
            Loaded::Key(key) => key,
            Loaded::Opened(key) => {
                progress.opened = true;

                key
            }
            Loaded::Locked => {
                progress.locked = true;

                continue;
            }
            Loaded::Refused => {
                progress.locked = true;
                progress.bad_passphrase = true;

                continue;
            }
            Loaded::NotRegular if file.named => {
                return Err(fail(io::ErrorKind::InvalidInput, "the key file is not a regular file"));
            }
            Loaded::NotRegular => {
                tracing::debug!("a default key file is not a regular file; it is skipped");

                continue;
            }
            Loaded::Skip => continue,
        };

        progress.tries += 1;

        let hash = rsa_hash(handle, key.algorithm().is_rsa()).await;
        let signed_in = handle
            .authenticate_publickey(credentials.user, PrivateKeyWithHashAlg::new(Arc::new(*key), hash))
            .await;

        if signed_in.is_ok_and(|result| result.success()) {
            return Ok(true);
        }
    }

    Ok(false)
}

/// The hash an RSA key (`rsa`) signs with: the best one the server accepts. Other keys have no choice.
pub(crate) async fn rsa_hash(handle: &Handle<Verifier>, rsa: bool) -> Option<HashAlg> {
    if !rsa {
        return None;
    }

    handle.best_supported_rsa_hash().await.ok().flatten().flatten()
}

/// What reading a key file came to.
enum Loaded {
    Key(Box<PrivateKey>),
    /// Encrypted, and the passphrase given opened it.
    Opened(Box<PrivateKey>),
    /// Encrypted, and no passphrase was given.
    Locked,
    /// Encrypted, and the passphrase given does not open it.
    Refused,
    /// Not there, not readable, or not a key.
    Skip,
    /// Something that is not a regular file (a pipe or a device would wait for ever; a folder holds no key).
    NotRegular,
}

/// The key in `path`, decrypted with `passphrase` when it needs it. The file is read (and a passphrase turned into a
/// key, which is slow by design) on a blocking thread.
async fn load_key(path: PathBuf, passphrase: Option<Password>) -> Loaded {
    tokio::task::spawn_blocking(move || read_key(&path, passphrase.as_ref()))
        .await
        .unwrap_or(Loaded::Skip)
}

fn read_key(path: &Path, passphrase: Option<&Password>) -> Loaded {
    let file = match open_regular(path) {
        Ok(file) => file,
        Err(Opening::NotRegular) => return Loaded::NotRegular,
        Err(Opening::Missing | Opening::Unreadable) => return Loaded::Skip,
    };

    let mut text = Zeroizing::new(String::new());

    if file.take(MAX_KEY_FILE).read_to_string(&mut text).is_err() {
        return Loaded::Skip;
    }

    match decode_secret_key(&text, None) {
        Ok(key) => Loaded::Key(Box::new(key)),
        Err(russh::keys::Error::KeyIsEncrypted) => match passphrase {
            None => Loaded::Locked,
            Some(passphrase) => decode_secret_key(&text, Some(passphrase.as_str()))
                .map_or(Loaded::Refused, |key| Loaded::Opened(Box::new(key))),
        },
        Err(_) => Loaded::Skip,
    }
}

/// The password, as a password.
async fn try_password(handle: &mut Handle<Verifier>, user: &str, password: &Password) -> bool {
    matches!(
        handle.authenticate_password(user, password.as_str()).await,
        Ok(russh::client::AuthResult::Success)
    )
}

/// The password as the answer to a keyboard-interactive exchange. Only one round that asks something is answered, and
/// only when it is a single hidden prompt: a server that goes on to ask more (a verification code) is not given the
/// password again, and the sign-in stops there. Rounds that ask nothing are answered with nothing.
async fn try_keyboard_interactive(handle: &mut Handle<Verifier>, user: &str, secret: &Password) -> bool {
    let Ok(mut response) = handle.authenticate_keyboard_interactive_start(user, None).await else {
        return false;
    };
    let mut answered = false;

    for _ in 0..MAX_ROUNDS {
        let answers = match response {
            KeyboardInteractiveAuthResponse::Success => return true,
            KeyboardInteractiveAuthResponse::Failure { .. } => return false,
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => match prompts.as_slice() {
                [] => Vec::new(),
                [prompt] if !prompt.echo && !answered => {
                    answered = true;

                    vec![secret.as_str().to_owned()]
                }
                _ => return false,
            },
        };

        response = match handle.authenticate_keyboard_interactive_respond(answers).await {
            Ok(next) => next,
            Err(_) => return false,
        };
    }

    matches!(response, KeyboardInteractiveAuthResponse::Success)
}
