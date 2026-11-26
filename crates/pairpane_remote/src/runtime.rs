//! The async runtime this crate owns, and the blocking facade over it.
//!
//! The app's threads are plain threads (the listing engine's, Tauri's blocking pool); none of them is a tokio worker.
//! [`call`] runs a future on the crate's own runtime and waits for it on the calling thread, so the callers stay
//! synchronous and the runtime stays two threads.

use std::future::Future;
use std::io;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tokio::runtime::{Builder, Handle, Runtime};

use crate::error::{closed, fail, timed_out};

/// Worker threads of the runtime: the connections' I/O only; nothing here is CPU bound.
const WORKERS: usize = 2;

/// The threads that run a prompt or a file write for a connection (`spawn_blocking`).
const BLOCKING_THREADS: usize = 4;

/// How often [`within`] looks at the clock.
const TICK: Duration = Duration::from_millis(50);

/// The runtime of a [`Pool`](crate::Pool); closed by [`shut_down`](Self::shut_down) or when dropped.
pub(crate) struct Owned {
    runtime: Mutex<Option<Runtime>>,
    handle: Handle,
}

impl Owned {
    pub(crate) fn new() -> io::Result<Self> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(WORKERS)
            .max_blocking_threads(BLOCKING_THREADS)
            .thread_name("pairpane-remote")
            .enable_all()
            .build()
            .map_err(|_| fail(io::ErrorKind::Other, "the remote service could not start"))?;
        let handle = runtime.handle().clone();

        Ok(Self {
            runtime: Mutex::new(Some(runtime)),
            handle,
        })
    }

    /// A handle that starts work on the runtime; after [`shut_down`](Self::shut_down) work started with it ends at once.
    pub(crate) fn handle(&self) -> Handle {
        self.handle.clone()
    }

    /// Stops the runtime, giving what still runs `wait` to finish. Idempotent.
    pub(crate) fn shut_down(&self, wait: Duration) {
        let runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();

        if let Some(runtime) = runtime {
            runtime.shutdown_timeout(wait);
        }
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        self.shut_down(Duration::ZERO);
    }
}

/// Runs `work` on the runtime and waits for its result on this thread. Every future given here must end by itself (each
/// step of it carries its own limit); the call ends with an error when the runtime is shut down meanwhile.
///
/// It blocks, so it must not be called from a thread of the runtime itself.
pub(crate) fn call<T, F>(handle: &Handle, work: F) -> io::Result<T>
where
    T: Send + 'static,
    F: Future<Output = io::Result<T>> + Send + 'static,
{
    let (send, receive) = mpsc::sync_channel(1);

    handle.spawn(async move {
        // The caller is gone only when it stopped waiting; nothing is left to tell it.
        let _ = send.send(work.await);
    });

    receive.recv().unwrap_or_else(|_| Err(closed()))
}

/// `work` with a limit of `limit`; `TimedOut` when it takes longer.
pub(crate) async fn limited<T>(limit: Duration, work: impl Future<Output = io::Result<T>>) -> io::Result<T> {
    tokio::time::timeout(limit, work)
        .await
        .unwrap_or_else(|_| Err(timed_out()))
}

/// [`limited`], where the time that `paused` is set does not count: a user's answer to a question can take as long as
/// it takes.
pub(crate) async fn within<T>(
    limit: Duration,
    paused: &AtomicBool,
    work: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    tokio::pin!(work);

    let mut spent = Duration::ZERO;
    let mut last = Instant::now();

    loop {
        tokio::select! {
            result = &mut work => return result,
            () = tokio::time::sleep(TICK) => {
                let now = Instant::now();

                if !paused.load(Ordering::Acquire) {
                    spent += now - last;
                }

                last = now;

                if spent >= limit {
                    return Err(timed_out());
                }
            }
        }
    }
}
