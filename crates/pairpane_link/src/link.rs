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
mod target;

use std::fs::File;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use iroh::endpoint::{Connection, VarInt};
use iroh::{EndpointId, TransportAddr};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::timeout;

pub use config::{LinkConfig, Status, ThisDevice};
pub use ctx::LinkEvent;
pub use discovery::{Discovery, Found};
pub use hello::{HELLO_VERSION, LINK_ALPN};
pub use target::Target;

use ctx::{Ctx, lock, now_ms};
use hello::Probe;

use crate::code::PairingCode;
use crate::devices::{Device, DeviceStore, validate_name};
use crate::error::{Error, Result};
use crate::fsutil;
use crate::identity::{KeyStores, StoreKind, open_identity};
use crate::pairing::{Change, PAIR_ALPN, PairingHost, PairingOffer, PairingState, SESSION_TIMEOUT, join_session};

/// The UDP port tried first, so a device keeps its address over a VPN.
pub const DEFAULT_PORT: u16 = 47620;

/// The most connections that are set up or served at once; more are refused.
pub const MAX_CONNECTIONS: usize = 32;

/// The name of the lock file in the data folder.
const LOCK_FILE_NAME: &str = "link.lock";

/// How long a connection to a device to pair with may take to be set up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The application error code of a connection that ended well.
const CLOSE_DONE: VarInt = VarInt::from_u32(0);
/// The application error code of a connection that was turned down: the protocol is not allowed now, or the device is
/// not paired.
const CLOSE_NOT_ALLOWED: VarInt = VarInt::from_u32(1);
/// The application error code of a pairing that failed.
const CLOSE_REFUSED: VarInt = VarInt::from_u32(2);

/// The running service. Keep it for as long as the app runs and call [`Link::shutdown`] when it ends; dropping it stops
/// its tasks and closes the endpoint, but without waiting for the peers.
pub struct Link {
    ctx: Arc<Ctx>,
    key_store: StoreKind,
    closed: AtomicBool,
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
        let folder_lock = fsutil::lock_dir(&config.data_dir, LOCK_FILE_NAME)?;

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
            closed: AtomicBool::new(false),
            folder_lock: Mutex::new(Some(folder_lock)),
            tasks: Mutex::new(tasks),
            expiry: Mutex::new(None),
        })
    }

    fn open(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
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
    pub fn begin_pairing(&self) -> Result<PairingOffer> {
        self.open()?;
        let offer = self.ctx.host.begin()?;

        let host = Arc::clone(&self.ctx.host);
        let due = offer.expires_in;
        let timer = tokio::spawn(async move {
            tokio::time::sleep(due).await;
            host.check_expiry();
        });
        self.replace_timer(Some(timer));

        Ok(offer)
    }

    /// Cancels the active pairing code.
    pub fn cancel_pairing(&self) -> Result<()> {
        self.open()?;
        self.ctx.host.cancel();
        self.replace_timer(None);

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
    /// The addresses of the target are tried first, then what is kept about the device and what mDNS found. The device
    /// is kept with the address the connection really used and the addresses that were typed.
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
        // user typed, not what mDNS said.
        let addrs: Vec<SocketAddr> = used_addr(&conn)
            .into_iter()
            .chain(target.addrs.iter().copied())
            .collect();
        let device =
            paired.and_then(|(node_id, name)| self.ctx.devices().add_with_addrs(node_id, &name, now_ms(), &addrs));
        conn.close(if device.is_ok() { CLOSE_DONE } else { CLOSE_REFUSED }, b"pairing over");

        if let Ok(device) = &device {
            // The device list has its addresses now.
            self.ctx.forget(&device.node_id);
        }

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
        self.ctx.devices().rename(node_id, name)
    }

    /// Unpairs a device: it can no longer connect, and this device forgets it and where it was.
    pub fn unpair(&self, node_id: &EndpointId) -> Result<()> {
        self.open()?;
        self.ctx.devices().remove(node_id)?;
        self.ctx.forget(node_id);

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
                self.ctx.remember(*node_id, addr);
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
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }

        self.ctx.host.cancel();
        self.replace_timer(None);
        for task in lock(&self.tasks).drain(..) {
            task.abort();
        }

        self.ctx.endpoint.close().await;
        lock(&self.folder_lock).take();
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        for task in lock(&self.tasks).drain(..) {
            task.abort();
        }
        self.replace_timer(None);

        // Closing waits for the peers, which Drop cannot do; it is left to the runtime if there is one.
        if !self.closed.load(Ordering::Acquire)
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
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
