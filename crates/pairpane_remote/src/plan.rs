//! From what the Connect dialog collected to where to connect: the host, port, user and keys, with what OpenSSH says
//! about the host (see [`resolve`](crate::resolve)).

use std::io;
use std::path::PathBuf;

use pairpane_core::model::ConnectTarget;

use crate::auth::KeyFile;
use crate::config::PoolConfig;
use crate::error::fail;
use crate::known_hosts::{Files, SSH_PORT, valid_host};
use crate::link::Target;
use crate::resolve::{Asked, resolve};
use crate::verify::NewKey;

/// The key files tried when none is named and there is no OpenSSH to list them, as OpenSSH does.
const DEFAULT_KEYS: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

/// Every name of a key that OpenSSH lists by default (`ssh -G` prints them all when the configuration names no
/// `IdentityFile`): a file by one of these names in `~/.ssh` is a default and not one that the user asked for.
const LISTED_DEFAULTS: [&str; 8] = [
    "id_rsa",
    "id_ecdsa",
    "id_ecdsa_sk",
    "id_ed25519",
    "id_ed25519_sk",
    "id_xmss",
    "id_dsa",
    "id_mldsa44_ed25519",
];

/// A connection to make.
#[derive(Debug)]
pub(crate) struct Plan {
    pub(crate) target: Target,
    /// What the path bar calls the server: `user@host`, with the port when it is not 22.
    pub(crate) label: String,
    /// The `known_hosts` files to check the server's key against, and where an accepted key is written.
    pub(crate) known_hosts: Files,
    /// What happens to a server key that is not known.
    pub(crate) new_key: NewKey,
    /// `HashKnownHosts yes`: a key that is saved is saved under a hashed name.
    pub(crate) hash_known_hosts: bool,
}

fn invalid(message: &'static str) -> io::Error {
    fail(io::ErrorKind::InvalidInput, message)
}

/// The characters OpenSSH's `valid_ruser` does not let into a user name (they could be taken for shell syntax where a
/// configuration puts the name in a command), and `@`, which the label could not tell from the host.
const USER_REFUSED: &str = "'`\"$\\;&<>|(){}@";

/// A user name OpenSSH would take: no leading `-`, no control character or white space, none of [`USER_REFUSED`].
fn valid_user(user: &str) -> bool {
    !user.is_empty()
        && user.len() <= 64
        && !user.starts_with('-')
        && !user
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || USER_REFUSED.contains(c))
}

/// Blocks while OpenSSH is asked (see [`resolve`]): not for an async task.
pub(crate) fn plan(config: &PoolConfig, input: &ConnectTarget) -> io::Result<Plan> {
    let typed = input.host.trim();

    // A name that starts with `-` could be taken for an option by the program that is asked about it.
    if !valid_host(typed) || typed.starts_with('-') {
        return Err(invalid("the host name is not valid"));
    }

    if input.port == Some(0) {
        return Err(invalid("the port is not valid"));
    }

    let user = input.user.as_deref().map(str::trim).filter(|user| !user.is_empty());

    if user.is_some_and(|user| !valid_user(user)) {
        return Err(invalid("the user name is not valid"));
    }

    let resolved = resolve(
        config,
        &Asked {
            host: typed,
            user,
            port: input.port,
        },
    )?;

    if resolved.port == 0 {
        return Err(invalid("the port is not valid"));
    }

    let user = user
        .map(str::to_owned)
        .or(resolved.user)
        .or_else(|| config.default_user.clone())
        .ok_or_else(|| invalid("there is no user name"))?;

    if !valid_user(&user) {
        return Err(invalid("the user name is not valid"));
    }

    let label = if resolved.port == SSH_PORT {
        format!("{user}@{typed}")
    } else {
        format!("{user}@{typed}:{}", resolved.port)
    };

    Ok(Plan {
        target: Target {
            host: resolved.host_name,
            port: resolved.port,
            user,
            key_files: key_files(
                config,
                input.key_file.as_deref(),
                resolved.identity_files,
                resolved.identities_listed,
            ),
            known_as: resolved.known_as,
            allowed: resolved.allowed,
            agent: resolved.agent,
        },
        label,
        known_hosts: resolved.known_hosts,
        new_key: resolved.new_key,
        hash_known_hosts: resolved.hash_known_hosts,
    })
}

/// The key to try first (as typed), then the configured ones; each once, and the first mention counts. When OpenSSH
/// listed the identity files (`listed`; it lists the default ones when the configuration names none, and then none else is
/// tried, as for `ssh`) they are all there is; with no OpenSSH the default names follow. A configured file that is one of
/// the names OpenSSH lists by default counts as a default.
fn key_files(config: &PoolConfig, named: Option<&str>, configured: Vec<PathBuf>, listed: bool) -> Vec<KeyFile> {
    let named = named
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(|path| match path.strip_prefix("~/") {
            Some(rest) => config.home.join(rest),
            None => config.home.join(path),
        });
    let ssh_folder = config.home.join(".ssh");
    let defaults: Vec<PathBuf> = DEFAULT_KEYS.iter().map(|name| ssh_folder.join(name)).collect();
    let typed = named.into_iter().map(|path| (path, true));
    let configured = configured.into_iter().map(|path| {
        let default = LISTED_DEFAULTS.iter().any(|name| path == ssh_folder.join(name));

        (path, !default)
    });
    let fallback = defaults.iter().filter(|_| !listed).cloned().map(|path| (path, false));
    let mut files: Vec<KeyFile> = Vec::new();

    for (path, named) in typed.chain(configured).chain(fallback) {
        if !files.iter().any(|file| file.path == path) {
            files.push(KeyFile { path, named });
        }
    }

    files
}
