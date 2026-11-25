//! The hello of a link, over the ALPN [`LINK_ALPN`]: the dialling device sends the protocol version and its name, the
//! other answers with the same, and both close. It is all a device needs to know that a paired device is online (a
//! connection that gets this far was accepted for its pinned node id), and it is the base later features build on.

use std::time::Duration;

use iroh::EndpointAddr;
use iroh::endpoint::Connection;
use tokio::time::timeout;

use super::ctx::Ctx;
use crate::devices::validate_name;
use crate::error::{Error, Result};
use crate::wire::Channel;

/// The ALPN of a link between paired devices.
pub const LINK_ALPN: &[u8] = b"pairpane/link/1";

/// The version a hello starts with.
pub const HELLO_VERSION: u8 = 1;

/// How long a probe may take, from dialling to the answer.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// How long the answering side waits for the other to take the answer before it closes the connection.
const LINGER: Duration = Duration::from_secs(2);

/// What a device that was reached says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    /// The protocol version.
    pub version: u8,
    /// The device's name.
    pub name: String,
}

fn encode(name: &str) -> Vec<u8> {
    let mut bytes = vec![HELLO_VERSION];
    bytes.extend_from_slice(name.as_bytes());
    bytes
}

fn decode(bytes: &[u8]) -> Result<Hello> {
    let (version, name) = bytes.split_first().ok_or(Error::Protocol("a hello is empty"))?;
    let name = std::str::from_utf8(name).map_err(|_| Error::Protocol("a device name is not text"))?;

    Ok(Hello {
        version: *version,
        name: validate_name(name).map_err(|_| Error::Protocol("a device name is not allowed"))?,
    })
}

/// Dials `addr` and says hello. `Some` is what the device answered; `None` means it could not be reached in time, or
/// refused (it has not paired this device), or did not speak the protocol.
pub async fn probe(ctx: &Ctx, addr: EndpointAddr) -> Option<Hello> {
    let exchange = async {
        let conn = ctx
            .endpoint
            .connect(addr, LINK_ALPN)
            .await
            .map_err(|_| Error::PairingRejected)?;

        let answer = async {
            let mut channel = Channel::open(&conn).await?;
            channel.send(&encode(&ctx.name)).await?;
            decode(&channel.recv().await?)
        }
        .await;

        conn.close(super::CLOSE_DONE, b"bye");
        answer
    };

    timeout(PROBE_TIMEOUT, exchange).await.ok()?.ok()
}

/// Answers the hello of a paired device on `conn`.
pub async fn answer(ctx: &Ctx, conn: &Connection) -> Result<Hello> {
    let mut channel = Channel::accept(conn).await?;
    let hello = decode(&channel.recv().await?)?;

    channel.send(&encode(&ctx.name)).await?;
    channel.finish();
    // The dialling side closes once it has the answer; closing first could drop the answer.
    let _ = timeout(LINGER, conn.closed()).await;

    Ok(hello)
}
