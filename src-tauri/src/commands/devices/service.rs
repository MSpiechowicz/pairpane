//! The device link of the app: it is started when the app has paired devices to answer (at launch) or when the Devices
//! screen is first opened, never before, so a fresh install does not listen on the network. It forwards the link's
//! events to the one channel the screen registered, and keeps whether a device is paired where the main thread can read
//! it without waiting for the device files.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use pairpane_core::model::{DevicesEvent, IpcErrorCode};
use pairpane_link::{DeviceStore, Error, KeyStores, Link, LinkConfig, LinkEvent, default_stores};
use tauri::async_runtime::{self, JoinHandle};
use tauri::ipc::Channel;
use tokio::sync::broadcast::error::RecvError;

use super::convert;
use crate::error::{CommandError, CommandResult};

/// What the screen registered: the channel, and the session of the open that made it (0 before any).
#[derive(Default)]
pub(super) struct Subscription {
    session: u64,
    channel: Option<Channel<DevicesEvent>>,
}

/// The latest channel the screen registered.
type Listener = Arc<Mutex<Subscription>>;

/// Called when the last paired device is gone.
type EmptyHook = Arc<dyn Fn() + Send + Sync>;

/// What the link's forwarder shares with the service.
#[derive(Clone)]
pub(super) struct Shared {
    pub(super) listener: Listener,
    /// Whether a device is paired, as of the last time the list may have changed.
    paired: Arc<AtomicBool>,
    on_empty: EmptyHook,
}

impl Shared {
    fn new(on_empty: EmptyHook) -> Self {
        Self {
            listener: Listener::default(),
            paired: Arc::new(AtomicBool::new(false)),
            on_empty,
        }
    }

    /// Reads the paired devices and remembers whether there are any; their number, when the list can be read.
    fn refresh(&self, link: &Link) -> Option<usize> {
        let count = link.devices().ok()?.len();
        self.paired.store(count > 0, Ordering::Release);

        Some(count)
    }
}

/// Whether `event`, with `remaining` devices paired after it, leaves the app with no device: an unpairing or a new list.
pub(super) fn left_empty(event: &DevicesEvent, remaining: Option<usize>) -> bool {
    matches!(event, DevicesEvent::Unpaired { .. } | DevicesEvent::Resync { .. }) && remaining == Some(0)
}

/// How the link is configured for a data folder and a device name.
type Setup = fn(&Path, &str) -> (LinkConfig, KeyStores);

/// Why the link did not start.
#[derive(Debug)]
pub enum StartError {
    /// The app has no data folder to keep the devices in.
    NoFolder,
    /// The app is quitting.
    Stopped,
    Link(Error),
}

impl StartError {
    /// A fixed word for the log.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NoFolder => "no data folder",
            Self::Stopped => "stopped",
            Self::Link(error) => convert::error_kind(error),
        }
    }

    pub fn into_command_error(self) -> CommandError {
        match self {
            Self::NoFolder => CommandError::new(
                IpcErrorCode::Unsupported,
                "Pairpane has no data folder to keep the paired devices in.",
            ),
            Self::Stopped => CommandError::new(IpcErrorCode::Internal, "The device link is not running."),
            Self::Link(error) => convert::command_error(&error),
        }
    }
}

/// The link, once it runs, and the channel its events go to.
pub struct LinkService {
    dir: Option<PathBuf>,
    name: String,
    setup: Setup,
    link: Mutex<Option<Arc<Link>>>,
    /// Held while the link starts, so two callers do not both start one.
    start: tokio::sync::Mutex<()>,
    stopped: AtomicBool,
    pub(super) shared: Shared,
    forwarder: Mutex<Option<JoinHandle<()>>>,
}

/// Whether the folder lists a paired device. It opens nothing for writing and creates nothing, so a fresh install, or a
/// list that cannot be read, is `false`.
pub fn paired_on_disk(dir: &Path) -> bool {
    DeviceStore::open(dir).is_ok_and(|store| !store.list().is_empty())
}

fn production(dir: &Path, name: &str) -> (LinkConfig, KeyStores) {
    (LinkConfig::new(dir, name), default_stores(dir))
}

impl LinkService {
    /// A service that keeps its files in `dir` (none: there is no data folder, so the link never starts) and announces
    /// itself as `name`; `on_empty` is called (from the async runtime) when the last paired device is gone, so the
    /// window that hides in the tray can be shown again. Nothing starts yet.
    pub fn new(dir: Option<PathBuf>, name: String, on_empty: EmptyHook) -> Self {
        Self::with_hook(dir, name, production, on_empty)
    }

    #[cfg(test)]
    pub fn with_setup(dir: Option<PathBuf>, name: String, setup: Setup) -> Self {
        Self::with_hook(dir, name, setup, Arc::new(|| {}))
    }

    pub(super) fn with_hook(dir: Option<PathBuf>, name: String, setup: Setup, on_empty: EmptyHook) -> Self {
        Self {
            dir,
            name,
            setup,
            link: Mutex::new(None),
            start: tokio::sync::Mutex::new(()),
            stopped: AtomicBool::new(false),
            shared: Shared::new(on_empty),
            forwarder: Mutex::new(None),
        }
    }

    /// The running link, if it has started.
    pub fn running(&self) -> Option<Arc<Link>> {
        lock(&self.link).clone()
    }

    /// The running link, or the error the commands answer when it has not started.
    pub fn link(&self) -> CommandResult<Arc<Link>> {
        self.running()
            .ok_or_else(|| CommandError::new(IpcErrorCode::Internal, "The device link is not running."))
    }

    /// Whether the app is to stay alive in the tray when its window closes: the link runs and a device is paired. It
    /// reads what was remembered, never the device files, so the main thread does not wait for them.
    pub fn keeps_app_alive(&self) -> bool {
        self.shared.paired.load(Ordering::Acquire)
    }

    /// Remembers whether a device is paired, after a command changed the list.
    pub fn refresh_paired(&self, link: &Link) {
        self.shared.refresh(link);
    }

    /// The running link, started first when it is not. It needs the async runtime (the pairing timer is a task of it).
    pub async fn ensure_started(&self) -> Result<Arc<Link>, StartError> {
        if let Some(link) = self.running() {
            return Ok(link);
        }

        let _starting = self.start.lock().await;
        if let Some(link) = self.running() {
            return Ok(link);
        }
        if self.stopped.load(Ordering::Acquire) {
            return Err(StartError::Stopped);
        }

        let dir = self.dir.as_deref().ok_or(StartError::NoFolder)?;
        let (config, stores) = (self.setup)(dir, &self.name);
        let link = Arc::new(Link::start(config, stores).await.map_err(StartError::Link)?);

        self.shared.refresh(&link);
        if let Ok(events) = link.subscribe() {
            let task = async_runtime::spawn(forward(events, Arc::downgrade(&link), self.shared.clone()));
            *lock(&self.forwarder) = Some(task);
        }
        *lock(&self.link) = Some(Arc::clone(&link));

        Ok(link)
    }

    /// Makes `channel` the one events go to, replacing the earlier one (after a reload of the page), unless `session` is
    /// older than the one that registered it: an open that was slow does not take the channel from the newer page.
    /// `true` when it was taken.
    pub fn subscribe(&self, channel: Channel<DevicesEvent>, session: u64) -> bool {
        let mut current = lock(&self.shared.listener);
        if session < current.session {
            return false;
        }

        *current = Subscription {
            session,
            channel: Some(channel),
        };

        true
    }

    /// Cancels the pairing code that is offered, if any.
    pub fn cancel_code(&self) {
        if let Some(link) = self.running() {
            // A link that is shut down has no code left.
            link.cancel_pairing().ok();
        }
    }

    /// Stops the link, as the app quits: no event is forwarded and none starts it again.
    pub async fn shutdown(&self) {
        let _starting = self.start.lock().await;
        self.stopped.store(true, Ordering::Release);
        self.shared.paired.store(false, Ordering::Release);

        if let Some(task) = lock(&self.forwarder).take() {
            task.abort();
        }

        let link = lock(&self.link).take();
        if let Some(link) = link {
            link.shutdown().await;
        }
    }
}

/// Tells the screen's channel what the link reports. A receiver that fell behind missed events, so the screen gets the
/// list to replace its own with. When the last device is gone, the hook is called after the screen is told. It ends with
/// the link.
pub(super) async fn forward(mut events: tokio::sync::broadcast::Receiver<LinkEvent>, link: Weak<Link>, shared: Shared) {
    loop {
        let event = match events.recv().await {
            Ok(event) => convert::event(event),
            Err(RecvError::Lagged(_)) => resync(&link),
            Err(RecvError::Closed) => return,
        };

        // The list may have changed with any event, and the one that tells so must find it remembered.
        let remaining = link.upgrade().and_then(|link| shared.refresh(&link));

        if let Some(event) = event {
            let emptied = left_empty(&event, remaining);

            deliver(&shared.listener, event);
            if emptied {
                (shared.on_empty)();
            }
        }
    }
}

/// The event that replaces the list the screen has.
fn resync(link: &Weak<Link>) -> Option<DevicesEvent> {
    let devices = link.upgrade()?.devices().ok()?;

    Some(DevicesEvent::Resync {
        devices: convert::paired_list(&devices),
    })
}

/// Sends `event` to the channel, if the screen registered one.
fn deliver(listener: &Listener, event: DevicesEvent) {
    if let Some(channel) = lock(listener).channel.as_ref() {
        // A closed channel only means the page went away; it registers again when it opens the screen.
        channel.send(event).ok();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
