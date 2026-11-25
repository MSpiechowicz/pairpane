//! Link: encrypted pairing of the user's own devices and a direct connection between them, without a cloud service.
//!
//! The crate has no dependency on Tauri; the app calls [`Link`]. Devices are QUIC peers (iroh) that know each other by
//! an Ed25519 key: [`identity`] keeps this device's key (OS keychain, or a private file), [`devices`] the keys of the
//! devices that were paired, and [`pairing`] pairs two devices with a one-time code shown on one and typed on the other.
//! [`Link`] runs it all: it accepts a pairing only while a code is active, accepts the link of a paired device only, and
//! answers the hello with which the app shows a device as online. Nothing leaves the user's own networks: the
//! endpoint has no relay and no address lookup service except mDNS on the local network.
//!
//! The usual use:
//!
//! 1. [`Link::start`] with a [`LinkConfig`] and the [`KeyStores`] ([`default_stores`]).
//! 2. On the device that is paired from, [`Link::begin_pairing`] and show [`PairingOffer::code`] and
//!    [`ThisDevice::target`]; on the other, [`Link::join`] with the typed code and a [`Target`] (a device found by
//!    [`Link::discovered`], or the text another device showed).
//! 3. [`Link::devices`], [`Link::status`], [`Link::rename_device`] and [`Link::unpair`] for the list, and
//!    [`Link::subscribe`] for what happens (a pairing, an unpairing, the end of a code).
//! 4. [`Link::shutdown`].

pub mod code;
pub mod devices;
mod error;
mod fsutil;
pub mod identity;
mod link;
pub mod pairing;
mod wire;

pub use code::PairingCode;
pub use devices::{Device, DeviceStore, MAX_NAME_CHARS, reset_devices};
pub use error::{Error, Result};
#[cfg(unix)]
pub use identity::FileStore;
pub use identity::{
    KeyStore, KeyStores, KeychainStore, MemoryStore, StoreKind, default_stores, load_or_create, open_identity,
};
pub use link::{
    DEFAULT_PORT, Discovery, Found, HELLO_VERSION, LINK_ALPN, Link, LinkConfig, LinkEvent, Status, Target, ThisDevice,
};
pub use pairing::{PAIR_ALPN, PairingEndReason, PairingOffer, PairingState};

/// The node id type of a device (its Ed25519 public key).
pub use iroh::EndpointId as NodeId;
