//! The two sides of the pairing conversation on one QUIC stream.
//!
//! 1. The joiner sends the protocol version and its SPAKE2 message; the host answers with its own.
//! 2. The joiner sends its confirmation tag. The host checks it, and only a host that is satisfied sends its tag, so a
//!    guess of the code never learns anything from the host. The joiner checks the host's tag.
//! 3. The joiner sends its device name; the host pins the joiner's node id, uses the code up, and answers with its name;
//!    the joiner pins the host's node id.
//!
//! The node ids pinned are the ones of the QUIC connections, which iroh has authenticated with each device's key. Any
//! failure before the end leaves nothing pinned.

use std::sync::Arc;
use std::time::Duration;

use iroh::EndpointId;
use iroh::endpoint::Connection;
use tokio::time::timeout;

use super::PAIR_VERSION;
use super::host::PairingHost;
use super::proto::{Handshake, MESSAGE_LEN, Role, TAG_LEN};
use crate::code::PairingCode;
use crate::devices::{Device, validate_name};
use crate::error::{Error, Result};
use crate::wire::{CLOSE_DONE, Channel};

/// How long a whole pairing may take on the host, from the connection on.
pub const SESSION_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a joiner has to send its first message. A connection that sends nothing in that time is closed; until then it
/// waits without taking the attempt, so it does not keep another joiner out or count against the code. The tests make it
/// longer, so connections that say nothing are surely still waiting when they check that a real joiner gets through.
#[cfg(not(test))]
pub const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(test)]
pub const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the host waits for the joiner to take the last message before it closes the connection. A test makes it long
/// to show that nothing else waits for it.
#[cfg(not(test))]
const LINGER: Duration = Duration::from_secs(3);
#[cfg(test)]
const LINGER: Duration = Duration::from_secs(60);

/// The joining side. Returns the host's node id and name, which the caller pins.
pub async fn join_session(
    conn: &Connection,
    own_id: &EndpointId,
    code: &PairingCode,
    own_name: &str,
) -> Result<(EndpointId, String)> {
    let host_id = conn.remote_id();
    let handshake = Handshake::start(Role::Join, code);
    let mut channel = Channel::open(conn).await?;

    let mut hello = vec![PAIR_VERSION];
    hello.extend_from_slice(handshake.message());
    channel.send(&hello).await?;

    let peer_message = channel.recv_exact(MESSAGE_LEN).await?;
    let confirmation = handshake.finish(&peer_message, own_id, &host_id)?;

    channel.send(&confirmation.tag(Role::Join)).await?;

    let host_tag = channel.recv_exact(TAG_LEN).await?;
    if !confirmation.verify(Role::Host, &host_tag) {
        return Err(Error::PairingRejected);
    }

    channel.send(own_name.as_bytes()).await?;
    let host_name = read_name(&mut channel).await?;

    Ok((host_id, host_name))
}

/// The host's side, for one connection. `pin` stores the joiner (its node id and name) and is called only once the code
/// is proved and used up. Returns what `pin` returned.
///
/// The connection waits for its first message without an attempt: a connection that sends nothing within
/// [`FIRST_FRAME_TIMEOUT`], or that ends before a first message, is [`Error::PairingTimedOut`] and costs the code
/// nothing, and any number of them (up to the caps of the accept loop) can wait at once. The connection whose first
/// message arrives takes the attempt of `host`; one that finds another running, or the code gone, is
/// [`Error::PairingRejected`] and is not counted. From the attempt on, any failure, or the attempt being dropped,
/// counts against the code.
///
/// Once the joiner is pinned the pairing is done and the device is returned at once. The host's name is sent after
/// that, by a task of its own that also closes `conn` (the joiner pins the host when it has the name, and may be gone):
/// a failure to send is only logged, and the caller must not close `conn` after an `Ok`.
pub async fn host_session(
    conn: &Connection,
    own_id: &EndpointId,
    host: &Arc<PairingHost>,
    own_name: &str,
    pin: impl AsyncFnOnce(EndpointId, String) -> Result<Device>,
) -> Result<Device> {
    let joiner_id = conn.remote_id();

    let first = async {
        let mut channel = Channel::accept(conn).await?;
        let hello = timeout(FIRST_FRAME_TIMEOUT, channel.recv())
            .await
            .map_err(|_| Error::PairingTimedOut)??;

        Ok((channel, hello))
    };
    let (mut channel, hello) = match timeout(FIRST_FRAME_TIMEOUT, first).await {
        Ok(Ok(started)) => started,
        // The stream or the connection ended, or nothing came: the joiner has not tried the code.
        Ok(Err(Error::PairingRejected | Error::PairingTimedOut)) | Err(_) => return Err(Error::PairingTimedOut),
        Ok(Err(error)) => {
            // Something came that is not a message: that is a try, if the code is free for one.
            if let Some(attempt) = host.start_attempt() {
                attempt.fail();
            }
            return Err(error);
        }
    };

    let Some(attempt) = host.start_attempt() else {
        return Err(Error::PairingRejected);
    };

    let work = async {
        if hello.len() != 1 + MESSAGE_LEN || hello[0] != PAIR_VERSION {
            return Err(Error::Protocol("the pairing message is not supported"));
        }

        let handshake = Handshake::start(Role::Host, attempt.code());
        channel.send(handshake.message()).await?;

        let confirmation = handshake.finish(&hello[1..], &joiner_id, own_id)?;

        let joiner_tag = channel.recv_exact(TAG_LEN).await?;
        if !confirmation.verify(Role::Join, &joiner_tag) {
            return Err(Error::PairingRejected);
        }

        channel.send(&confirmation.tag(Role::Host)).await?;

        read_name(&mut channel).await
    };

    let joiner_name = match timeout(SESSION_TIMEOUT, work).await {
        Ok(Ok(name)) => name,
        Ok(Err(error)) => {
            attempt.fail();
            return Err(error);
        }
        Err(_) => {
            attempt.fail();
            return Err(Error::PairingTimedOut);
        }
    };

    if !attempt.succeed() {
        return Err(Error::PairingRejected);
    }

    let device = pin(joiner_id, joiner_name).await?;

    tokio::spawn(send_name_and_close(conn.clone(), channel, own_name.to_owned()));

    Ok(device)
}

/// The end of a pairing that is done: the host's name goes to the joiner, and the connection is closed once the joiner
/// has taken it (or [`LINGER`] has passed).
async fn send_name_and_close(conn: Connection, mut channel: Channel, name: String) {
    match channel.send(name.as_bytes()).await {
        Ok(()) => {
            channel.finish();
            // Closing the connection now could drop the last message before the joiner has it.
            let _ = timeout(LINGER, conn.closed()).await;
        }
        Err(error) => tracing::debug!(%error, "the pairing was saved but the joiner did not take the host's name"),
    }

    conn.close(CLOSE_DONE, b"paired");
}

/// Reads a frame that is a device name and checks it.
async fn read_name(channel: &mut Channel) -> Result<String> {
    let bytes = channel.recv().await?;
    let name = std::str::from_utf8(&bytes).map_err(|_| Error::Protocol("a device name is not text"))?;

    validate_name(name).map_err(|_| Error::Protocol("a device name is not allowed"))
}
