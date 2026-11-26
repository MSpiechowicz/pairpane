//! A server in this process and the app's remote state connected to it, for the tests of the commands. Every file the
//! pool reads or writes is below a temporary home; nothing touches the real `~/.ssh` or agent.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pairpane_core::model::{ConnectOutcome, ConnectTarget, SecretKind, SiteId};
use pairpane_remote::test_server::{Settings, TestServer, USER};
use pairpane_remote::{AgentSource, HostKeyPrompt, Pool, PoolConfig, Timeouts, UnknownHost};
use tauri::async_runtime::block_on;
use tempfile::TempDir;

use super::{Remotes, ThreadCap};
use crate::error::CommandResult;

/// The password the server accepts by default.
pub(in crate::commands) const PASSWORD: &str = "correct horse";

/// A prompt that answers as told and counts its questions.
pub(in crate::commands) struct Answers {
    pub(in crate::commands) trust: bool,
    pub(in crate::commands) asked: AtomicUsize,
}

impl Answers {
    pub(in crate::commands) fn new(trust: bool) -> Arc<Self> {
        Arc::new(Self {
            trust,
            asked: AtomicUsize::new(0),
        })
    }
}

impl HostKeyPrompt for Answers {
    fn trust_new(&self, _question: &UnknownHost) -> bool {
        self.asked.fetch_add(1, Ordering::Relaxed);

        self.trust
    }
}

/// A running server and the remote state whose pool has its home in a temporary folder.
pub(in crate::commands) struct Fixture {
    pub(in crate::commands) home: TempDir,
    pub(in crate::commands) server: TestServer,
    pub(in crate::commands) remotes: Remotes,
}

impl Fixture {
    pub(in crate::commands) fn new() -> Self {
        let home = tempfile::tempdir().expect("temp home");
        let pool = Pool::new(PoolConfig {
            home: home.path().to_owned(),
            system_known_hosts: None,
            ssh_program: None,
            search_ssh: false,
            ssh_config_file: None,
            agent: AgentSource::None,
            default_user: Some(USER.to_owned()),
            timeouts: Timeouts::default(),
        })
        .expect("a pool");

        Self {
            home,
            server: TestServer::start(Settings::default()),
            remotes: Remotes::with(pool),
        }
    }

    pub(in crate::commands) fn target(&self) -> ConnectTarget {
        ConnectTarget {
            host: "127.0.0.1".to_owned(),
            port: Some(self.server.addr.port()),
            user: Some(USER.to_owned()),
            key_file: None,
        }
    }

    /// `remote_connect` with `password` (when there is one) as the answer, and `prompt` answering about a new host key.
    pub(in crate::commands) fn connect(
        &self,
        password: Option<&str>,
        prompt: &Arc<Answers>,
    ) -> CommandResult<ConnectOutcome> {
        let answer = password.map(|text| (SecretKind::Password, text));

        self.answer(answer, prompt)
    }

    /// `remote_connect` with the secret `(kind, text)` as the answer to a question of that kind.
    pub(in crate::commands) fn answer(
        &self,
        answer: Option<(SecretKind, &str)>,
        prompt: &Arc<Answers>,
    ) -> CommandResult<ConnectOutcome> {
        // A secret answers a question: where the test did not get one first, the pool is told it was asked for this kind.
        if let (Some((kind, _)), Ok(pool)) = (answer, self.remotes.pool()) {
            pool.assume_asked(&self.target(), kind);
        }

        block_on(self.remotes.connect(
            self.target(),
            answer.map(|(_, text)| text.to_owned()),
            answer.map(|(kind, _)| kind),
            Arc::clone(prompt) as Arc<dyn HostKeyPrompt>,
        ))
    }

    /// A connected site, as the password and a trusting prompt make one.
    pub(in crate::commands) fn site(&self) -> SiteId {
        match self.connect(Some(PASSWORD), &Answers::new(true)).expect("connected") {
            ConnectOutcome::Connected { site, .. } => site,
            other => panic!("expected a connection, got {other:?}"),
        }
    }
}

/// The remote state of an app whose pool could not be made.
pub(in crate::commands) fn without_pool() -> Remotes {
    Remotes {
        pool: None,
        connects: ThreadCap::default(),
    }
}
