//! What the running service shares between its tasks and the calls of the app: the endpoint, the device list, the active
//! pairing code and what is known about where devices can be reached.

use std::collections::{BTreeSet, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use iroh::{Endpoint, EndpointAddr, EndpointId, TransportAddr};
use tokio::sync::{Semaphore, broadcast, watch};

use super::discovery::FoundEntry;
use super::local_net::source_key;
use crate::devices::{Device, DeviceStore, MAX_ADDRS, merged};
use crate::error::{Error, Result};
use crate::pairing::{PairingEndReason, PairingHost};

/// The most devices whose addresses are remembered in memory, and the most devices mDNS finds that are listed.
pub const MAX_REMEMBERED: usize = 256;

/// The most connections one source may have at once (one address; see [`source_key`]), of the
/// [`super::MAX_CONNECTIONS`] in all.
pub const MAX_CONNECTIONS_PER_IP: usize = 4;

/// The most probes ([`super::Link::status`]) that run at once.
pub const MAX_PROBES: usize = 8;

/// Something that happened that the app may want to show.
///
/// Events come over a channel that holds a few of them: a receiver that does not keep up gets
/// [`RecvError::Lagged`](tokio::sync::broadcast::error::RecvError::Lagged) with the number it missed, and should read
/// the lists again ([`super::Link::devices`], [`super::Link::pairing_state`]) instead of trusting what it saw.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinkEvent {
    /// A device paired with this one, using a code this device showed.
    Paired(Device),
    /// A device was unpaired from this one by [`super::Link::unpair`].
    Unpaired(EndpointId),
    /// The pairing code is no longer active.
    PairingEnded {
        /// Why.
        reason: PairingEndReason,
    },
}

/// The addresses a device that is not paired yet was reached at, kept in memory.
#[derive(Default)]
struct Remembered {
    addrs: HashMap<EndpointId, (Vec<SocketAddr>, u64)>,
    clock: u64,
}

pub struct Ctx {
    pub endpoint: Endpoint,
    pub name: String,
    pub devices: Mutex<DeviceStore>,
    pub host: Arc<PairingHost>,
    pub events: broadcast::Sender<LinkEvent>,
    /// Connections that are being set up or served, so a flood of them cannot use up the process.
    pub connections: Arc<Semaphore>,
    /// Probes that are running, so a long list of devices does not open a connection to each at once.
    pub probes: Semaphore,
    /// Whether mDNS should announce this device: a pairing code is active.
    pub announcing: watch::Sender<bool>,
    /// Set when the service is shut down; the device list is not written after that, because the lock on the data folder
    /// is let go (see [`Ctx::writer`]).
    pub closed: AtomicBool,
    per_ip: Mutex<HashMap<IpAddr, usize>>,
    /// What is remembered in memory about devices that are not paired. When this is held together with the device list,
    /// it is taken first: a change of who is paired and of what is remembered about them is then one step.
    remembered: Mutex<Remembered>,
    pub found: Mutex<HashMap<EndpointId, FoundEntry>>,
    pub found_clock: AtomicU64,
}

/// A connection counted against its source address; the count goes down when it is dropped.
pub struct IpGuard {
    ctx: Arc<Ctx>,
    ip: IpAddr,
}

impl Drop for IpGuard {
    fn drop(&mut self) {
        let mut per_ip = lock(&self.ctx.per_ip);
        if let Some(count) = per_ip.get_mut(&self.ip) {
            *count -= 1;
            if *count == 0 {
                per_ip.remove(&self.ip);
            }
        }
    }
}

impl Ctx {
    pub fn new(endpoint: Endpoint, name: String, devices: DeviceStore, host: Arc<PairingHost>) -> Self {
        Self {
            endpoint,
            name,
            devices: Mutex::new(devices),
            host,
            events: broadcast::channel(16).0,
            connections: Arc::new(Semaphore::new(super::MAX_CONNECTIONS)),
            probes: Semaphore::new(MAX_PROBES),
            announcing: watch::channel(false).0,
            closed: AtomicBool::new(false),
            per_ip: Mutex::default(),
            remembered: Mutex::default(),
            found: Mutex::default(),
            found_clock: AtomicU64::new(0),
        }
    }

    pub fn devices(&self) -> MutexGuard<'_, DeviceStore> {
        lock(&self.devices)
    }

    /// The device list for a change. The service being shut down is checked under the lock of the list, which shutdown
    /// takes once more before it lets go of the folder, so no change is written after that.
    pub fn writer(&self) -> Result<MutexGuard<'_, DeviceStore>> {
        let devices = self.devices();
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }

        Ok(devices)
    }

    pub fn is_paired(&self, node_id: &EndpointId) -> bool {
        self.devices().is_paired(node_id)
    }

    /// Counts a connection from `ip`, or `None` when its source already has [`MAX_CONNECTIONS_PER_IP`]. A source is its
    /// address (see [`source_key`]).
    pub fn admit(self: &Arc<Self>, ip: IpAddr) -> Option<IpGuard> {
        let ip = source_key(ip);
        let mut per_ip = lock(&self.per_ip);
        let count = per_ip.entry(ip).or_insert(0);
        if *count >= MAX_CONNECTIONS_PER_IP {
            return None;
        }
        *count += 1;

        Some(IpGuard {
            ctx: Arc::clone(self),
            ip,
        })
    }

    /// Pairs a device: it goes in the device list with `addrs` (what is known to be its addresses) and then what was
    /// remembered in memory about it (what the user gave with [`super::Link::remember_addresses`]), which is not
    /// forgotten but now kept. The file is written first, then the memory is changed. This writes a file, so an async
    /// task calls [`Ctx::pin`].
    pub fn add_device(&self, node_id: EndpointId, name: &str, addrs: &[SocketAddr]) -> Result<Device> {
        let mut table = lock(&self.remembered);
        let remembered = table.addrs.get(&node_id).map(|(addrs, _)| addrs.clone());
        let all: Vec<SocketAddr> = addrs.iter().copied().chain(remembered.into_iter().flatten()).collect();

        let device = self.writer()?.add_with_addrs(node_id, name, now_ms(), &all)?;
        table.addrs.remove(&node_id);

        Ok(device)
    }

    /// [`Ctx::add_device`] on a thread meant for blocking work, for the tasks of the runtime.
    pub async fn pin(self: &Arc<Self>, node_id: EndpointId, name: String, addrs: Vec<SocketAddr>) -> Result<Device> {
        let ctx = Arc::clone(self);

        tokio::task::spawn_blocking(move || ctx.add_device(node_id, &name, &addrs))
            .await
            .map_err(|_| Error::Closed)?
    }

    /// Unpairs a device and forgets what is remembered about it, as one step to [`Ctx::remember`].
    pub fn unpair(&self, node_id: &EndpointId) -> Result<()> {
        let mut table = lock(&self.remembered);
        self.writer()?.remove(node_id)?;
        table.addrs.remove(node_id);

        Ok(())
    }

    /// Remembers addresses a device was reached at, for the next time it is dialled. The caller gives only addresses it
    /// has reason to trust: the source of an authenticated connection, or what the user typed. A paired device's go in
    /// the device list, newest first, so they last across a restart; the file is written only when they are new. For a
    /// device that is not paired yet they are kept in memory: the newest [`MAX_ADDRS`] of each, and of the devices the
    /// newest [`MAX_REMEMBERED`], the oldest one making room when the table is full. A write of the file happens on the
    /// calling thread; a task of the runtime calls [`Ctx::remember_in_background`].
    pub fn remember(&self, node_id: EndpointId, addrs: impl IntoIterator<Item = SocketAddr>) {
        if let Some(addrs) = self.place(node_id, addrs.into_iter().collect()) {
            self.keep(&node_id, &addrs);
        }
    }

    /// [`Ctx::remember`] for the tasks of the runtime, done whole on a thread meant for blocking work, which is not
    /// waited for: deciding whether the device is paired takes the device list's lock, which a write of the file holds
    /// while it waits for the disk, so not even that waits on a thread of the runtime. A write is not done at all when
    /// the service has been shut down by then.
    pub fn remember_in_background(self: &Arc<Self>, node_id: EndpointId, addrs: impl IntoIterator<Item = SocketAddr>) {
        let ctx = Arc::clone(self);
        let addrs: Vec<SocketAddr> = addrs.into_iter().collect();
        drop(tokio::task::spawn_blocking(move || ctx.remember(node_id, addrs)));
    }

    /// Notes the addresses of a device that is not paired in the table in memory. A paired device is decided under the
    /// same lock as the table, so an unpairing cannot come between the two; its addresses are given back to be kept in
    /// the device list.
    fn place(&self, node_id: EndpointId, addrs: Vec<SocketAddr>) -> Option<Vec<SocketAddr>> {
        let mut table = lock(&self.remembered);
        if self.is_paired(&node_id) {
            return Some(addrs);
        }

        if !table.addrs.contains_key(&node_id) && table.addrs.len() >= MAX_REMEMBERED {
            let oldest = table
                .addrs
                .iter()
                .min_by_key(|(_, (_, stamp))| *stamp)
                .map(|(id, _)| *id);
            if let Some(oldest) = oldest {
                table.addrs.remove(&oldest);
            }
        }

        table.clock += 1;
        let stamp = table.clock;
        let entry = table.addrs.entry(node_id).or_default();
        if let Some(next) = merged(&entry.0, &addrs) {
            entry.0 = next;
        }
        entry.1 = stamp;
        debug_assert!(entry.0.len() <= MAX_ADDRS);

        None
    }

    /// Whether anything is remembered in memory about a device.
    #[cfg(test)]
    pub fn has_remembered(&self, node_id: &EndpointId) -> bool {
        lock(&self.remembered).addrs.contains_key(node_id)
    }

    /// Keeps addresses a paired device was reached at in the device list; a device that is not paired is ignored.
    fn keep(&self, node_id: &EndpointId, addrs: &[SocketAddr]) {
        let kept = self
            .writer()
            .and_then(|mut devices| devices.remember_addrs(node_id, addrs));
        if let Err(error) = kept
            && !matches!(error, Error::Closed)
        {
            tracing::warn!(%error, "the addresses of a device could not be saved");
        }
    }

    /// Where to try to reach a device: `extra` (what the caller was given), the addresses kept in the device list, what
    /// was remembered since and what mDNS found. mDNS is not authenticated, so its addresses are only tried, never kept.
    /// Empty is fine; iroh may still find the device through the address lookup.
    pub fn dial(&self, node_id: EndpointId, extra: &[SocketAddr]) -> EndpointAddr {
        let mut addrs: BTreeSet<SocketAddr> = extra.iter().copied().collect();
        addrs.extend(
            self.devices()
                .get(&node_id)
                .into_iter()
                .flat_map(|device| device.addrs.iter().copied()),
        );
        addrs.extend(
            lock(&self.remembered)
                .addrs
                .get(&node_id)
                .into_iter()
                .flat_map(|(addrs, _)| addrs.iter().copied()),
        );
        addrs.extend(
            lock(&self.found)
                .get(&node_id)
                .into_iter()
                .flat_map(|found| found.addrs.iter().copied()),
        );

        EndpointAddr::from_parts(node_id, addrs.into_iter().map(TransportAddr::Ip))
    }

    /// The next stamp of an entry of the found list; a higher one is newer.
    pub fn next_found_stamp(&self) -> u64 {
        self.found_clock.fetch_add(1, Ordering::Relaxed)
    }
}

/// A lock whose holder panicked is still usable: every critical section here leaves the data whole.
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The time now in milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}
