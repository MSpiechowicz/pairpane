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

use std::time::Duration;

use iroh::EndpointId;
use iroh::endpoint::Connection;
use tokio::time::timeout;

use super::PAIR_VERSION;
use super::host::Attempt;
use super::proto::{Handshake, MESSAGE_LEN, Role, TAG_LEN};
use crate::code::PairingCode;
use crate::devices::{Device, validate_name};
use crate::error::{Error, Result};
use crate::wire::Channel;

/// How long a whole pairing may take on the host, from the connection on.
pub const SESSION_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the host waits for the joiner to take the last message before it closes the connection.
const LINGER: Duration = Duration::from_secs(3);

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

/// The host's side, for one attempt. `pin` stores the joiner (its node id and name) and is called only once the code is
/// proved and used up; the host's name goes out only after it succeeds. Returns what `pin` returned. A failure, or an
/// attempt that is dropped, counts against the code.
pub async fn host_session(
    conn: &Connection,
    own_id: &EndpointId,
    attempt: Attempt,
    own_name: &str,
    pin: impl FnOnce(EndpointId, &str) -> Result<Device>,
) -> Result<Device> {
    let joiner_id = conn.remote_id();

    let work = async {
        let mut channel = Channel::accept(conn).await?;

        let hello = channel.recv_exact(1 + MESSAGE_LEN).await?;
        if hello[0] != PAIR_VERSION {
            return Err(Error::Protocol("the pairing version is not supported"));
        }

        let handshake = Handshake::start(Role::Host, attempt.code());
        channel.send(handshake.message()).await?;

        let confirmation = handshake.finish(&hello[1..], &joiner_id, own_id)?;

        let joiner_tag = channel.recv_exact(TAG_LEN).await?;
        if !confirmation.verify(Role::Join, &joiner_tag) {
            return Err(Error::PairingRejected);
        }

        channel.send(&confirmation.tag(Role::Host)).await?;

        let joiner_name = read_name(&mut channel).await?;
        Ok((channel, joiner_name))
    };

    let (mut channel, joiner_name) = match timeout(SESSION_TIMEOUT, work).await {
        Ok(Ok(done)) => done,
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

    let device = pin(joiner_id, &joiner_name)?;

    channel.send(own_name.as_bytes()).await?;
    channel.finish();
    // Closing the connection now could drop the last message before the joiner has it.
    let _ = timeout(LINGER, conn.closed()).await;

    Ok(device)
}

/// Reads a frame that is a device name and checks it.
async fn read_name(channel: &mut Channel) -> Result<String> {
    let bytes = channel.recv().await?;
    let name = std::str::from_utf8(&bytes).map_err(|_| Error::Protocol("a device name is not text"))?;

    validate_name(name).map_err(|_| Error::Protocol("a device name is not allowed"))
}
