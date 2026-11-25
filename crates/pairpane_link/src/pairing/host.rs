//! The device that shows a code: the one active code, how long it lives, how many wrong tries it survives, and which
//! attempt is using it now.
//!
//! A code lives for [`CODE_TTL`], is used up by the first pairing that succeeds, is cancelled when [`MAX_FAILURES`]
//! attempts in total have failed, and only one attempt runs at a time. Anything that ends an attempt without success
//! counts as a failure, also an attempt that is dropped half way, so a peer that connects and goes quiet cannot keep a
//! code alive or get more tries.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::code::PairingCode;
use crate::error::Result;

/// How long a code can be used.
pub const CODE_TTL: Duration = Duration::from_secs(120);
/// How many failed attempts cancel the code.
pub const MAX_FAILURES: u8 = 3;

/// What the user is shown for a new code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingOffer {
    /// The code as `XXXX-XXXX`.
    pub code: String,
    /// How long the code can still be used.
    pub expires_in: Duration,
    /// When the code expires, in milliseconds since the Unix epoch.
    pub expires_at_ms: u64,
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
}

impl PairingHost {
    /// A host whose codes live for `ttl`.
    pub fn new(ttl: Duration) -> Arc<Self> {
        Arc::new(Self {
            ttl,
            state: Mutex::new(State::default()),
        })
    }

    /// Makes a new code. A code that is active is replaced; an attempt that is using it ends as a failure of the old
    /// code, which no longer exists.
    pub fn begin(&self) -> Result<PairingOffer> {
        let code = PairingCode::generate()?;
        let mut state = self.lock();

        state.codes_made += 1;
        state.active = Some(Active {
            id: state.codes_made,
            code: code.clone(),
            expires: Instant::now() + self.ttl,
            failures: 0,
            busy: false,
        });

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
        self.lock().active = None;
    }

    /// Whether a code is waiting to be used.
    pub fn state(&self) -> PairingState {
        let mut state = self.lock();
        Self::drop_if_expired(&mut state);

        match &state.active {
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
        let mut state = self.lock();
        Self::drop_if_expired(&mut state);

        let active = state.active.as_mut().filter(|active| !active.busy)?;
        active.busy = true;

        Some(Attempt {
            host: Arc::clone(self),
            id: active.id,
            code: active.code.clone(),
            ended: false,
        })
    }

    fn drop_if_expired(state: &mut State) {
        if state
            .active
            .as_ref()
            .is_some_and(|active| Instant::now() >= active.expires)
        {
            state.active = None;
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
        PairingHost::drop_if_expired(&mut state);

        match &state.active {
            Some(active) if active.id == self.id => {
                state.active = None;
                true
            }
            _ => false,
        }
    }

    /// Ends the attempt as a failure.
    pub fn fail(mut self) {
        self.record_failure();
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

        if active.failures >= MAX_FAILURES {
            state.active = None;
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
