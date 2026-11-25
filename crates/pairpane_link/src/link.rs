//! The running link service: one iroh endpoint that accepts pairings and hellos, finds devices on the local network, and
//! the calls the app makes on it.
//!
//! The endpoint is made without relays and without DNS or pkarr address lookup, so it never contacts a server: devices
//! reach each other by the addresses on their own networks (the local network, or a VPN such as Tailscale), or the ones
//! mDNS finds. It listens on one UDP port; [`DEFAULT_PORT`] is tried first so the address of a device over a VPN stays
//! the same from one start to the next.
//!
//! One service runs on a data folder at a time (a lock file keeps a second one out), and after [`Link::shutdown`] every
//! call returns [`Error::Closed`].

mod accept;
mod bind;
mod config;
mod ctx;
mod discovery;
mod hello;
mod local_net;
mod target;

use std::fs::File;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::{EndpointId, TransportAddr};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::timeout;

pub use config::{LinkConfig, Status, ThisDevice};
pub use ctx::LinkEvent;
pub use discovery::{Discovery, Found};
pub use hello::{HELLO_VERSION, LINK_ALPN};
pub use target::Target;

use ctx::{Ctx, lock};
use hello::Probe;

use crate::code::PairingCode;
use crate::devices::{Device, DeviceStore, validate_name};
use crate::error::{Error, Result};
use crate::fsutil;
use crate::identity::{KeyStores, StoreKind, open_identity};
use crate::pairing::{Change, PAIR_ALPN, PairingHost, PairingOffer, PairingState, SESSION_TIMEOUT, join_session};
use crate::wire::{CLOSE_DONE, CLOSE_REFUSED};

/// The UDP port tried first, so a device keeps its address over a VPN.
pub const DEFAULT_PORT: u16 = 47620;

/// The most connections that are set up or served at once; more are refused.
pub const MAX_CONNECTIONS: usize = 32;

/// How long a connection to a device to pair with may take to be set up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The running service. Keep it for as long as the app runs and call [`Link::shutdown`] when it ends; dropping it stops
/// its tasks and closes the endpoint, but without waiting for the peers.
pub struct Link {
    ctx: Arc<Ctx>,
    key_store: StoreKind,
    /// The lock on the data folder, held until the service is shut down.
    folder_lock: Mutex<Option<File>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    /// The timer that ends the active pairing code when its time is up.
    expiry: Mutex<Option<JoinHandle<()>>>,
}

impl Link {
    /// Starts the service: takes the lock on the data folder ([`Error::AlreadyRunning`] if another service has it),
    /// opens the device list, finds the device key as [`open_identity`] says (the stores may talk to the OS keychain,
    /// which blocks, so that runs on a thread meant for blocking work), binds the endpoint and starts accepting
    /// connections and finding devices.
    pub async fn start(config: LinkConfig, stores: KeyStores) -> Result<Self> {
        let name = validate_name(&config.device_name)?;
        let folder_lock = fsutil::lock_dir(&config.data_dir, fsutil::LOCK_FILE_NAME)?;

        let devices = DeviceStore::open(&config.data_dir)?;
        let has_devices = !devices.list().is_empty();

        let dir = config.data_dir.clone();
        let (key, key_store) = tokio::task::spawn_blocking(move || open_identity(&dir, &stores, has_devices))
            .await
            .map_err(|_| Error::Closed)??;

        let endpoint = bind::bind(&config, key, &name).await?;
        let ctx = Arc::new(Ctx::new(endpoint, name, devices, PairingHost::new(config.pairing_ttl)));
        watch_pairing(&ctx);

        let mut tasks = vec![tokio::spawn(accept::run(Arc::clone(&ctx)))];
        if config.discovery != Discovery::Off {
            tasks.push(tokio::spawn(discovery::run(
                Arc::clone(&ctx),
                config.discovery,
                ctx.announcing.subscribe(),
            )));
        }

        Ok(Self {
            ctx,
            key_store,
            folder_lock: Mutex::new(Some(folder_lock)),
            tasks: Mutex::new(tasks),
            expiry: Mutex::new(None),
        })
    }

    fn open(&self) -> Result<()> {
        if self.ctx.closed.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }

        Ok(())
    }

    /// This device's id, name and addresses.
    pub fn this_device(&self) -> Result<ThisDevice> {
        self.open()?;
        let endpoint = &self.ctx.endpoint;

        Ok(ThisDevice {
            node_id: endpoint.id(),
            name: self.ctx.name.clone(),
            bound: endpoint.bound_sockets(),
            addrs: endpoint.addr().ip_addrs().copied().collect(),
            key_store: self.key_store,
        })
    }

    /// Makes a pairing code and starts accepting pairings with it. A code that is active is replaced. While it is active
    /// the device is announced on the local network (unless discovery is off), so the other device can find it.
    ///
    /// The timer that ends the code is a task of the async runtime, so this must be called where one is running (on
    /// one of its threads, or inside [`tokio::runtime::Handle::enter`]); anywhere else it is [`Error::NoRuntime`], and
    /// no code is made. This is a plain function, not an async one, so a caller that is not async can use it.
    pub fn begin_pairing(&self) -> Result<PairingOffer> {
        self.open()?;
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| Error::NoRuntime)?;
        // The timer's slot is held while the code is made, so two calls at once cannot leave the newer code with the
        // older code's timer (which the newer timer would have replaced): codes and their timers change in one order.
        let mut slot = lock(&self.expiry);
        let offer = self.ctx.host.begin()?;

        let host = Arc::clone(&self.ctx.host);
        let due = offer.expires_in;
        let timer = runtime.spawn(async move {
            tokio::time::sleep(due).await;
            host.check_expiry();
        });
        if let Some(old) = slot.replace(timer) {
            old.abort();
        }

        Ok(offer)
    }

    /// Cancels the active pairing code.
    pub fn cancel_pairing(&self) -> Result<()> {
        self.open()?;
        // The timer's slot is held over both steps, as in `begin_pairing`, so a code made in between keeps its timer.
        let mut slot = lock(&self.expiry);
        self.ctx.host.cancel();
        if let Some(old) = slot.take() {
            old.abort();
        }

        Ok(())
    }

    fn replace_timer(&self, timer: Option<JoinHandle<()>>) {
        if let Some(old) = std::mem::replace(&mut *lock(&self.expiry), timer) {
            old.abort();
        }
    }

    /// Whether a pairing code is waiting, and for how long and how many wrong tries it has left. A user who sees fewer
    /// tries than the three it started with knows someone has tried it.
    pub fn pairing_state(&self) -> Result<PairingState> {
        self.open()?;
        Ok(self.ctx.host.state())
    }

    /// Pairs with the device at `target` by the code its user sees. On success both devices have pinned each other and
    /// the host's [`Device`] is returned. A wrong, used, expired or cancelled code is [`Error::PairingRejected`].
    ///
    /// The addresses of the target are tried first, then what is kept about the device, what was remembered in memory
    /// and what mDNS found; this last also for a target made from a [`Found`], which carries no addresses of its own.
    /// The device is kept with the address the connection really used, the addresses that were typed and the ones
    /// given earlier to [`Link::remember_addresses`], but never with what mDNS said.
    pub async fn join(&self, code: &str, target: &Target) -> Result<Device> {
        let code = PairingCode::parse(code)?;
        self.open()?;

        let conn = self.connect(target, PAIR_ALPN).await?;
        let own_id = self.ctx.endpoint.id();

        let paired = match timeout(SESSION_TIMEOUT, join_session(&conn, &own_id, &code, &self.ctx.name)).await {
            Ok(result) => result,
            Err(_) => Err(Error::PairingTimedOut),
        };

        // Only addresses that are known to be the device's: the one the authenticated connection used and the ones the
        // user typed (what was remembered about the device is added when it is pinned), not what mDNS said.
        let addrs: Vec<SocketAddr> = used_addr(&conn)
            .into_iter()
            .chain(target.addrs.iter().copied())
            .collect();
        let device = match paired {
            Ok((node_id, name)) => self.ctx.pin(node_id, name, addrs).await,
            Err(error) => Err(error),
        };
        conn.close(if device.is_ok() { CLOSE_DONE } else { CLOSE_REFUSED }, b"pairing over");

        device
    }

    async fn connect(&self, target: &Target, alpn: &[u8]) -> Result<Connection> {
        let addr = self.ctx.dial(target.node_id, &target.addrs);

        match timeout(CONNECT_TIMEOUT, self.ctx.endpoint.connect(addr, alpn)).await {
            Ok(Ok(conn)) => Ok(conn),
            Ok(Err(error)) => Err(Error::Unreachable(error.to_string())),
            Err(_) => Err(Error::Unreachable("no answer in time".to_owned())),
        }
    }

    /// The paired devices.
    pub fn devices(&self) -> Result<Vec<Device>> {
        self.open()?;
        Ok(self.ctx.devices().list().to_vec())
    }

    /// Gives a paired device a new name.
    pub fn rename_device(&self, node_id: &EndpointId, name: &str) -> Result<()> {
        self.open()?;
        self.ctx.writer()?.rename(node_id, name)
    }

    /// Unpairs a device: it can no longer connect, and this device forgets it and where it was.
    pub fn unpair(&self, node_id: &EndpointId) -> Result<()> {
        self.open()?;
        self.ctx.unpair(node_id)?;

        let _ = self.ctx.events.send(LinkEvent::Unpaired(*node_id));
        Ok(())
    }

    /// Tells the service where a device may be reached, for devices that are not on the local network (the addresses of
    /// a VPN). A paired device's addresses are kept in the device list (the newest eight, which last across a restart);
    /// for another device they are remembered until the app ends or the table makes room for a newer one.
    pub fn remember_addresses(&self, node_id: EndpointId, addrs: Vec<SocketAddr>) -> Result<()> {
        self.open()?;
        self.ctx.remember(node_id, addrs);

        Ok(())
    }

    /// Whether a paired device can be reached: connects to it and says hello, giving up after three seconds, with at
    /// most eight probes at a time. A device that is not paired is [`Status::Offline`] without a connection being tried.
    /// A device that answers that it does not have this one paired is [`Status::Refused`]. The address a device answered
    /// from is kept for the next start.
    pub async fn status(&self, node_id: &EndpointId) -> Result<Status> {
        self.open()?;
        if !self.ctx.is_paired(node_id) {
            return Ok(Status::Offline);
        }

        let status = match hello::probe(&self.ctx, self.ctx.dial(*node_id, &[])).await {
            Probe::Answered { hello, addr } => {
                self.ctx.remember_in_background(*node_id, addr);
                Status::Online { name: hello.name }
            }
            Probe::Refused => Status::Refused,
            Probe::Unreachable => Status::Offline,
        };

        Ok(status)
    }

    /// The devices found on the local network, not including this one.
    pub fn discovered(&self) -> Result<Vec<Found>> {
        self.open()?;
        let found = lock(&self.ctx.found);
        let devices = self.ctx.devices();

        let mut list: Vec<Found> = found
            .iter()
            .map(|(node_id, entry)| Found {
                node_id: *node_id,
                name: entry.name.clone(),
                addrs: entry.addrs.iter().copied().collect(),
                paired: devices.is_paired(node_id),
            })
            .collect();
        list.sort_by_key(|found| found.node_id);

        Ok(list)
    }

    /// Events of the service, such as a device that paired with this one. See [`LinkEvent`] for a receiver that falls
    /// behind.
    pub fn subscribe(&self) -> Result<broadcast::Receiver<LinkEvent>> {
        self.open()?;
        Ok(self.ctx.events.subscribe())
    }

    /// Stops the service: no new connections are accepted, the open ones are closed and the lock on the data folder is
    /// let go. Every call after this returns [`Error::Closed`].
    pub async fn shutdown(&self) {
        if self.ctx.closed.swap(true, Ordering::AcqRel) {
            return;
        }

        {
            let mut slot = lock(&self.expiry);
            self.ctx.host.cancel();
            if let Some(old) = slot.take() {
                old.abort();
            }
        }
        for task in lock(&self.tasks).drain(..) {
            task.abort();
        }

        self.ctx.endpoint.close().await;

        // A change of the device list that is being written finishes first, and none starts after this (see
        // `Ctx::writer`), so the lock on the folder is let go only when nothing of this service writes in it.
        drop(self.ctx.devices());
        lock(&self.folder_lock).take();
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        for task in lock(&self.tasks).drain(..) {
            task.abort();
        }
        self.replace_timer(None);

        // As in `shutdown`: no change of the device list starts after this, and one being written finishes before the
        // lock on the folder (a field, let go after this) is, so a second service on the folder never races a write.
        let was_closed = self.ctx.closed.swap(true, Ordering::AcqRel);
        drop(self.ctx.devices());

        // Closing waits for the peers, which Drop cannot do; it is left to the runtime if there is one.
        if !was_closed && let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let endpoint = self.ctx.endpoint.clone();
            runtime.spawn(async move { endpoint.close().await });
        }
    }
}

/// Tells the app's event channel and the mDNS task what the pairing code is doing.
fn watch_pairing(ctx: &Ctx) {
    let events = ctx.events.clone();
    let announcing = ctx.announcing.clone();

    ctx.host.observe(move |change| match change {
        Change::Began => {
            let _ = announcing.send(true);
        }
        Change::Ended(reason) => {
            let _ = announcing.send(false);
            let _ = events.send(LinkEvent::PairingEnded { reason });
        }
    });
}

/// The address a connection really uses to reach the other device, if it is a direct one.
fn used_addr(conn: &Connection) -> Option<SocketAddr> {
    let paths = conn.paths();
    let mut addrs: Vec<(bool, SocketAddr)> = paths
        .iter()
        .filter_map(|path| match path.remote_addr() {
            TransportAddr::Ip(addr) => Some((path.is_selected(), *addr)),
            _ => None,
        })
        .collect();
    addrs.sort_by_key(|(selected, _)| !*selected);

    addrs.first().map(|(_, addr)| *addr)
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "link_hardening_tests.rs"]
mod hardening_tests;

#[cfg(test)]
#[path = "link_pairing_tests.rs"]
mod pairing_tests;
