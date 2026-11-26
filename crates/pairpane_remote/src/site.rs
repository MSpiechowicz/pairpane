//! One server the app is connected to: its credentials, its connection, and what keeps the connection healthy.
//!
//! The connection is the site's, not the caller's: one that dropped is opened again by the next call, without a question
//! (the same key, and the sign-in the user gave, each secret only in its own role); one that was idle for a while is
//! closed and opened again the same way. A connection that cannot be opened again is not tried again for
//! [`RETRY_AFTER`]: the callers that wait for it, and the ones that come meanwhile, get the same failure at once.
//!
//! A site is who it is connected to (user, host, port; see [`SiteKey`]), not how it was asked for: a second `ssh_config`
//! alias for the same server reuses the first one's connection and its settings (its keys, its `known_hosts` files, its
//! host key check). That is a known limit, made safe in one direction: a connect whose settings are stricter than the
//! site's is refused ([`Site::accepts`]) and not answered with a site that was opened under looser ones.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pairpane_core::model::{ConnectOutcome, SiteId};
use russh::keys::PublicKey;
use tokio::runtime::Handle;

use crate::auth::Secrets;
use crate::config::PoolConfig;
use crate::error::{closed, fail};
use crate::link::{self, Link, Opened, Target};
use crate::terms::Terms;
use crate::verify::Policy;

/// How long a failed reopening is remembered: callers within it get the failure without another attempt.
pub(crate) const RETRY_AFTER: Duration = Duration::from_secs(10);

/// Who a site is: the user, the host (in lower case) and the port. Two requests for the same one share a site.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SiteKey {
    user: String,
    host: String,
    port: u16,
}

impl SiteKey {
    pub(crate) fn of(target: &Target) -> Self {
        Self {
            user: target.user.clone(),
            host: target.host.to_lowercase(),
            port: target.port,
        }
    }
}

pub(crate) struct Site {
    pub(crate) id: SiteId,
    pub(crate) label: String,
    /// The folder the server started in.
    pub(crate) home: String,
    pub(crate) key: SiteKey,
    pub(crate) config: Arc<PoolConfig>,
    pub(crate) runtime: Handle,
    target: Target,
    secrets: Secrets,
    /// What the site was opened under, which a later connect to it may not loosen (see [`Terms`]).
    pub(crate) terms: Terms,
    /// The server's key, which an opened-again connection must present too.
    pinned: PublicKey,
    link: Mutex<Option<Arc<Link>>>,
    /// Held while a connection is opened again, so that callers that find it gone open it once.
    reopening: tokio::sync::Mutex<()>,
    /// The last time opening it again failed, and why.
    failed: Mutex<Option<Failure>>,
    busy: AtomicUsize,
    idle_since: Mutex<Instant>,
    closed: AtomicBool,
}

/// What a site is made of when it is first connected.
pub(crate) struct Fresh {
    pub(crate) id: SiteId,
    pub(crate) label: String,
    pub(crate) home: String,
    pub(crate) target: Target,
    pub(crate) secrets: Secrets,
    pub(crate) terms: Terms,
    pub(crate) link: Arc<Link>,
}

impl Site {
    pub(crate) fn new(fresh: Fresh, config: Arc<PoolConfig>, runtime: Handle) -> Self {
        Self {
            id: fresh.id,
            label: fresh.label,
            home: fresh.home,
            key: SiteKey::of(&fresh.target),
            config,
            runtime,
            pinned: fresh.link.host_key.clone(),
            link: Mutex::new(Some(fresh.link)),
            reopening: tokio::sync::Mutex::new(()),
            failed: Mutex::new(None),
            busy: AtomicUsize::new(0),
            idle_since: Mutex::new(Instant::now()),
            closed: AtomicBool::new(false),
            target: fresh.target,
            secrets: fresh.secrets,
            terms: fresh.terms,
        }
    }

    /// Whether a connect with `terms` may be answered with this site: not when it asks for more than the site was opened
    /// with (a stricter host key check, other `known_hosts` files, fewer ways of signing in, no questions); the settings
    /// of the first connect would be used silently in place of the ones that were asked for.
    pub(crate) fn accepts(&self, terms: &Terms) -> io::Result<()> {
        if terms.stricter_than(&self.terms) {
            return Err(fail(
                io::ErrorKind::InvalidInput,
                "this server is already open under another name with different ssh_config settings; disconnect it first",
            ));
        }

        Ok(())
    }

    pub(crate) fn outcome(&self) -> ConnectOutcome {
        ConnectOutcome::Connected {
            site: self.id,
            label: self.label.clone(),
            home: self.home.clone(),
        }
    }

    /// The open connection, or a new one when there is none: no question is asked, and the server must present the key it
    /// presented before.
    ///
    /// Callers that find it gone share one attempt: the first opens it while the others wait, and when it fails they all
    /// get that failure (so does any caller within [`RETRY_AFTER`]). The attempt has the limits of any connection (see
    /// [`Timeouts`](crate::Timeouts)), so a caller waits for one attempt at the most.
    pub(crate) async fn link(&self) -> io::Result<Arc<Link>> {
        if let Some(link) = self.current()? {
            return Ok(link);
        }

        self.recent_failure()?;

        let _turn = self.reopening.lock().await;

        // Another caller may have opened it, or failed to, while this one waited.
        if let Some(link) = self.current()? {
            return Ok(link);
        }

        self.recent_failure()?;

        match self.reopen().await {
            Ok(link) => {
                *self.failed() = None;

                Ok(link)
            }
            Err(error) => {
                *self.failed() = Some(Failure::of(&error));

                Err(error)
            }
        }
    }

    /// Forgets that opening the connection again failed, so that the next call tries it (the user asked to connect).
    pub(crate) fn forgive(&self) {
        *self.failed() = None;
    }

    async fn reopen(&self) -> io::Result<Arc<Link>> {
        let opened = link::open(
            &self.config,
            &self.target,
            Policy::Pinned(self.pinned.clone()),
            &self.secrets,
        )
        .await?;
        tracing::debug!(site = self.id.0, "a connection to a site was opened again");

        let Opened::Open { link, .. } = opened else {
            return Err(fail(
                io::ErrorKind::PermissionDenied,
                "the server no longer accepts the sign-in",
            ));
        };
        *self.slot() = Some(Arc::clone(&link));

        // Closed while it was opening: nobody will use it.
        if self.closed.load(Ordering::Acquire) {
            self.take_link().close_if_any().await;

            return Err(closed());
        }

        Ok(link)
    }

    /// The failure of an attempt that is too recent to try again, as an error.
    fn recent_failure(&self) -> io::Result<()> {
        match self
            .failed()
            .as_ref()
            .filter(|failure| failure.at.elapsed() < RETRY_AFTER)
        {
            Some(failure) => Err(failure.error()),
            None => Ok(()),
        }
    }

    fn failed(&self) -> std::sync::MutexGuard<'_, Option<Failure>> {
        self.failed.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn current(&self) -> io::Result<Option<Arc<Link>>> {
        if self.closed.load(Ordering::Acquire) {
            return Err(closed());
        }

        Ok(self.slot().as_ref().filter(|link| link.is_open()).cloned())
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Arc<Link>>> {
        self.link.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn take_link(&self) -> Taken {
        Taken(self.slot().take())
    }

    /// Lets go of `dead`, which stopped answering, unless a newer connection has taken its place.
    pub(crate) fn forget(&self, dead: &Arc<Link>) {
        let mut slot = self.slot();

        if slot.as_ref().is_some_and(|link| Arc::ptr_eq(link, dead)) {
            *slot = None;
        }

        drop(slot);

        let dead = Arc::clone(dead);

        self.runtime.spawn(async move { dead.close().await });
    }

    /// Marks the site as in use until the returned guard is dropped.
    pub(crate) fn lease(&self) -> Lease<'_> {
        self.busy.fetch_add(1, Ordering::AcqRel);

        Lease(self)
    }

    /// Closes the connection when the site has not been used for `idle`; it opens again when used.
    pub(crate) async fn close_if_idle(&self, idle: Duration) {
        let unused = self.busy.load(Ordering::Acquire) == 0
            && self
                .idle_since
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .elapsed()
                >= idle;

        if unused {
            tracing::debug!(site = self.id.0, "an idle connection was closed");

            self.take_link().close_if_any().await;
        }
    }

    /// Closes the site for good; calls that come later fail.
    pub(crate) async fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.take_link().close_if_any().await;
    }
}

/// Why opening a connection again failed, and when (the errors of this crate carry a fixed text, which is kept).
struct Failure {
    kind: io::ErrorKind,
    message: String,
    at: Instant,
}

impl Failure {
    fn of(error: &io::Error) -> Self {
        Self {
            kind: error.kind(),
            message: error.to_string(),
            at: Instant::now(),
        }
    }

    fn error(&self) -> io::Error {
        io::Error::new(self.kind, self.message.clone())
    }
}

/// A connection taken out of a site, to be closed.
struct Taken(Option<Arc<Link>>);

impl Taken {
    async fn close_if_any(self) {
        if let Some(link) = self.0 {
            link.close().await;
        }
    }
}

/// A site in use: it is not closed for being idle meanwhile.
pub(crate) struct Lease<'a>(&'a Site);

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        *self
            .0
            .idle_since
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Instant::now();
        self.0.busy.fetch_sub(1, Ordering::AcqRel);
    }
}
