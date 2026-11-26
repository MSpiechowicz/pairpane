//! Where a host leads, as OpenSSH says it: the real `ssh -G`, run as a fixed program with an explicit argument list
//! (never a shell), so that `Host`/`Match` blocks, `Include`s, `CanonicalizeHostname` and the system's `ssh_config` are
//! OpenSSH's own and not a second opinion.
//!
//! What is used of the answer: the `HostName`, `User`, `Port`, `IdentityFile`s, the `known_hosts` files, `HostKeyAlias`,
//! `StrictHostKeyChecking`, `HashKnownHosts`, `IdentityAgent`, `BatchMode`, `NumberOfPasswordPrompts` and the options that
//! allow or forbid a way of signing in (see [`Allowed`]). A host that OpenSSH would reach through a proxy (`ProxyJump`,
//! `ProxyCommand`), or whose keys are judged by a `RevokedHostKeys` file or a `KnownHostsCommand` (which are not read
//! here), is refused: connecting to it directly would be a different route, or a weaker check, than `ssh` takes. So is a
//! configuration that allows no way of signing in that Pairpane has.
//!
//! The question is put as `sftp` puts it to `ssh` (`-s <host> sftp`, with `RemoteCommand` switched off: `sftp` itself fails on a host that has one), so that `Match
//! sessiontype subsystem`/`Match command sftp` blocks (which can give a host a proxy, a stricter host key check or other
//! `known_hosts` files for file transfers) are the ones that apply. `Match sessiontype` needs an OpenSSH that has it: an
//! older one refuses such a configuration, and so does Pairpane (`ssh -G` fails).
//!
//! The `Match exec` commands of the user's `ssh_config` are run by `ssh -G` while it works out which blocks apply, as they
//! are when `ssh` connects; Pairpane runs nothing itself, and a command that takes too long is ended with everything it
//! started (see `run`). `ssh -G` is given the app's own environment (cleaned for an AppImage, without `LD_*` and
//! `DYLD_*`), so a command that tests a variable, or a `${VAR}` in a path, comes out as it does for the user's own `ssh`.
//!
//! With no OpenSSH on the machine ([`PoolConfig::ssh_program`], looked for again at every connect when the pool was made
//! with [`PoolConfig::search_ssh`]) the host, user and port are used as typed, with the default key files and
//! `known_hosts` files, and only when the user has no `~/.ssh/config`: a configuration that cannot be read is not
//! guessed at, and every connect is refused.
//!
//! A host name is made lower case (`HostKeyAlias` too), as OpenSSH 10 does for the name it looks up in `known_hosts`.

mod known;
mod parse;
mod run;

use std::io::{self, Read};
use std::path::{Path, PathBuf};

pub(crate) use self::run::find_ssh;
use crate::agent::{self, AgentSource};
use crate::allowed::Allowed;
use crate::config::PoolConfig;
use crate::error::fail;
use crate::known_hosts::{Files, Name, SSH_PORT, valid_host};
use crate::regular::{Opening, open_regular};
use crate::verify::NewKey;

const PROXY: &str = "ssh_config sends this host through a proxy, which Pairpane does not support; connect with ssh";

const KEYS_BY_OTHERS: &str =
    "ssh_config names revoked host keys or a known-hosts command, which Pairpane does not support; connect with ssh";

const NO_WAY_IN: &str = "ssh_config allows no way of signing in that Pairpane supports";

const NO_SSH: &str = "OpenSSH was not found, so ~/.ssh/config cannot be read; install OpenSSH (ssh) to connect";

/// The most that is read of `~/.ssh/config` to see whether it has anything in it.
const MAX_CONFIG: u64 = 1024 * 1024;

/// What the Connect dialog collected.
pub(crate) struct Asked<'a> {
    /// The host as typed, which is also what `Host` lines are matched against.
    pub(crate) host: &'a str,
    pub(crate) user: Option<&'a str>,
    pub(crate) port: Option<u16>,
}

/// What a host leads to.
#[derive(Debug)]
pub(crate) struct Resolved {
    /// The name or address to connect to: valid for [`Name`].
    pub(crate) host_name: String,
    pub(crate) user: Option<String>,
    pub(crate) port: u16,
    /// The `IdentityFile`s as absolute paths, in the order they are given.
    pub(crate) identity_files: Vec<PathBuf>,
    pub(crate) known_hosts: Files,
    /// What `known_hosts` calls the server.
    pub(crate) known_as: Name,
    /// What happens to a server key that is not known.
    pub(crate) new_key: NewKey,
    /// `HashKnownHosts yes`: a key that is saved is saved under a hashed name.
    pub(crate) hash_known_hosts: bool,
    /// The ways of signing in that the configuration allows.
    pub(crate) allowed: Allowed,
    /// Where the ssh-agent is (`IdentityAgent`).
    pub(crate) agent: AgentSource,
    /// The identity files are the configuration's (`ssh -G` lists the default ones when it names none) and not the default
    /// key names that Pairpane would try.
    pub(crate) identities_listed: bool,
}

/// The OpenSSH client to ask: the one the pool was given, or (for a pool made with [`PoolConfig::search_ssh`]) the one
/// that is there now, so that installing it needs no restart.
pub(crate) fn ssh_program(config: &PoolConfig) -> Option<PathBuf> {
    match &config.ssh_program {
        Some(program) => Some(program.clone()),
        None if config.search_ssh => find_ssh(),
        None => None,
    }
}

/// `asked` resolved: by `ssh -G` when there is an OpenSSH client, else as typed. Blocks for a few seconds at the most;
/// call it on a thread that may block, never from an async task.
///
/// Errors: `InvalidInput` (OpenSSH cannot read the configuration, or says something unusable; no OpenSSH and a
/// configuration of the user's), `Unsupported` (a proxy, host keys judged by others, no way of signing in that is
/// supported), `TimedOut`, `Other` (OpenSSH could
/// not be started). The messages are fixed.
pub(crate) fn resolve(config: &PoolConfig, asked: &Asked<'_>) -> io::Result<Resolved> {
    let Some(program) = ssh_program(config) else {
        return unconfigured(config, asked);
    };
    let output = run::run(&program, arguments(config, asked), run::LIMIT)?;
    let answer = parse::parse(&output)?;

    if answer.proxied {
        return Err(fail(io::ErrorKind::Unsupported, PROXY));
    }

    if answer.keys_by_others {
        return Err(fail(io::ErrorKind::Unsupported, KEYS_BY_OTHERS));
    }

    let host_name = answer.host_name.ok_or_else(unusable)?;

    // What is connected to, and written to the prompt, whatever name the keys are looked up under.
    if !valid_host(&host_name) {
        return Err(not_valid());
    }

    let port = answer.port.or(asked.port).unwrap_or(SSH_PORT);
    let known_as = match &answer.host_key_alias {
        Some(alias) => Name::alias(alias),
        None => Name::of(&host_name, port),
    }
    .ok_or_else(not_valid)?;
    let context = Context {
        home: &config.home,
        local_user: config.default_user.as_deref(),
        remote_user: answer.user.as_deref(),
        host: &host_name,
        port,
        typed: asked.host,
    };
    let identity_files = answer
        .identity_files
        .iter()
        .filter_map(|file| expand(file, &context))
        .collect();
    let allowed = Allowed::of(&answer.auth);

    // Nothing to try: no connection is made to find that out.
    if allowed.phases().is_empty() {
        return Err(fail(io::ErrorKind::Unsupported, NO_WAY_IN));
    }

    let program_data = std::env::var("ProgramData").ok();
    let rules = known::Rules::here(&config.home, program_data.as_deref());

    let user_files = known::files(&answer.user_known_hosts, &rules)?;
    let global_files = known::files(&answer.global_known_hosts, &rules)?;

    Ok(Resolved {
        known_hosts: Files {
            user: user_files.paths,
            joined: user_files.joined,
            global: global_files.paths,
        },
        known_as,
        new_key: NewKey::of(
            matches!(answer.strict.as_deref(), Some("true" | "yes")),
            answer.auth.batch_mode,
        ),
        hash_known_hosts: answer.hash_known_hosts,
        agent: agent::source(
            answer.auth.identity_agent.as_deref(),
            &config.agent,
            &config.home,
            |name| std::env::var_os(name),
        ),
        allowed,
        identities_listed: true,
        identity_files,
        user: answer.user,
        host_name,
        port,
    })
}

/// The host as typed, and the files `ssh` reads by default (the user's `known_hosts`, the system's), for a machine with
/// no OpenSSH: and only when the user has no `ssh_config`, which only OpenSSH can read.
fn unconfigured(config: &PoolConfig, asked: &Asked<'_>) -> io::Result<Resolved> {
    if has_config(&config.home) {
        return Err(fail(io::ErrorKind::InvalidInput, NO_SSH));
    }

    let port = asked.port.unwrap_or(SSH_PORT);
    let known_as = Name::of(asked.host, port).ok_or_else(not_valid)?;

    Ok(Resolved {
        host_name: asked.host.to_owned(),
        user: None,
        port,
        identity_files: Vec::new(),
        known_hosts: Files {
            user: vec![config.home.join(".ssh").join("known_hosts")],
            joined: Vec::new(),
            global: config.system_known_hosts.iter().cloned().collect(),
        },
        known_as,
        new_key: NewKey::Ask,
        hash_known_hosts: false,
        allowed: Allowed::default(),
        agent: config.agent.clone(),
        identities_listed: false,
    })
}

/// Whether `~/.ssh/config` is there and says anything (a line that is not blank and not a comment). A file that is there
/// and cannot be read counts: what it says is not known.
fn has_config(home: &Path) -> bool {
    let file = match open_regular(&home.join(".ssh").join("config")) {
        Ok(file) => file,
        Err(Opening::Missing) => return false,
        Err(_) => return true,
    };
    let mut bytes = Vec::new();

    if file.take(MAX_CONFIG + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_CONFIG {
        return true;
    }

    String::from_utf8_lossy(&bytes)
        .lines()
        .map(str::trim)
        .any(|line| !line.is_empty() && !line.starts_with('#'))
}

fn not_valid() -> io::Error {
    fail(io::ErrorKind::InvalidInput, "the host name is not valid")
}

fn unusable() -> io::Error {
    fail(
        io::ErrorKind::InvalidInput,
        "ssh gave a value that Pairpane cannot use for this host",
    )
}

/// The arguments of `ssh -G` for a host, as `sftp` runs its `ssh` (`ssh [-l user] [-p port] -s host sftp`) and the way
/// this client opens the connection (an `sftp` subsystem session): `Match sessiontype subsystem` and `Match command sftp`
/// blocks apply as they do for `sftp host`, and not as they do for a login shell. What was typed for the user and port is
/// given as options, so that it wins over the configuration as it does for `ssh -l -p`. `RemoteCommand` is switched off
/// here (`sftp` does not do it, and fails on such a host), for `ssh` takes no command on the command line next to one in
/// the configuration; a `Match command` block never matches the configured `RemoteCommand`, so no block is lost. The host follows
/// `--`, and no option can be made of it.
fn arguments(config: &PoolConfig, asked: &Asked<'_>) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = vec!["-G".into()];

    if let Some(file) = &config.ssh_config_file {
        args.extend(["-F".into(), file.into()]);
    }

    if let Some(user) = asked.user {
        args.extend(["-l".into(), user.into()]);
    }

    if let Some(port) = asked.port {
        args.extend(["-p".into(), port.to_string().into()]);
    }

    args.extend(["-o".into(), "RemoteCommand=none".into(), "-s".into()]);
    args.extend(["--".into(), asked.host.into(), "sftp".into()]);

    args
}

/// What the `%` tokens of an `IdentityFile` stand for.
struct Context<'a> {
    home: &'a Path,
    local_user: Option<&'a str>,
    remote_user: Option<&'a str>,
    host: &'a str,
    port: u16,
    typed: &'a str,
}

/// An `IdentityFile` as an absolute path: `ssh -G` leaves `~` and `%d %u %h %r %p %n %%` in it. A file whose name has
/// another token, or that is not absolute, is left out.
fn expand(file: &str, context: &Context<'_>) -> Option<PathBuf> {
    if file == "none" {
        return None;
    }

    let mut text = String::new();
    let mut chars = file.chars();

    while let Some(c) = chars.next() {
        if c != '%' {
            text.push(c);

            continue;
        }

        match chars.next()? {
            '%' => text.push('%'),
            'd' => text.push_str(&context.home.to_string_lossy()),
            'u' => text.push_str(context.local_user?),
            'r' => text.push_str(context.remote_user?),
            'h' => text.push_str(context.host),
            'p' => text.push_str(&context.port.to_string()),
            'n' => text.push_str(context.typed),
            _ => return None,
        }
    }

    let path = match text.strip_prefix("~/") {
        Some(rest) => context.home.join(rest),
        None if text == "~" => context.home.to_owned(),
        None => PathBuf::from(text),
    };

    path.is_absolute().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context<'a>(home: &'a Path) -> Context<'a> {
        Context {
            home,
            local_user: Some("me"),
            remote_user: Some("bob"),
            host: "files.example",
            port: 2200,
            typed: "files",
        }
    }

    #[test]
    fn an_identity_file_is_made_absolute_and_its_tokens_are_filled_in() {
        let home = std::env::temp_dir().join("home");
        let context = context(&home);

        assert_eq!(expand("~/.ssh/id", &context), Some(home.join(".ssh/id")));
        assert_eq!(expand("~", &context), Some(home.clone()));
        assert_eq!(
            expand("%d/k/%u-%r-%h-%p-%n-100%%", &context),
            Some(PathBuf::from(format!(
                "{}/k/me-bob-files.example-2200-files-100%",
                home.display()
            )))
        );
        assert_eq!(expand("none", &context), None);
        assert_eq!(expand("relative/key", &context), None);
        assert_eq!(expand("~other/key", &context), None);
        assert_eq!(expand("%l/key", &context), None);
        assert_eq!(expand("%", &context), None);
    }
}
