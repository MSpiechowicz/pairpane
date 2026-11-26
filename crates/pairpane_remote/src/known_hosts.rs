//! OpenSSH's `~/.ssh/known_hosts`: which server keys the user trusts, and the system's `ssh_known_hosts`, which is read
//! the same way (its keys count as known, its `@revoked` lines refuse) and never written.
//!
//! Read: one host or `[host]:port`, a comma list of them, wildcards, `!` exclusions and hashed (`|1|`) names. A line
//! with `@revoked` refuses its key. A line for the host that cannot be used here (`@cert-authority`, or a key kind that
//! is not supported: `ssh-dss`, the security-key kinds) is not ignored: when no usable line settles the key, the server
//! is refused ([`Verdict::Unusable`]) rather than asked about, as the user's file vouches for it in a way that cannot be
//! checked. Written: one
//! line (plain, or hashed under `HashKnownHosts yes`) appended for a key the user accepted, which creates `~/.ssh` (0700) and the file (0600) on Unix and never
//! rewrites what is there.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, KeyInit, Mac};
use russh::keys::Algorithm;
use sha1::Sha1;

use crate::error::fail;
use crate::pattern;
use crate::regular::{Opening, open_regular};

/// The most that is read of the file; a bigger one is a mistake, and refused rather than half read.
const MAX_FILE: u64 = 16 * 1024 * 1024;

/// The port that `known_hosts` leaves out of a name, and that SSH uses when none is given.
pub(crate) const SSH_PORT: u16 = 22;

/// What `known_hosts` says about a key a server presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// A line for this host holds this key.
    Known,
    /// No line for this host holds a key of this kind.
    Unknown,
    /// A line for this host holds a different key of the same kind, or this key is revoked: `line` is its number.
    Changed { line: usize },
    /// The same, in another file than the user's first (whose line numbers mean nothing to the user).
    ChangedSystem,
    /// No usable line holds the key, but a line for this host cannot be used here (a certificate authority, or a key
    /// kind that is not supported): the file vouches for the server in a form that cannot be checked.
    Unusable,
}

#[derive(Debug, PartialEq, Eq)]
enum Marker {
    None,
    Revoked,
    CertAuthority,
}

#[derive(Debug)]
enum Hosts {
    /// `|1|salt|hash`: the HMAC-SHA1 of the name, keyed by the salt.
    Hashed { salt: Vec<u8>, hash: Vec<u8> },
    /// Patterns, `!` before one excludes.
    Patterns(Vec<String>),
}

#[derive(Debug)]
struct Line {
    number: usize,
    marker: Marker,
    hosts: Hosts,
    algorithm: String,
    key: Vec<u8>,
    /// The line is from another file than the user's first (the system's, or one more that `ssh_config` names).
    system: bool,
}

impl Line {
    fn parse(number: usize, text: &str) -> Option<Self> {
        let mut fields = text.split_whitespace();
        let mut first = fields.next()?;

        if first.starts_with('#') {
            return None;
        }

        let marker = match first {
            "@revoked" => Marker::Revoked,
            "@cert-authority" => Marker::CertAuthority,
            _ => Marker::None,
        };

        if marker != Marker::None {
            first = fields.next()?;
        }

        let algorithm = fields.next()?.to_owned();
        let key = STANDARD.decode(fields.next()?).ok()?;

        Some(Self {
            number,
            marker,
            hosts: Hosts::parse(first)?,
            algorithm,
            key,
            system: false,
        })
    }

    fn changed(&self) -> Verdict {
        if self.system {
            Verdict::ChangedSystem
        } else {
            Verdict::Changed { line: self.number }
        }
    }

    /// Whether a key of this line's kind can be checked here: the kinds `russh` makes host keys of.
    fn usable(&self) -> bool {
        matches!(
            self.algorithm.parse::<Algorithm>(),
            Ok(Algorithm::Ed25519 | Algorithm::Ecdsa { .. } | Algorithm::Rsa { .. })
        )
    }

    fn matches(&self, name: &str) -> bool {
        match &self.hosts {
            Hosts::Hashed { salt, hash } => {
                let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(salt) else {
                    return false;
                };

                mac.update(name.as_bytes());
                mac.verify_slice(hash).is_ok()
            }
            Hosts::Patterns(patterns) => pattern::selected(patterns.iter().map(String::as_str), name),
        }
    }
}

impl Hosts {
    fn parse(field: &str) -> Option<Self> {
        let Some(hashed) = field.strip_prefix("|1|") else {
            return Some(Self::Patterns(field.split(',').map(str::to_owned).collect()));
        };

        let (salt, hash) = hashed.split_once('|')?;

        Some(Self::Hashed {
            salt: STANDARD.decode(salt).ok()?,
            hash: STANDARD.decode(hash).ok()?,
        })
    }
}

/// The lines of a `known_hosts` file.
#[derive(Debug, Default)]
pub(crate) struct KnownHosts {
    lines: Vec<Line>,
}

impl KnownHosts {
    pub(crate) fn parse(text: &str) -> Self {
        let lines = text
            .lines()
            .enumerate()
            .filter_map(|(index, text)| Line::parse(index + 1, text))
            .collect();

        Self { lines }
    }

    /// The file at `path`; a file that is not there holds nothing. Any other failure is an error: a file that cannot be
    /// read must not be taken for one without the key that would have stopped the connection.
    pub(crate) fn load(path: &Path) -> io::Result<Self> {
        let file = match open_regular(path) {
            Ok(file) => file,
            Err(Opening::Missing) => return Ok(Self::default()),
            Err(_) => return Err(unreadable()),
        };

        let mut bytes = Vec::new();

        file.take(MAX_FILE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| unreadable())?;

        if bytes.len() as u64 > MAX_FILE {
            return Err(unreadable());
        }

        Ok(Self::parse(&String::from_utf8_lossy(&bytes)))
    }

    /// The files `ssh` would read, as one: a file that is not there holds nothing. Only the first of the user's files has
    /// line numbers the user can use; a line of any other file is reported as "another file".
    pub(crate) fn load_all(files: &Files) -> io::Result<Self> {
        let mut hosts = Self::default();

        let user = files.user.iter().enumerate().map(|(index, path)| (path, index > 0));
        let global = files.global.iter().map(|path| (path, true));

        for (path, other) in user.chain(global) {
            let mut lines = Self::load(path)?.lines;

            for line in &mut lines {
                line.system = other;
            }

            hosts.lines.append(&mut lines);
        }

        Ok(hosts)
    }

    /// What the file says about `algorithm`'s key `key` (its wire form) of the server called `name`.
    pub(crate) fn check(&self, name: &Name, algorithm: &str, key: &[u8]) -> Verdict {
        let name = name.as_str();
        let mut known = false;
        let mut unusable = false;
        let mut changed: Option<Verdict> = None;

        for line in self.lines.iter().filter(|line| line.matches(name)) {
            match line.marker {
                Marker::CertAuthority => unusable = true,
                Marker::Revoked => {
                    if line.key == key {
                        return line.changed();
                    }
                }
                Marker::None => {
                    unusable |= !line.usable();

                    if line.algorithm != algorithm {
                        continue;
                    }

                    if line.key == key {
                        known = true;
                    } else if changed.is_none() {
                        changed = Some(line.changed());
                    }
                }
            }
        }

        match (known, changed) {
            (true, _) => Verdict::Known,
            (false, Some(verdict)) => verdict,
            (false, None) if unusable => Verdict::Unusable,
            (false, None) => Verdict::Unknown,
        }
    }

    /// The key kinds the file holds for the server called `name`, in file order.
    pub(crate) fn algorithms(&self, name: &Name) -> Vec<String> {
        let name = name.as_str();
        let mut kinds: Vec<String> = Vec::new();

        for line in self
            .lines
            .iter()
            .filter(|line| line.marker == Marker::None && line.matches(name))
        {
            if !kinds.contains(&line.algorithm) {
                kinds.push(line.algorithm.clone());
            }
        }

        kinds
    }
}

/// The files `ssh` reads for a host (`UserKnownHostsFile` and `GlobalKnownHostsFile`); the user's first one is where a
/// key that is accepted is written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Files {
    pub(crate) user: Vec<PathBuf>,
    /// The user's files whose paths were put together from several words of what `ssh -G` printed (a guess): a key is not
    /// saved to one of these when its folder is not there, for a folder is not made on a guess.
    pub(crate) joined: Vec<PathBuf>,
    /// Only read, never written.
    pub(crate) global: Vec<PathBuf>,
}

impl Files {
    /// Where an accepted key is appended: the first of the user's files, and none when it is a guessed path whose folder
    /// does not exist (the key is then trusted for the session only).
    pub(crate) fn writable(&self) -> Option<&Path> {
        let first = self.user.first()?;

        if self.joined.contains(first) && !first.parent().is_some_and(Path::is_dir) {
            return None;
        }

        Some(first.as_path())
    }
}

/// How `known_hosts` spells a server: the host (in lower case), with the port in brackets unless it is 22; or a
/// `HostKeyAlias`, as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Name(String);

impl Name {
    /// `None` for a host that cannot be written into a line (see [`valid_host`]).
    pub(crate) fn of(host: &str, port: u16) -> Option<Self> {
        if !valid_host(host) {
            return None;
        }

        let host = host.to_lowercase();

        Some(Self(if port == SSH_PORT {
            host
        } else {
            format!("[{host}]:{port}")
        }))
    }

    /// `None` for an alias that cannot be written into a line.
    pub(crate) fn alias(alias: &str) -> Option<Self> {
        valid_host(alias).then(|| Self(alias.to_lowercase()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

fn unreadable() -> io::Error {
    fail(io::ErrorKind::Other, "the known_hosts file cannot be read")
}

/// A host name that is safe to write into a line and to look up: letters, digits and `. - _ : %` only, so it can hold
/// no space, no newline and none of the characters that make a pattern (`* ? ! , | [ ]`).
pub(crate) fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '%'))
}

/// `name` as a hashed `known_hosts` name, `|1|salt|hash`: the HMAC-SHA1 of the name keyed by a random salt of 20 bytes
/// (what `HashKnownHosts yes` writes).
fn hashed(name: &str) -> io::Result<String> {
    let mut salt = [0u8; 20];

    getrandom::fill(&mut salt).map_err(|_| fail(io::ErrorKind::Other, "no random numbers are available"))?;

    let mut mac =
        Hmac::<Sha1>::new_from_slice(&salt).map_err(|_| fail(io::ErrorKind::Other, "no hash is available"))?;

    mac.update(name.as_bytes());

    Ok(format!(
        "|1|{}|{}",
        STANDARD.encode(salt),
        STANDARD.encode(mac.finalize().into_bytes())
    ))
}

/// Appends the line that trusts `key` (of `algorithm`, in wire form) for the server called `name`, under its name as it
/// is or (`hash`) as a hashed name. The file ends in a newline afterwards, and nothing before the new line is changed.
pub(crate) fn append(path: &Path, name: &Name, algorithm: &str, key: &[u8], hash: bool) -> io::Result<()> {
    if algorithm.contains(char::is_whitespace) {
        return Err(fail(io::ErrorKind::InvalidInput, "the key type is not valid"));
    }

    if let Some(folder) = path.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        create_folder(folder)?;
    }

    let mut options = OpenOptions::new();

    options.read(true).append(true).create(true);
    private_file(&mut options);

    let mut file = options.open(path)?;
    let mut line = String::new();

    if !ends_with_newline(&mut file)? {
        line.push('\n');
    }

    let written = if hash {
        hashed(name.as_str())?
    } else {
        name.as_str().to_owned()
    };

    line.push_str(&format!("{written} {algorithm} {}\n", STANDARD.encode(key)));
    file.write_all(line.as_bytes())
}

/// Whether the file is empty or its last byte is a newline.
fn ends_with_newline(file: &mut fs::File) -> io::Result<bool> {
    if file.metadata()?.len() == 0 {
        return Ok(true);
    }

    let mut last = [0u8];

    file.seek(SeekFrom::End(-1))?;
    file.read_exact(&mut last)?;

    Ok(last[0] == b'\n')
}

#[cfg(unix)]
fn create_folder(folder: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    DirBuilder::new().recursive(true).mode(0o700).create(folder)
}

#[cfg(not(unix))]
fn create_folder(folder: &Path) -> io::Result<()> {
    DirBuilder::new().recursive(true).create(folder)
}

#[cfg(unix)]
fn private_file(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.mode(0o600);
}

#[cfg(not(unix))]
fn private_file(_options: &mut OpenOptions) {}
