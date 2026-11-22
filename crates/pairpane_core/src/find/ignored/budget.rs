//! The share of memory the ignore files of a search may hold, and the turns to compile the big ones.

use std::sync::{Arc, Condvar, Mutex, PoisonError};

use super::{HELD_BYTES, HELD_LINES};

/// How many threads compile big ignore files at once: the rules take far more memory than the text while they are built.
pub(super) const COMPILING: usize = 4;

/// The lines and bytes of ignore files held by the rules that are alive: those remembered and those a lookup still uses.
#[derive(Default)]
pub(super) struct Budget {
    used: Mutex<Used>,
}

#[derive(Default)]
struct Used {
    lines: usize,
    bytes: usize,
    /// The most that was held at once.
    #[cfg(test)]
    peak: (usize, usize),
}

/// A share of the [`Budget`], given back when the rules it was taken for are dropped.
pub(super) struct Charge {
    budget: Arc<Budget>,
    pub(super) lines: usize,
    pub(super) bytes: usize,
}

impl Budget {
    /// A share for a file of `lines` and `bytes`; when the budget has no room for it, how much more room it takes
    /// (lines, bytes), worked out under the same lock.
    pub(super) fn take(self: &Arc<Self>, lines: usize, bytes: usize) -> Result<Charge, (usize, usize)> {
        let mut used = self.used.lock().unwrap_or_else(PoisonError::into_inner);

        let more = (
            (used.lines + lines).saturating_sub(HELD_LINES),
            (used.bytes + bytes).saturating_sub(HELD_BYTES),
        );
        if more != (0, 0) {
            return Err(more);
        }

        used.lines += lines;
        used.bytes += bytes;
        #[cfg(test)]
        {
            used.peak = (used.peak.0.max(used.lines), used.peak.1.max(used.bytes));
        }

        Ok(Charge {
            budget: Arc::clone(self),
            lines,
            bytes,
        })
    }
}

impl Budget {
    /// The most lines and bytes of ignore files that were held at once.
    #[cfg(test)]
    pub(super) fn peak(&self) -> (usize, usize) {
        self.used.lock().unwrap_or_else(PoisonError::into_inner).peak
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        let mut used = self.budget.used.lock().unwrap_or_else(PoisonError::into_inner);

        used.lines -= self.lines;
        used.bytes -= self.bytes;
    }
}

/// A turn to compile a big ignore file; at most [`COMPILING`] are given out.
pub(super) struct Turns {
    free: Mutex<usize>,
    wake: Condvar,
}

pub(super) struct Turn<'a>(&'a Turns);

impl Turns {
    pub(super) fn new() -> Self {
        Self {
            free: Mutex::new(COMPILING),
            wake: Condvar::new(),
        }
    }

    pub(super) fn wait(&self) -> Turn<'_> {
        let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);

        while *free == 0 {
            free = self.wake.wait(free).unwrap_or_else(PoisonError::into_inner);
        }
        *free -= 1;

        Turn(self)
    }
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        *self.0.free.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        self.0.wake.notify_one();
    }
}
