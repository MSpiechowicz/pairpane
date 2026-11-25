//! The device that shows a code: the one active code, how long it lives, how many wrong tries it survives, and which
//! attempt is using it now.
//!
//! A code lives for [`CODE_TTL`], is used up by the first pairing that succeeds, is cancelled when [`MAX_FAILURES`]
//! attempts in total have failed, and only one attempt runs at a time. An attempt that gets as far as the key exchange
//! and does not succeed counts as a failure, also one that is dropped half way, so a peer cannot get more tries. An
//! attempt that never sent anything is [released](Attempt::release) without counting: a connection that only sits
//! there must not use up the tries of the user who shows the code.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::code::PairingCode;
use crate::error::Result;

/// How long a code can be used.
pub const CODE_TTL: Duration = Duration::from_secs(120);
/// How many failed attempts cancel the code.
pub const MAX_FAILURES: u8 = 3;

/// Why a code is no longer active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingEndReason {
    /// A device paired with it.
    Used,
    /// Its time ran out.
    Expired,
    /// It was cancelled, or replaced by a new code.
    Cancelled,
    /// [`MAX_FAILURES`] attempts failed.
    TooManyAttempts,
}

/// A change of the active code, for whoever watches it (see [`PairingHost::observe`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// A code became active.
    Began,
    /// The active code ended.
    Ended(PairingEndReason),
}

type Observer = Box<dyn Fn(Change) + Send + Sync>;

/// What the user is shown for a new code.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingOffer {
    /// The code as `XXXX-XXXX`.
    pub code: String,
    /// How long the code can still be used.
    pub expires_in: Duration,
    /// When the code expires, in milliseconds since the Unix epoch.
    pub expires_at_ms: u64,
}

/// The code is secret until it is used, so it is not in the debug output.
impl fmt::Debug for PairingOffer {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("PairingOffer")
            .field("code", &"..")
            .field("expires_in", &self.expires_in)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

/// Whether a code is waiting to be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingState {
    /// No code is active: none was made, or it was used, cancelled, expired or failed three times.
    Idle,
    /// A code is active.
    Waiting {
        /// How long it can still be used.
        expires_in: Duration,
        /// How many more failed attempts it survives.
        attempts_left: u8,
    },
}

struct Active {
    id: u64,
    code: PairingCode,
    expires: Instant,
    failures: u8,
    busy: bool,
}

#[derive(Default)]
struct State {
    active: Option<Active>,
    codes_made: u64,
}

/// The one active pairing code of a device.
pub struct PairingHost {
    ttl: Duration,
    state: Mutex<State>,
    observer: OnceLock<Observer>,
}

impl PairingHost {
    /// A host whose codes live for `ttl`.
    pub fn new(ttl: Duration) -> Arc<Self> {
        Arc::new(Self {
            ttl,
            state: Mutex::new(State::default()),
            observer: OnceLock::new(),
        })
    }

    /// Sets the function that is told when a code becomes active and when it ends, from whatever thread caused it and
    /// never while the host's state is locked. It can be set once.
    pub fn observe(&self, observer: impl Fn(Change) + Send + Sync + 'static) {
        let _ = self.observer.set(Box::new(observer));
    }

    /// Makes a new code. A code that is active is replaced (and ends as cancelled); an attempt that is using it ends as
    /// a failure of the old code, which no longer exists.
    pub fn begin(&self) -> Result<PairingOffer> {
        let code = PairingCode::generate()?;
        let mut state = self.lock();

        let replaced = state.active.is_some();
        state.codes_made += 1;
        state.active = Some(Active {
            id: state.codes_made,
            code: code.clone(),
            expires: Instant::now() + self.ttl,
            failures: 0,
            busy: false,
        });
        drop(state);

        if replaced {
            self.notify(Change::Ended(PairingEndReason::Cancelled));
        }
        self.notify(Change::Began);

        let expires_at = SystemTime::now() + self.ttl;
        Ok(PairingOffer {
            code: code.to_string(),
            expires_in: self.ttl,
            expires_at_ms: expires_at
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_millis() as u64),
        })
    }

    /// Cancels the active code, if there is one.
    pub fn cancel(&self) {
        let ended = self.lock().active.take().is_some();
        if ended {
            self.notify(Change::Ended(PairingEndReason::Cancelled));
        }
    }

    /// Ends the active code if its time has run out; the observer is told. A timer calls this when the code is due.
    pub fn check_expiry(&self) {
        let mut state = self.lock();
        let expired = Self::drop_if_expired(&mut state);
        drop(state);

        if expired {
            self.notify(Change::Ended(PairingEndReason::Expired));
        }
    }

    /// Whether a code is waiting to be used.
    pub fn state(&self) -> PairingState {
        self.check_expiry();

        match &self.lock().active {
            Some(active) => PairingState::Waiting {
                expires_in: active.expires.saturating_duration_since(Instant::now()),
                attempts_left: MAX_FAILURES - active.failures,
            },
            None => PairingState::Idle,
        }
    }

    /// Starts an attempt on the active code, or `None` when there is no code, it has expired, or another attempt is
    /// running.
    pub fn start_attempt(self: &Arc<Self>) -> Option<Attempt> {
        self.check_expiry();

        let mut state = self.lock();
        let active = state.active.as_mut().filter(|active| !active.busy)?;
        active.busy = true;

        Some(Attempt {
            host: Arc::clone(self),
            id: active.id,
            code: active.code.clone(),
            ended: false,
        })
    }

    /// Drops the active code when it has expired; whether it did.
    fn drop_if_expired(state: &mut State) -> bool {
        let expired = state
            .active
            .as_ref()
            .is_some_and(|active| Instant::now() >= active.expires);
        if expired {
            state.active = None;
        }

        expired
    }

    fn notify(&self, change: Change) {
        if let Some(observer) = self.observer.get() {
            observer(change);
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// One try to pair with the active code. It ends as a failure unless [`Attempt::succeed`] is called.
pub struct Attempt {
    host: Arc<PairingHost>,
    id: u64,
    code: PairingCode,
    ended: bool,
}

impl Attempt {
    /// The code the attempt has to prove.
    pub fn code(&self) -> &PairingCode {
        &self.code
    }

    /// Ends the attempt as a success and uses the code up. This is `false` when the code is not valid any more (it
    /// expired while the attempt ran, or was cancelled or replaced), and then the pairing must not be kept.
    pub fn succeed(mut self) -> bool {
        self.ended = true;

        let mut state = self.host.lock();
        let expired = PairingHost::drop_if_expired(&mut state);

        let used = match &state.active {
            Some(active) if active.id == self.id => {
                state.active = None;
                true
            }
            _ => false,
        };
        drop(state);

        if used {
            self.host.notify(Change::Ended(PairingEndReason::Used));
        } else if expired {
            self.host.notify(Change::Ended(PairingEndReason::Expired));
        }

        used
    }

    /// Ends the attempt as a failure.
    pub fn fail(mut self) {
        self.record_failure();
    }

    /// Ends an attempt that never sent anything. It does not count against the code, and the next attempt can start.
    pub fn release(mut self) {
        self.ended = true;

        let mut state = self.host.lock();
        if let Some(active) = state.active.as_mut().filter(|active| active.id == self.id) {
            active.busy = false;
        }
    }

    fn record_failure(&mut self) {
        if self.ended {
            return;
        }
        self.ended = true;

        let mut state = self.host.lock();
        let Some(active) = state.active.as_mut().filter(|active| active.id == self.id) else {
            return;
        };

        active.busy = false;
        active.failures += 1;

        let cancelled = active.failures >= MAX_FAILURES;
        if cancelled {
            state.active = None;
        }
        drop(state);

        if cancelled {
            self.host.notify(Change::Ended(PairingEndReason::TooManyAttempts));
        }
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        self.record_failure();
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
