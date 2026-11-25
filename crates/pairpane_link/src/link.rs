//! The running link service: one iroh endpoint that accepts pairings and hellos, finds devices on the local network, and
//! the calls the app makes on it.
//!
//! The endpoint is made without relays and without DNS or pkarr address lookup, so it never contacts a server: devices
//! reach each other by the addresses on their own networks (the local network, or a VPN such as Tailscale), or the ones
//! mDNS finds. It listens on one UDP port; [`DEFAULT_PORT`] is tried first so the address of a device over a VPN stays
//! the same from one start to the next.

mod accept;
mod ctx;
mod discovery;
mod hello;
mod target;

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{BindOpts, Connection, VarInt, presets};
use iroh::{Endpoint, EndpointId, SecretKey};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::timeout;

pub use ctx::LinkEvent;
pub use discovery::{Discovery, Found};
pub use hello::{HELLO_VERSION, LINK_ALPN};
pub use target::Target;

use ctx::{Ctx, lock, now_ms};

use crate::code::PairingCode;
use crate::devices::{Device, DeviceStore, validate_name};
use crate::error::{Error, Result};
use crate::identity::{KeyStore, StoreKind, load_or_create};
use crate::pairing::{CODE_TTL, PAIR_ALPN, PairingHost, PairingOffer, PairingState, SESSION_TIMEOUT, join_session};

/// The UDP port tried first, so a device keeps its address over a VPN.
pub const DEFAULT_PORT: u16 = 47620;

/// The most connections that are set up or served at once; more are refused.
pub const MAX_CONNECTIONS: usize = 32;

/// How long a connection to a device to pair with may take to be set up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The application error code of a connection that ended well.
const CLOSE_DONE: VarInt = VarInt::from_u32(0);
/// The application error code of a connection that was turned down: the protocol is not allowed now, or the device is
/// not paired.
const CLOSE_NOT_ALLOWED: VarInt = VarInt::from_u32(1);
/// The application error code of a pairing that failed.
const CLOSE_REFUSED: VarInt = VarInt::from_u32(2);

/// How the service is set up.
#[derive(Debug, Clone)]
pub struct LinkConfig {
    /// The folder of the device list (and of the key file, when the key store is a file).
    pub data_dir: PathBuf,
    /// This device's name, shown on the devices it pairs with. 1 to 64 characters.
    pub device_name: String,
    /// The UDP ports to try to listen on, in order; `0` is a port the system picks. The default is
    /// [`DEFAULT_PORT`], then `0`.
    pub ports: Vec<u16>,
    /// Listens on the loopback address only, which nothing else on the network can reach. For tests.
    pub loopback_only: bool,
    /// What the device does on the local network; the default is to find and to announce.
    pub discovery: Discovery,
    /// How long a pairing code can be used; the default is two minutes ([`CODE_TTL`]). Tests shorten it.
    pub pairing_ttl: Duration,
}

impl LinkConfig {
    /// The default setup for a device with this name and folder.
    pub fn new(data_dir: impl Into<PathBuf>, device_name: impl Into<String>) -> Self {
        Self {
            data_dir: data_dir.into(),
            device_name: device_name.into(),
            ports: vec![DEFAULT_PORT, 0],
            loopback_only: false,
            discovery: Discovery::FindAndAnnounce,
            pairing_ttl: CODE_TTL,
        }
    }
}

/// What the device knows about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThisDevice {
    /// The node id (the public key) other devices pin.
    pub node_id: EndpointId,
    /// The name.
    pub name: String,
    /// The addresses the UDP sockets are bound to, such as `0.0.0.0:47620`.
    pub bound: Vec<SocketAddr>,
    /// The addresses the device can be reached at, one per network interface, which is what to type on another
    /// device over a VPN.
    pub addrs: Vec<SocketAddr>,
    /// Where the device key is kept.
    pub key_store: StoreKind,
}

impl ThisDevice {
    /// How another device reaches this one: its node id and addresses.
    pub fn target(&self) -> Target {
        Target {
            node_id: self.node_id,
            addrs: self.addrs.clone(),
        }
    }
}

/// Whether a paired device can be reached right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The device answered; this is the name it answered with.
    Online {
        /// The name the device reports for itself.
        name: String,
    },
    /// The device did not answer, or does not have this one paired.
    Offline,
}

/// The running service. Keep it for as long as the app runs and call [`Link::shutdown`] when it ends; dropping it stops
/// its tasks but closes connections less gracefully.
pub struct Link {
    ctx: Arc<Ctx>,
    key_store: StoreKind,
    tasks: std::sync::Mutex<Vec<JoinHandle<()>>>,
}

impl Link {
    /// Starts the service: loads or makes the device key in `store`, opens the device list in the data folder, binds the
    /// endpoint and starts accepting connections and finding devices.
    ///
    /// The key store may talk to the OS keychain, which blocks, so it is called on a thread meant for blocking work.
    pub async fn start(config: LinkConfig, store: Box<dyn KeyStore>) -> Result<Self> {
        let name = validate_name(&config.device_name)?;
        let key_store = store.kind();

        let key = tokio::task::spawn_blocking(move || load_or_create(store.as_ref()))
            .await
            .map_err(|_| Error::Closed)??;
        let devices = DeviceStore::open(&config.data_dir)?;

        let endpoint = bind(&config, key, &name).await?;
        let host = PairingHost::new(config.pairing_ttl);
        let ctx = Arc::new(Ctx::new(endpoint, name, devices, host));

        let mut tasks = vec![tokio::spawn(accept::run(Arc::clone(&ctx)))];
        tasks.extend(discovery::start(&ctx, config.discovery).await);

        Ok(Self {
            ctx,
            key_store,
            tasks: std::sync::Mutex::new(tasks),
        })
    }

    /// This device's id, name and addresses.
    pub fn this_device(&self) -> ThisDevice {
        let endpoint = &self.ctx.endpoint;

        ThisDevice {
            node_id: endpoint.id(),
            name: self.ctx.name.clone(),
            bound: endpoint.bound_sockets(),
            addrs: endpoint.addr().ip_addrs().copied().collect(),
            key_store: self.key_store,
        }
    }

    /// Makes a pairing code and starts accepting pairings with it. A code that is active is replaced.
    pub fn begin_pairing(&self) -> Result<PairingOffer> {
        self.ctx.host.begin()
    }

    /// Cancels the active pairing code.
    pub fn cancel_pairing(&self) {
        self.ctx.host.cancel();
    }

    /// Whether a pairing code is waiting, and for how long.
    pub fn pairing_state(&self) -> PairingState {
        self.ctx.host.state()
    }

    /// Pairs with the device at `target` by the code its user sees. On success both devices have pinned each other and
    /// the host's [`Device`] is returned. A wrong, used, expired or cancelled code is [`Error::PairingRejected`].
    pub async fn join(&self, code: &str, target: &Target) -> Result<Device> {
        let code = PairingCode::parse(code)?;
        self.ctx.remember(target.node_id, target.addrs.iter().copied());

        let conn = self.connect(target, PAIR_ALPN).await?;
        let own_id = self.ctx.endpoint.id();

        let paired = match timeout(SESSION_TIMEOUT, join_session(&conn, &own_id, &code, &self.ctx.name)).await {
            Ok(result) => result,
            Err(_) => Err(Error::PairingTimedOut),
        };

        let device = paired.and_then(|(node_id, name)| {
            self.ctx
                .devices()
                .add_with_addrs(node_id, &name, now_ms(), &target.addrs)
        });
        conn.close(if device.is_ok() { CLOSE_DONE } else { CLOSE_REFUSED }, b"pairing over");

        device
    }

    async fn connect(&self, target: &Target, alpn: &[u8]) -> Result<Connection> {
        let addr = self.ctx.dial(target.node_id);

        match timeout(CONNECT_TIMEOUT, self.ctx.endpoint.connect(addr, alpn)).await {
            Ok(Ok(conn)) => Ok(conn),
            Ok(Err(error)) => Err(Error::Unreachable(error.to_string())),
            Err(_) => Err(Error::Unreachable("no answer in time".to_owned())),
        }
    }

    /// The paired devices.
    pub fn devices(&self) -> Vec<Device> {
        self.ctx.devices().list().to_vec()
    }

    /// Gives a paired device a new name.
    pub fn rename_device(&self, node_id: &EndpointId, name: &str) -> Result<()> {
        self.ctx.devices().rename(node_id, name)
    }

    /// Unpairs a device: it can no longer connect, and this device forgets it.
    pub fn unpair(&self, node_id: &EndpointId) -> Result<()> {
        self.ctx.devices().remove(node_id)
    }

    /// Tells the service where a device may be reached, for devices that are not on the local network (the addresses of
    /// a VPN). A paired device's addresses are kept in the device list (the newest eight, which last across a restart);
    /// for another device they are remembered until the app ends.
    pub fn remember_addresses(&self, node_id: EndpointId, addrs: Vec<SocketAddr>) {
        self.ctx.remember(node_id, addrs);
    }

    /// Whether a paired device can be reached: connects to it and says hello, giving up after three seconds. A device
    /// that is not paired is [`Status::Offline`] without a connection being tried.
    pub async fn status(&self, node_id: &EndpointId) -> Status {
        if !self.ctx.is_paired(node_id) {
            return Status::Offline;
        }

        match hello::probe(&self.ctx, self.ctx.dial(*node_id)).await {
            Some(hello) => Status::Online { name: hello.name },
            None => Status::Offline,
        }
    }

    /// The devices found on the local network, not including this one.
    pub fn discovered(&self) -> Vec<Found> {
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

        list
    }

    /// Events of the service, such as a device that paired with this one.
    pub fn subscribe(&self) -> broadcast::Receiver<LinkEvent> {
        self.ctx.events.subscribe()
    }

    /// Stops the service: no new connections are accepted and the open ones are closed.
    pub async fn shutdown(&self) {
        self.ctx.host.cancel();

        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
        for task in &tasks {
            task.abort();
        }

        self.ctx.endpoint.close().await;
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        for task in self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
        {
            task.abort();
        }
    }
}

/// Makes the endpoint on the first port of the configuration that can be bound. Relays are off (the endpoint is built
/// from the empty relay setting) and no address lookup is added here, so it never contacts a server.
async fn bind(config: &LinkConfig, key: SecretKey, name: &str) -> Result<Endpoint> {
    let mut last_error = Error::Bind("no port to try".to_owned());

    for port in &config.ports {
        match bind_on(config, key.clone(), name, *port).await {
            Ok(endpoint) => return Ok(endpoint),
            Err(error) => last_error = error,
        }
    }

    Err(last_error)
}

async fn bind_on(config: &LinkConfig, key: SecretKey, name: &str, port: u16) -> Result<Endpoint> {
    let failed = |error: &dyn std::fmt::Display| Error::Bind(error.to_string());

    let mut builder = Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .alpns(vec![PAIR_ALPN.to_vec(), LINK_ALPN.to_vec()])
        .clear_ip_transports();

    if config.loopback_only {
        builder = builder
            .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
            .map_err(|e| failed(&e))?;
    } else {
        builder = builder
            .bind_addr(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))
            .map_err(|e| failed(&e))?
            // A machine without IPv6 (or one whose IPv4 socket already covers it) still works.
            .bind_addr_with_opts(
                SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)),
                BindOpts::default().set_is_required(false),
            )
            .map_err(|e| failed(&e))?;
    }

    if config.discovery == Discovery::FindAndAnnounce
        && let Some(data) = discovery::announced_name(name)
    {
        builder = builder.user_data_for_address_lookup(data);
    }

    builder.bind().await.map_err(|e| failed(&e))
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;
