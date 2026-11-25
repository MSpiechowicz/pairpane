//! What the running service shares between its tasks and the calls of the app: the endpoint, the device list, the active
//! pairing code and what is known about where devices can be reached.

use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use iroh::{Endpoint, EndpointAddr, EndpointId, TransportAddr};
use tokio::sync::{Semaphore, broadcast};

use super::discovery::FoundEntry;
use crate::devices::{Device, DeviceStore};
use crate::pairing::PairingHost;

/// The most devices whose addresses are remembered, and the most devices mDNS finds that are listed.
pub const MAX_REMEMBERED: usize = 256;

/// Something that happened that the app may want to show.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinkEvent {
    /// A device paired with this one, using a code this device showed.
    Paired(Device),
}

pub struct Ctx {
    pub endpoint: Endpoint,
    pub name: String,
    pub devices: Mutex<DeviceStore>,
    pub host: Arc<PairingHost>,
    pub events: broadcast::Sender<LinkEvent>,
    /// Connections that are being set up or served, so a flood of them cannot use up the process.
    pub connections: Arc<Semaphore>,
    addrs: Mutex<HashMap<EndpointId, BTreeSet<SocketAddr>>>,
    pub found: Mutex<HashMap<EndpointId, FoundEntry>>,
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
            addrs: Mutex::default(),
            found: Mutex::default(),
        }
    }

    pub fn devices(&self) -> MutexGuard<'_, DeviceStore> {
        lock(&self.devices)
    }

    pub fn is_paired(&self, node_id: &EndpointId) -> bool {
        self.devices().is_paired(node_id)
    }

    /// Remembers addresses a device was reached at, for the next time it is dialled. A paired device's addresses are
    /// kept in the device list, newest first, so they last across a restart; the file is written only when they are
    /// new. Other devices (one that is being paired) are kept in memory only, and only so many of them that the
    /// table stays small.
    pub fn remember(&self, node_id: EndpointId, addrs: impl IntoIterator<Item = SocketAddr>) {
        let addrs: Vec<SocketAddr> = addrs.into_iter().collect();
        if self.is_paired(&node_id) {
            self.keep(&node_id, &addrs);
            return;
        }

        let mut known = lock(&self.addrs);
        if !known.contains_key(&node_id) && known.len() >= MAX_REMEMBERED {
            return;
        }

        known.entry(node_id).or_default().extend(addrs);
    }

    /// Keeps addresses a paired device was reached at in the device list; a device that is not paired is ignored.
    pub fn keep(&self, node_id: &EndpointId, addrs: &[SocketAddr]) {
        if let Err(error) = self.devices().remember_addrs(node_id, addrs) {
            tracing::warn!(%error, "the addresses of a device could not be saved");
        }
    }

    /// Where to try to reach a device: the addresses kept in the device list, what was remembered since and what mDNS
    /// found. Empty is fine; iroh may still find it through the address lookup.
    pub fn dial(&self, node_id: EndpointId) -> EndpointAddr {
        let mut addrs: BTreeSet<SocketAddr> = self
            .devices()
            .get(&node_id)
            .map(|device| device.addrs.iter().copied().collect())
            .unwrap_or_default();
        addrs.extend(lock(&self.addrs).get(&node_id).into_iter().flatten());
        if let Some(found) = lock(&self.found).get(&node_id) {
            addrs.extend(&found.addrs);
        }

        EndpointAddr::from_parts(node_id, addrs.into_iter().map(TransportAddr::Ip))
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
