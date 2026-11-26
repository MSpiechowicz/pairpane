//! The set of servers the app is connected to: [`Pool`].

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use pairpane_core::model::{ConnectOutcome, ConnectTarget, SecretKind, SiteId, SshHost};
use pairpane_core::vfs::{RemoteFs, RemotePath};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::auth::{Secret, Secrets};
use crate::config::PoolConfig;
use crate::error::{closed, fail};
use crate::known_hosts::KnownHosts;
use crate::link::{self, Opened};
use crate::plan::{Plan, plan};
use crate::questions::{Question, Questions};
use crate::resolve::ssh_program;
use crate::runtime::{Owned, call};
use crate::sftp::SftpFs;
use crate::site::{Fresh, Site, SiteKey};
use crate::suggestions;
use crate::terms::Terms;
use crate::verify::{Accepted, HostKeyPrompt, Policy};

/// The most servers connected at once.
pub const MAX_SITES: usize = 8;

/// How many connections are being set up at once; the others wait their turn.
const CONCURRENT_CONNECTS: usize = 2;

/// How long [`Pool::shut_down`] gives the connections to say goodbye, and then the runtime to stop.
const GOODBYES: Duration = Duration::from_millis(700);
const STOP: Duration = Duration::from_millis(200);

/// The servers the app is connected to, and the runtime that serves them.
///
/// Every method is blocking and none may be called from an async task: the app calls them from its own threads. A
/// connection has its own time limits (see [`Timeouts`](crate::Timeouts)), so a call never waits forever on a server that
/// does not answer.
pub struct Pool {
    shared: Arc<Shared>,
}

struct Shared {
    config: Arc<PoolConfig>,
    runtime: Owned,
    sites: Mutex<HashMap<SiteId, Arc<Site>>>,
    next_site: AtomicU64,
    connecting: Semaphore,
    down: AtomicBool,
    /// The host keys the user accepted in this run.
    accepted: Arc<Accepted>,
    /// The last question asked about each server, so that an answer is only taken for what was asked.
    questions: Mutex<Questions<SiteKey>>,
}

impl Pool {
    /// A pool with no connections. Starts the runtime (two threads) and the closing of idle connections.
    pub fn new(config: PoolConfig) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            config: Arc::new(config),
            runtime: Owned::new()?,
            sites: Mutex::new(HashMap::new()),
            next_site: AtomicU64::new(1),
            connecting: Semaphore::new(CONCURRENT_CONNECTS),
            down: AtomicBool::new(false),
            accepted: Arc::default(),
            questions: Mutex::new(Questions::new()),
        });

        shared.runtime.handle().spawn(close_idle(Arc::downgrade(&shared)));

        Ok(Self { shared })
    }

    /// Connects to `target`, or says what is needed to: the `secret` is the user's answer to an earlier
    /// [`ConnectOutcome::SecretNeeded`], `None` the first time. It is used for what it was asked for and nothing else: a
    /// password goes to the server, a passphrase only opens a key file on this machine and is never sent.
    ///
    /// The `secret` must answer the question that was asked about this server last (its kind is the one asked for, or the
    /// `other` that was offered): `InvalidInput` for an answer with no question or to another one. A server (same user,
    /// host and port) that is connected already is not connected again: its site is returned, and a failure of its
    /// connection that was remembered is forgotten, so that the next call tries again. An
    /// unknown server key goes to `prompt`; a key that differs from `known_hosts` is refused with `PermissionDenied`.
    /// Other errors: `InvalidInput` (the target, a configuration OpenSSH cannot read, or a `~/.ssh/config` with no OpenSSH
    /// to read it, or a server that is open already under looser settings than this connect asks for: a stricter host key
    /// check, other `known_hosts` files, fewer ways of signing in), `Unsupported` (a host that `ssh_config` sends through
    /// a proxy or whose keys it has others judge, a configuration with no way of signing in that is supported, no SFTP),
    /// `TimedOut`, `ConnectionRefused`, `NotConnected` (unreachable), `PermissionDenied` (no way of signing in worked, the key was
    /// not trusted, or `ssh_config` forbids the question that was needed), `QuotaExceeded` (too many sites).
    pub fn connect(
        &self,
        target: &ConnectTarget,
        secret: Option<Secret>,
        prompt: Arc<dyn HostKeyPrompt>,
    ) -> io::Result<ConnectOutcome> {
        let shared = &self.shared;

        shared.check_up()?;

        let plan = plan(&shared.config, target)?;

        let key = SiteKey::of(&plan.target);
        let terms = Terms::of(&plan);

        if let Some(site) = shared.site_for(&key) {
            site.accepts(&terms)?;
            site.forgive();
            shared.questions().forget(&key);

            return Ok(site.outcome());
        }

        shared.check_answer(&key, secret.as_ref().map(|secret| secret.kind))?;

        if shared.sites().len() >= MAX_SITES {
            return Err(too_many());
        }

        let policy = Policy::Known {
            hosts: KnownHosts::load_all(&plan.known_hosts)?,
            path: plan.known_hosts.writable().map(Path::to_owned),
            hash: plan.hash_known_hosts,
            new_key: plan.new_key,
            prompt,
            accepted: Arc::clone(&shared.accepted),
        };
        let owned = Arc::clone(shared);
        let outcome = call(&shared.runtime.handle(), async move {
            owned.open_site(plan, terms, Secrets::of(secret), policy).await
        });

        shared.remember(key, outcome.as_ref().ok());

        outcome
    }

    /// For tests that give a secret without going through the question first: takes it as the answer to a question for
    /// `kind` about `target`, unless that server has a question already.
    #[cfg(any(test, feature = "test-server"))]
    pub fn assume_asked(&self, target: &ConnectTarget, kind: SecretKind) {
        if let Ok(plan) = plan(&self.shared.config, target) {
            let key = SiteKey::of(&plan.target);
            let mut questions = self.shared.questions();

            if questions.get(&key).is_none() {
                questions.ask(
                    key,
                    Question {
                        secret: kind,
                        other: None,
                    },
                );
            }
        }
    }

    /// The calls a pane's listing makes on a connected site. `NotConnected` when the site is not (or no longer) there.
    pub fn fs(&self, site: SiteId) -> io::Result<Arc<dyn RemoteFs>> {
        self.shared
            .sites()
            .get(&site)
            .map(|site| Arc::new(SftpFs::new(Arc::clone(site))) as Arc<dyn RemoteFs>)
            .ok_or_else(closed)
    }

    /// Closes a site. Does not wait for the server: calls that are running on it end with `NotConnected`.
    pub fn disconnect(&self, site: SiteId) {
        let removed = self.shared.sites().remove(&site);

        if let Some(site) = removed {
            self.shared.runtime.handle().spawn(async move { site.close().await });
        }
    }

    /// The hosts `~/.ssh/config` names (plain `Host` names, as typed), for suggestions. Empty when there is no file, and
    /// when there is no OpenSSH (which is what reads the file when it is connected to: a host is not suggested that
    /// cannot be connected to).
    pub fn hosts(&self) -> Vec<SshHost> {
        if ssh_program(&self.shared.config).is_none() {
            return Vec::new();
        }

        suggestions::hosts(&self.shared.config.home)
    }

    /// Closes every connection and stops the runtime, in about a second at the most. Calls after it fail with
    /// `NotConnected`. Safe to call twice.
    pub fn shut_down(&self) {
        if self.shared.down.swap(true, Ordering::AcqRel) {
            return;
        }

        let sites: Vec<Arc<Site>> = self.shared.sites().drain().map(|(_, site)| site).collect();

        // Their goodbyes go out together; a server that does not answer costs nothing past the limit.
        let _ = call(&self.shared.runtime.handle(), async move {
            let mut closing = JoinSet::new();

            for site in sites {
                closing.spawn(async move { site.close().await });
            }

            let _ = tokio::time::timeout(GOODBYES, async { while closing.join_next().await.is_some() {} }).await;

            Ok(())
        });

        self.shared.runtime.shut_down(STOP);
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.shut_down();
    }
}

impl Shared {
    fn check_up(&self) -> io::Result<()> {
        if self.down.load(Ordering::Acquire) {
            Err(closed())
        } else {
            Ok(())
        }
    }

    fn sites(&self) -> MutexGuard<'_, HashMap<SiteId, Arc<Site>>> {
        self.sites.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn questions(&self) -> MutexGuard<'_, Questions<SiteKey>> {
        self.questions.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// An answer (of `kind`) is only taken for the question that was asked about this server.
    fn check_answer(&self, key: &SiteKey, kind: Option<SecretKind>) -> io::Result<()> {
        let Some(kind) = kind else {
            return Ok(());
        };

        match self.questions().get(key) {
            Some(asked) if asked.takes(kind) => Ok(()),
            _ => Err(fail(
                io::ErrorKind::InvalidInput,
                "the answer does not match the question",
            )),
        }
    }

    /// Keeps what `outcome` asked about the server; a question is only forgotten when the server connected. A failed
    /// attempt (a timeout, a lost connection, a refused one) leaves it, so that the same answer can be given again.
    fn remember(&self, key: SiteKey, outcome: Option<&ConnectOutcome>) {
        let mut questions = self.questions();

        match outcome {
            Some(&ConnectOutcome::SecretNeeded { secret, other, .. }) => {
                questions.ask(key, Question { secret, other });
            }
            Some(ConnectOutcome::Connected { .. }) => questions.forget(&key),
            None => {}
        }
    }

    fn site_for(&self, key: &SiteKey) -> Option<Arc<Site>> {
        self.sites().values().find(|site| site.key == *key).cloned()
    }

    async fn open_site(
        &self,
        plan: Plan,
        terms: Terms,
        secrets: Secrets,
        policy: Policy,
    ) -> io::Result<ConnectOutcome> {
        let _turn = self.connecting.acquire().await.map_err(|_| closed())?;

        let (link, home) = match link::open(&self.config, &plan.target, policy, &secrets).await? {
            Opened::Secret {
                kind,
                wrong,
                other,
                reason,
            } => {
                return Ok(ConnectOutcome::SecretNeeded {
                    secret: kind,
                    wrong,
                    other,
                    reason,
                });
            }
            Opened::Open { link, home } => (link, home),
        };
        let home = RemotePath::new(&home).map_or_else(|_| "/".to_owned(), |path| path.to_string());
        let site = Arc::new(Site::new(
            Fresh {
                id: SiteId(self.next_site.fetch_add(1, Ordering::Relaxed)),
                label: plan.label,
                home,
                target: plan.target,
                secrets,
                terms,
                link,
            },
            Arc::clone(&self.config),
            self.runtime.handle(),
        ));

        // Another connect to the same server may have finished first, and the pool may have filled meanwhile.
        let existing = {
            let mut sites = self.sites();

            match sites.values().find(|other| other.key == site.key).cloned() {
                Some(other) => Some(Ok(other)),
                None if sites.len() >= MAX_SITES => Some(Err(too_many())),
                None => {
                    sites.insert(site.id, Arc::clone(&site));

                    None
                }
            }
        };

        match existing {
            None => Ok(site.outcome()),
            Some(found) => {
                site.close().await;

                found.and_then(|other| {
                    other.accepts(&site.terms)?;

                    Ok(other.outcome())
                })
            }
        }
    }
}

fn too_many() -> io::Error {
    fail(
        io::ErrorKind::QuotaExceeded,
        "too many servers are connected; disconnect one first",
    )
}

/// Closes the connection of a site that is not used for a while. Ends when the pool is gone.
async fn close_idle(shared: Weak<Shared>) {
    loop {
        let Some(pool) = shared.upgrade() else {
            return;
        };
        let idle = pool.config.timeouts.idle;
        let sites: Vec<Arc<Site>> = pool.sites().values().cloned().collect();

        drop(pool);

        for site in sites {
            site.close_if_idle(idle).await;
        }

        tokio::time::sleep((idle / 4).clamp(Duration::from_millis(50), Duration::from_secs(30))).await;
    }
}
