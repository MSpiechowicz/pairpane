//! End-to-end tests: a [`Pool`] against the in-process server, with every file it reads or writes below a temporary home.

mod browse;
mod connect;
mod hardening;
mod hostkeys;
#[cfg(unix)]
mod key_files;
mod limits;
mod questions;
mod secrets;
mod ssh_batch;
mod ssh_config;
mod ssh_connect;
mod ssh_env;
mod ssh_policy;
mod ssh_session;
mod ssh_sites;
#[cfg(windows)]
mod ssh_windows;

use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pairpane_core::archive::Password;
use pairpane_core::model::{ConnectOutcome, ConnectTarget, SecretKind, SecretReason, SiteId};
use pairpane_core::vfs::{RemoteFs, RemotePath};
use russh::keys::PrivateKey;
use tempfile::TempDir;

use crate::known_hosts::{Name, append};
use crate::resolve::find_ssh;
use crate::test_server::{self, Settings, TestServer, USER};
use crate::{AgentSource, HostKeyPrompt, Pool, PoolConfig, Secret, Timeouts, UnknownHost};

/// A prompt that answers as told and remembers what it was asked.
pub(super) struct Prompt {
    pub(super) trust: bool,
    pub(super) asked: Mutex<Vec<UnknownHost>>,
}

impl Prompt {
    pub(super) fn new(trust: bool) -> Arc<Self> {
        Arc::new(Self {
            trust,
            asked: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn times(&self) -> usize {
        self.asked.lock().expect("asked").len()
    }
}

impl HostKeyPrompt for Prompt {
    fn trust_new(&self, question: &UnknownHost) -> bool {
        self.asked.lock().expect("asked").push(question.clone());

        self.trust
    }
}

/// Timeouts short enough for a test that waits for one, long enough for a loaded machine to finish a handshake.
pub(super) fn quick() -> Timeouts {
    Timeouts {
        connect: Duration::from_secs(5),
        handshake: Duration::from_secs(10),
        request: Duration::from_secs(10),
        idle: Duration::from_secs(600),
        keepalive: Duration::from_secs(15),
    }
}

/// The OpenSSH client of this machine for a test that needs the real one; `None`, with a note, when there is none. Where
/// `PAIRPANE_REQUIRE_SSH` is set (CI, whose runners have OpenSSH) a missing client fails the test instead.
pub(super) fn ssh_program() -> Option<PathBuf> {
    let found = find_ssh();

    if found.is_none() {
        assert!(
            std::env::var_os("PAIRPANE_REQUIRE_SSH").is_none(),
            "PAIRPANE_REQUIRE_SSH is set and there is no OpenSSH client"
        );
        eprintln!("skipped: there is no OpenSSH client (ssh) on this machine");
    }

    found
}

/// A server, a pool whose home is a temporary folder, and what is needed to talk to the one through the other.
pub(super) struct Harness {
    pub(super) home: TempDir,
    pub(super) server: TestServer,
    pub(super) pool: Pool,
    pub(super) prompt: Arc<Prompt>,
}

impl Harness {
    pub(super) fn new(settings: Settings) -> Self {
        Self::with(settings, quick(), AgentSource::None)
    }

    pub(super) fn with(settings: Settings, timeouts: Timeouts, agent: AgentSource) -> Self {
        Self::build(settings, timeouts, agent, |_, _| None)
    }

    /// A harness whose pool asks the real OpenSSH (`ssh -G -F` a file with the text that `config` gives for the home
    /// folder and the server's port) where hosts lead; `None` (with a note) on a machine that has none. A test that
    /// connects like this must name every file it depends on in the text: the defaults are the real user's.
    pub(super) fn via_ssh(settings: Settings, config: impl FnOnce(&Path, u16) -> String) -> Option<Self> {
        Self::via_ssh_agent(settings, AgentSource::None, config)
    }

    /// The same, with an ssh-agent.
    pub(super) fn via_ssh_agent(
        settings: Settings,
        agent: AgentSource,
        config: impl FnOnce(&Path, u16) -> String,
    ) -> Option<Self> {
        let program = ssh_program()?;

        Some(Self::build(settings, quick(), agent, |home, port| {
            let file = home.join("ssh_config");

            fs::write(&file, config(home, port)).expect("config");

            Some((program, file))
        }))
    }

    fn build(
        settings: Settings,
        timeouts: Timeouts,
        agent: AgentSource,
        ssh: impl FnOnce(&Path, u16) -> Option<(PathBuf, PathBuf)>,
    ) -> Self {
        let home = tempfile::tempdir().expect("temp home");

        fs::create_dir(home.path().join(".ssh")).expect(".ssh");

        let server = TestServer::start(settings);
        let (ssh_program, ssh_config_file) = ssh(home.path(), server.addr.port()).unzip();
        let pool = Pool::new(PoolConfig {
            home: home.path().to_owned(),
            system_known_hosts: Some(home.path().join("system_known_hosts")),
            ssh_program,
            search_ssh: false,
            ssh_config_file,
            agent,
            default_user: Some(USER.to_owned()),
            timeouts,
        })
        .expect("a pool");

        Self {
            home,
            server,
            pool,
            prompt: Prompt::new(true),
        }
    }

    pub(super) fn ssh(&self, name: &str) -> PathBuf {
        self.home.path().join(".ssh").join(name)
    }

    pub(super) fn known_hosts(&self) -> PathBuf {
        self.ssh("known_hosts")
    }

    /// The system's `ssh_known_hosts` the pool reads (not there until a test writes it).
    pub(super) fn system_known_hosts(&self) -> PathBuf {
        self.home.path().join("system_known_hosts")
    }

    pub(super) fn target(&self) -> ConnectTarget {
        target_of(self.server.addr)
    }

    /// Makes the server's key a trusted one, as an earlier "Yes" would have.
    pub(super) fn trust(&self) {
        let key = &self.server.host_key;

        append(
            &self.known_hosts(),
            &Name::of("127.0.0.1", self.server.addr.port()).expect("a valid host"),
            key.algorithm().as_str(),
            &key.to_bytes().expect("key bytes"),
            false,
        )
        .expect("trusted");
    }

    /// A secret of `kind`, as the answer to a question for that kind.
    pub(super) fn secret(kind: SecretKind, text: &str) -> Secret {
        Secret {
            kind,
            value: Password::new(text.to_owned()),
        }
    }

    /// Connects with the harness's prompt and exactly what is given: no question is assumed to have been asked.
    pub(super) fn connect_with(&self, target: &ConnectTarget, secret: Option<Secret>) -> io::Result<ConnectOutcome> {
        self.pool.connect(target, secret, self.prompt.clone())
    }

    /// Connects with the harness's prompt, giving `password` as the answer to a question for the password.
    pub(super) fn connect(&self, password: Option<&str>) -> io::Result<ConnectOutcome> {
        self.connect_to(&self.target(), password)
    }

    pub(super) fn connect_to(&self, target: &ConnectTarget, password: Option<&str>) -> io::Result<ConnectOutcome> {
        self.answer_to(target, password.map(|text| (SecretKind::Password, text)))
    }

    /// Connects with the answer `(kind, text)` to the question the last attempt left.
    pub(super) fn answer(&self, answer: Option<(SecretKind, &str)>) -> io::Result<ConnectOutcome> {
        self.answer_to(&self.target(), answer)
    }

    /// A secret is only taken as the answer to a question: where the test did not get one first, the pool is told it
    /// was asked for this kind.
    fn answer_to(&self, target: &ConnectTarget, answer: Option<(SecretKind, &str)>) -> io::Result<ConnectOutcome> {
        if let Some((kind, _)) = answer {
            self.pool.assume_asked(target, kind);
        }

        self.connect_with(target, answer.map(|(kind, text)| Self::secret(kind, text)))
    }

    /// Connects, expecting it to work, and gives the site.
    pub(super) fn site(&self, secret: Option<&str>) -> SiteId {
        match self.connect(secret).expect("connected") {
            ConnectOutcome::Connected { site, .. } => site,
            other => panic!("expected a connection, got {other:?}"),
        }
    }
}

/// The question a sign-in leaves: `secret` is asked for, and `other` is the kind that is offered instead.
pub(super) fn needed(secret: SecretKind, wrong: bool, other: Option<SecretKind>) -> ConnectOutcome {
    ConnectOutcome::SecretNeeded {
        secret,
        wrong,
        other,
        reason: None,
    }
}

/// The same, with the reason for asking for this kind after the other was answered.
pub(super) fn needed_because(
    secret: SecretKind,
    wrong: bool,
    other: Option<SecretKind>,
    reason: SecretReason,
) -> ConnectOutcome {
    ConnectOutcome::SecretNeeded {
        secret,
        wrong,
        other,
        reason: Some(reason),
    }
}

pub(super) fn target_of(addr: SocketAddr) -> ConnectTarget {
    ConnectTarget {
        host: "127.0.0.1".to_owned(),
        port: Some(addr.port()),
        user: Some(USER.to_owned()),
        key_file: None,
    }
}

/// How many entries `dir` has, listed through `fs`.
pub(super) fn count(fs: &dyn RemoteFs, dir: &str) -> usize {
    fs.read_dir(&path(dir), &AtomicBool::new(false)).expect("listed").len()
}

/// A server that signs in with `key` only: no password.
pub(super) fn key_only(key: &PrivateKey) -> Settings {
    Settings {
        password: None,
        authorized_key: Some(key.public_key().clone()),
        ..Settings::default()
    }
}

pub(super) fn path(text: &str) -> RemotePath {
    RemotePath::new(text).expect("a path")
}

/// An error never says where it went, or what the secret was.
pub(super) fn assert_fixed(error: &io::Error, addr: SocketAddr) {
    let text = error.to_string();

    for secret in [
        "127.0.0.1",
        &addr.port().to_string(),
        "correct horse",
        test_server::HOME,
    ] {
        assert!(!text.contains(secret), "{text:?} holds {secret:?}");
    }
}
