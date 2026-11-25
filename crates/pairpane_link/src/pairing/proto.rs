//! The cryptography of a pairing, without the network: SPAKE2 over the code, then a key confirmation in both
//! directions.
//!
//! SPAKE2 turns the short code into a strong shared key, and an attacker who does not know the code gets one guess per
//! session, no more. Neither side trusts that key until the other has proved it holds it: each sends a BLAKE3 keyed
//! hash of a transcript that has both SPAKE2 messages, both node ids as the authenticated QUIC connection reported them,
//! a label of the sender's role and the protocol version. Because the node ids are in it, a man in the middle who relays
//! the exchange between two connections cannot make the tags match: the two sides saw different ids.

use iroh::EndpointId;
use spake2::{Ed25519Group, Identity, Password, Spake2};

use crate::code::PairingCode;
use crate::error::{Error, Result};

/// The length of a SPAKE2 message of the Ed25519 group: a side byte and a 32-byte point.
pub const MESSAGE_LEN: usize = 33;
/// The length of a confirmation tag.
pub const TAG_LEN: usize = 32;

/// The identity the joining device uses in the key exchange, and so the side `A`.
const JOIN_IDENTITY: &[u8] = b"pairpane-join";
/// The identity of the device that shows the code, the side `B`.
const HOST_IDENTITY: &[u8] = b"pairpane-host";

/// Names the key derived from the SPAKE2 output (BLAKE3 `derive_key` context).
const KEY_CONTEXT: &str = "pairpane link pairing v1 confirmation key";
/// Starts the transcript, so a tag made for another protocol or version never matches.
const TRANSCRIPT_LABEL: &[u8] = b"pairpane/pair/1 transcript";

/// Which side of the pairing a device is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The device that types the code.
    Join,
    /// The device that shows the code.
    Host,
}

impl Role {
    /// The role's label in the transcript of the tag it sends.
    fn label(self) -> &'static [u8] {
        match self {
            Self::Join => b"join",
            Self::Host => b"host",
        }
    }
}

/// A key exchange that is under way: this side's message is made, the other's is not read yet.
pub struct Handshake {
    role: Role,
    state: Spake2<Ed25519Group>,
    message: Vec<u8>,
}

impl Handshake {
    /// Starts the exchange of `role` with `code` as the password.
    pub fn start(role: Role, code: &PairingCode) -> Self {
        let password = Password::new(code.as_bytes());
        let join = Identity::new(JOIN_IDENTITY);
        let host = Identity::new(HOST_IDENTITY);

        let (state, message) = match role {
            Role::Join => Spake2::<Ed25519Group>::start_a(&password, &join, &host),
            Role::Host => Spake2::<Ed25519Group>::start_b(&password, &join, &host),
        };

        Self { role, state, message }
    }

    /// This side's message, to send to the other.
    pub fn message(&self) -> &[u8] {
        &self.message
    }

    /// Finishes with the other side's message. `join_id` and `host_id` are the node ids of the two devices as the
    /// authenticated connection gave them: a side's own id comes from its endpoint and the other's from the
    /// connection.
    pub fn finish(self, peer_message: &[u8], join_id: &EndpointId, host_id: &EndpointId) -> Result<Confirmation> {
        let (join_message, host_message) = match self.role {
            Role::Join => (self.message.as_slice(), peer_message),
            Role::Host => (peer_message, self.message.as_slice()),
        };

        let mut transcript = Vec::new();
        for part in [
            TRANSCRIPT_LABEL,
            join_message,
            host_message,
            join_id.as_bytes(),
            host_id.as_bytes(),
        ] {
            // Every part is length-prefixed, so no two different transcripts have the same bytes.
            transcript.extend_from_slice(&(part.len() as u32).to_be_bytes());
            transcript.extend_from_slice(part);
        }

        let shared = self
            .state
            .finish(peer_message)
            .map_err(|_| Error::Protocol("the key exchange message is bad"))?;

        Ok(Confirmation {
            key: blake3::derive_key(KEY_CONTEXT, &shared),
            transcript,
        })
    }
}

/// The shared key and the transcript of a finished exchange, which can make the tag of a role and check the other's.
pub struct Confirmation {
    key: [u8; 32],
    transcript: Vec<u8>,
}

impl Confirmation {
    /// The tag that proves the holder of the key is `role`.
    pub fn tag(&self, role: Role) -> [u8; TAG_LEN] {
        let mut hasher = blake3::Hasher::new_keyed(&self.key);
        hasher.update(&self.transcript);
        hasher.update(role.label());

        *hasher.finalize().as_bytes()
    }

    /// Whether `tag` is the tag of `role`. The comparison takes the same time wherever the two differ.
    pub fn verify(&self, role: Role, tag: &[u8]) -> bool {
        tags_match(&self.tag(role), tag)
    }
}

/// Compares two tags in constant time (`blake3::Hash` compares without stopping at the first difference). A tag of
/// another length never matches.
pub fn tags_match(expected: &[u8; TAG_LEN], received: &[u8]) -> bool {
    match <[u8; TAG_LEN]>::try_from(received) {
        Ok(received) => blake3::Hash::from_bytes(*expected) == blake3::Hash::from_bytes(received),
        Err(_) => false,
    }
}

#[cfg(test)]
#[path = "proto_tests.rs"]
mod tests;
