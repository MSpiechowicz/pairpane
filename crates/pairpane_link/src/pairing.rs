//! Pairing two devices with a one-time code, over iroh with the ALPN [`PAIR_ALPN`].
//!
//! One device shows a code ([`PairingHost`]), the other types it and connects ([`join_session`]). Both connections
//! are QUIC with TLS 1.3, so each side knows the other's node id before a byte of the protocol is sent; the protocol
//! (see `session`) then proves that the other side also knows the code and has seen the same two node ids, and only
//! then do both sides pin each other's node id in their device list.

mod host;
mod proto;
mod session;

pub use host::{CODE_TTL, Change, MAX_FAILURES, PairingEndReason, PairingHost, PairingOffer, PairingState};
pub use proto::{Handshake, Role, tags_match};

#[cfg(test)]
pub(crate) use proto::{Confirmation, MESSAGE_LEN, TAG_LEN};
pub(crate) use session::{SESSION_TIMEOUT, host_session, join_session};

/// The ALPN of the pairing protocol.
pub const PAIR_ALPN: &[u8] = b"pairpane/pair/1";

/// The version byte the joiner sends first.
pub(crate) const PAIR_VERSION: u8 = 1;
