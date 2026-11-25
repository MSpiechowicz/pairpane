//! The setup of the service and the plain values it reports.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use iroh::EndpointId;

use super::{DEFAULT_PORT, Discovery, Target};
use crate::identity::StoreKind;
use crate::pairing::CODE_TTL;

/// How the service is set up.
#[derive(Debug, Clone)]
pub struct LinkConfig {
    /// The folder of the device list, the identity marker and the lock (and of the key file, when the key store is a
    /// file).
    pub data_dir: PathBuf,
    /// This device's name, shown on the devices it pairs with. 1 to 64 characters.
    pub device_name: String,
    /// The UDP ports to try to listen on, in order; `0` is a port the system picks. The default is
    /// [`DEFAULT_PORT`], then `0`; a port that cannot be bound is logged and the next one is tried.
    pub ports: Vec<u16>,
    /// Listens on the loopback address only, which nothing else on the network can reach. For tests.
    pub loopback_only: bool,
    /// What the device does on the local network; the default is to find devices and to announce only while a pairing
    /// code is active.
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
            discovery: Discovery::Find,
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
    /// The device did not answer in time, or could not be reached.
    Offline,
    /// The device answered that it does not have this one paired (it was unpaired there). Unpair it here too, or pair
    /// again.
    Refused,
}
